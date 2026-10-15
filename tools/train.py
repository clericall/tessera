#!/usr/bin/env python3
"""Train a Tessera NNUE net and export it in the engine's `.tnn` format.

The network is exactly what engine/bb.ter evaluates:

    acc[i]    = clip_i16(bias[i] + sum_{f active} w1[f][i])        768 -> hidden
    hidden    = clip_i16(sum_i acc[i] * w2[i])
    value     = clip_i16(sum_{f active} psqt[f] + hidden)
    score     = value * scale / 64                                   (cp)

Weights are trained in float32 and quantized to i16 on export, mirroring what the
engine reads. `scale` defaults to 64 so the engine's integer divide is a no-op.

Data is the 40-byte record produced by tools/gen_data.py:
    bytes 0..31 board nibbles (a1 first, 1-6 = P N B R Q K white, 7-12 black)
    byte 32 side to move (0 white)   byte 33 castling   byte 34 ep   bytes 36..39 result

Feature convention (must match engine/bb.ter `feat()`): for the view of the side to
move, an own piece is block `pt-1` at square `sq`, an enemy piece is block
`6+(pt-1)` at square `63-sq`.

Usage:
    python tools/train.py --data selfplay.bin --out engine/nnue-smoke.tnn \
        --hidden 32 --epochs 20 --lr 0.002 --batch 256
"""

import argparse
import math
import struct
import sys
import time

import numpy as np

MAGIC = b"TNNU"
VERSION = 1
FEATURES = 768
I16_MAX = 32767
I16_MIN = -32768


def load_records(path: str):
    """Return (board[N,64] uint8, stm[N] uint8, game[N] uint8, result[N] float32)."""
    raw = np.fromfile(path, dtype=np.uint8)
    if len(raw) % 40 != 0:
        raise ValueError(f"{path}: size {len(raw)} is not a multiple of 40")
    raw = raw.reshape(-1, 40)
    packed = raw[:, :32]
    even = packed & 0x0F
    odd = packed >> 4
    board = np.empty((raw.shape[0], 64), dtype=np.uint8)
    board[:, 0::2] = even
    board[:, 1::2] = odd
    stm = raw[:, 32].astype(np.uint8)
    game = raw[:, 35].astype(np.uint8)
    result = raw[:, 36:40].copy().view(np.float32).ravel()
    return board, stm, game, result


def feature_indices(board: np.ndarray, stm: np.ndarray) -> np.ndarray:
    """(N, 64) int16 feature index per square; -1 where the square is empty."""
    n, squares = board.shape
    idx = np.full((n, squares), -1, dtype=np.int32)
    codes = board.astype(np.int32)
    occupied = codes != 0
    rows, cols = np.nonzero(occupied)
    code = codes[rows, cols]
    # codes are 1..6 for white pieces and 7..12 for black ones, so the colour is
    # "greater than 6" -- not code // 6, which misreads the white king (6 // 6 == 1).
    piece_type = (code - 1) % 6 + 1  # 1..6
    colour = (code > 6).astype(np.int32)  # 0 = white, 1 = black
    own = colour == stm[rows]
    block = np.where(own, piece_type - 1, 6 + (piece_type - 1))
    rel = np.where(own, cols, 63 - cols)
    idx[rows, cols] = block * 64 + rel
    return idx



def quantised_score(q, feats, h, scale, n_max=20000):
    """Run the engine's integer forward pass over a feature matrix.

    Used at export time to pick the weight scaling that preserves the float model,
    instead of guessing one. Mirrors nnue_eval() in engine/bb.ter exactly.
    """
    n = min(feats.shape[0], n_max)
    out = np.empty(n, dtype=np.float64)
    w1 = q["w1"].astype(np.int64)
    w2 = q["w2"].astype(np.int64)
    psqt = q["psqt"].astype(np.int64)
    bias = q["bias"].astype(np.int64)
    for i in range(n):
        f = feats[i]
        f = f[f >= 0]
        acc = np.clip(bias + w1[f].sum(axis=0), I16_MIN, I16_MAX)
        hidden = int(np.clip((acc * w2).sum(), I16_MIN, I16_MAX))
        value = int(np.clip(psqt[f].sum() + hidden, I16_MIN, I16_MAX))
        num = value * scale
        score = (abs(num) // 64) * (1 if num >= 0 else -1)
        out[i] = max(-8000, min(8000, score))
    return out

class Net:
    def __init__(self, hidden: int, rng: np.random.Generator, scale: int = 64):
        self.hidden = hidden
        self.scale = scale
        # Small init: a wide transformer with saturated activations is the usual failure mode.
        self.w1 = (rng.standard_normal((FEATURES, hidden)) * 0.05).astype(np.float32)
        self.w2 = (rng.standard_normal(hidden) * 0.05).astype(np.float32)
        self.psqt = (rng.standard_normal(FEATURES) * 3.0).astype(np.float32)
        self.bias = np.zeros(hidden, dtype=np.float32)
        self.step = 0
        self.m = {k: np.zeros_like(v) for k, v in self.params().items()}
        self.v = {k: np.zeros_like(v) for k, v in self.params().items()}

    def params(self):
        return {"w1": self.w1, "w2": self.w2, "psqt": self.psqt, "bias": self.bias}

    def forward(self, feats: np.ndarray, need_grad: bool = True):
        """feats: (N, 64) feature indices (-1 = empty). Returns score (N,) and cache."""
        n = feats.shape[0]
        h = self.hidden
        mask = feats >= 0
        rows = np.repeat(np.arange(n), mask.sum(axis=1))
        cols = np.nonzero(mask)[1]
        flat = feats[rows, cols]

        acc = np.tile(self.bias, (n, 1))
        # scatter each active feature's transformer row into the accumulator
        np.add.at(acc, (rows[:, None], np.arange(h)[None, :]), self.w1[flat])

        clipped = (acc < I16_MIN) | (acc > I16_MAX)
        acc_c = np.clip(acc, I16_MIN, I16_MAX)

        hidden = acc_c @ self.w2
        hid_clipped = (hidden < I16_MIN) | (hidden > I16_MAX)
        hidden_c = np.clip(hidden, I16_MIN, I16_MAX)

        psqt_sum = np.zeros(n, dtype=np.float32)
        np.add.at(psqt_sum, rows, self.psqt[flat])

        value = hidden_c + psqt_sum
        val_clipped = (value < I16_MIN) | (value > I16_MAX)
        value_c = np.clip(value, I16_MIN, I16_MAX)
        score = value_c * (self.scale / 64.0)

        cache = dict(rows=rows, cols=cols, flat=flat, mask=mask, n=n,
                     clipped=clipped, hid_clipped=hid_clipped, val_clipped=val_clipped,
                     acc_c=acc_c, value=value, value_c=value_c)
        return score, cache

    def backward(self, score: np.ndarray, target: np.ndarray, cache, lr: float, clip_grad: float = 1.0, decay: float = 0.0):
        n = cache["n"]
        rows, flat = cache["rows"], cache["flat"]
        acc_c, value = cache["acc_c"], cache["value"]

        dscore = (2.0 / n) * (score - target)  # d/d(score) of mean squared error
        dvalue = dscore * (self.scale / 64.0)
        # straight-through: saturated values pass no gradient
        if cache["val_clipped"].any():
            dvalue = np.where(cache["val_clipped"], 0.0, dvalue)
        dhidden = dvalue.copy()
        dhidden[cache["hid_clipped"]] = 0.0

        dacc = np.outer(dhidden, self.w2)
        dacc[cache["clipped"]] = 0.0

        dw1 = np.zeros_like(self.w1)
        np.add.at(dw1, flat, dacc[rows])
        dw2 = dhidden @ acc_c
        dbias = dacc.sum(axis=0)
        dpsqt = np.zeros(FEATURES, dtype=np.float32)
        np.add.at(dpsqt, flat, dvalue[rows])

        grads = {"w1": dw1, "w2": dw2, "psqt": dpsqt, "bias": dbias}
        self.apply(grads, lr, clip_grad, decay)

    def apply(self, grads, lr, clip_grad, decay=0.0):
        self.step += 1
        b1, b2, eps = 0.9, 0.999, 1e-8
        for name, grad in grads.items():
            if decay:
                grad = grad + decay * getattr(self, name)
            if clip_grad:
                norm = np.linalg.norm(grad)
                if norm > clip_grad:
                    grad = grad * (clip_grad / norm)
            self.m[name] = b1 * self.m[name] + (1 - b1) * grad
            self.v[name] = b2 * self.v[name] + (1 - b2) * (grad * grad)
            mhat = self.m[name] / (1 - b1 ** self.step)
            vhat = self.v[name] / (1 - b2 ** self.step)
            setattr(self, name, getattr(self, name) - lr * mhat / (np.sqrt(vhat) + eps))

    def quantise(self, a: float = 1.0, b: float = 1.0):
        """Round to i16 with separate transformer/output scales.

        w1 and w2 have very different magnitudes (|w1| ~ 1-2, |w2| ~ 0.05), so
        rounding both to integers throws most of the model away: a float-path
        correlation of +0.52 became +0.36 after export. Scaling w1/bias by `a` and
        w2/psqt by `b` keeps the float function unchanged up to the factor a*b
        (acc scales by a; the w2 dot product and the psqt sum scale by a*b), so the
        engine's `scale` field is multiplied by a*b. Which (a, b) is best depends on
        the trained magnitudes, so `pick_quantisation` measures instead of guessing.
        """
        return {
            "w1": np.clip(np.round(self.w1 * a), I16_MIN, I16_MAX).astype(np.int16),
            "w2": np.clip(np.round(self.w2 * b), I16_MIN, I16_MAX).astype(np.int16),
            "psqt": np.clip(np.round(self.psqt * b), I16_MIN, I16_MAX).astype(np.int16),
            "bias": np.clip(np.round(self.bias * a), I16_MIN, I16_MAX).astype(np.int16),
        }

    def write_tnn(self, path: str, a: float = 1.0, b: float = 1.0):
        """Write the engine's `.tnn` layout.

        The engine keeps a white-view and a black-view accumulator and selects
        acc[stm] at eval time. Because features are always built in the side to
        move's frame, one transformer serves both views, so the two tables in
        the file are identical. (Splitting them later is a retraining choice,
        not a format change.)
        """
        q = self.quantise()
        with open(path, "wb") as out:
            out.write(MAGIC)
            out.write(struct.pack("<i", VERSION))
            out.write(struct.pack("<i", self.hidden))
            out.write(struct.pack("<i", max(1, int(self.scale * a * b))))
            for arr in (q["bias"], q["w2"], q["psqt"], q["w1"], q["w1"]):
                out.write(arr.astype("<i2").tobytes())



def pick_quantisation(net, feats, result, scale, candidates=None, verbose=True):
    """Choose (a, b) by measuring which one preserves the trained model best.

    The float model is the thing we care about; the file only has i16 weights. Each
    candidate is scored by running the engine's own integer forward pass on held-out
    positions and taking the MSE, so the choice is measured, not assumed.
    """
    if candidates is None:
        candidates = [(1, 1), (4, 4), (8, 4), (8, 8), (16, 4), (16, 8), (32, 4),
                      (32, 8), (64, 4), (64, 8), (128, 2), (128, 4)]
    best = None
    for a, b in candidates:
        q = net.quantise(a, b)
        qs = quantised_score(q, feats, net.hidden, max(1, int(scale * a * b)))
        # Rank by correlation with the actual game result, not MSE: MSE is dominated
        # by the bulk of near-draw positions and says little about ranking quality.
        r = np.asarray(result)[:len(qs)]   # quantised_score caps at n_max
        corr = float(np.corrcoef(qs, r)[0, 1]) if len(r) > 2 and qs.std() > 0 else 0.0
        if verbose:
            print(f"    candidate w1 x{a:<4} w2 x{b:<3} -> corr {corr:+.4f}")
        if best is None or corr > best[0]:
            best = (corr, a, b)
    return best[1], best[2], best[0]

def evaluate(net: Net, feats, target, batch=4096) -> float:
    total, count = 0.0, 0
    for i in range(0, feats.shape[0], batch):
        sl = slice(i, i + batch)
        score, _ = net.forward(feats[sl], need_grad=False)
        total += float(np.sum((score - target[sl]) ** 2))
        count += len(score)
    return total / max(count, 1)


def load_tnn(path: str):
    """Read a .tnn back into integer arrays, mirroring the engine's arithmetic."""
    raw = np.fromfile(path, dtype=np.uint8)
    if raw[:4].tobytes() != MAGIC:
        raise ValueError(f"{path}: bad magic")
    version, hidden, scale = struct.unpack_from("<iii", raw.tobytes(), 4)
    if version != VERSION:
        raise ValueError(f"{path}: unsupported version {version}")
    off = 16
    i16 = lambda n: raw[off:off + 2 * n].view("<i2").astype(np.int64).copy() or None  # noqa: E731
    off_get = lambda n: (raw[off:off + 2 * n].view("<i2").astype(np.int64).copy(), off + 2 * n)  # noqa: E731
    bias, off = off_get(hidden)
    w2, off = off_get(hidden)
    psqt, off = off_get(FEATURES)
    w1, off = off_get(FEATURES * hidden)
    _, off = off_get(FEATURES * hidden)  # the second (view-b) table is identical in v1
    # w1 is stored row-major (feature * hidden + unit) -- it MUST be reshaped before
    # indexing by feature, or w1[f] silently becomes a single weight.
    w1 = w1.reshape(FEATURES, hidden)
    return {"hidden": hidden, "scale": scale, "bias": bias, "w2": w2, "psqt": psqt, "w1": w1}


def score_fen(net, fen: str) -> int:
    """Exactly the engine's integer evaluation for a FEN."""
    import chess

    board = chess.Board(fen)
    codes = np.zeros(64, dtype=np.int32)
    for square in chess.SQUARES:
        piece = board.piece_at(square)
        if piece is None:
            continue
        from_code = {1: chess.PAWN, 2: chess.KNIGHT, 3: chess.BISHOP, 4: chess.ROOK, 5: chess.QUEEN, 6: chess.KING}
        base = from_code[[k for k, v in from_code.items() if v == piece.piece_type][0]]
        codes[square] = base + (6 if piece.color == chess.BLACK else 0)
    stm = 0 if board.turn == chess.WHITE else 1
    idx = feature_indices(codes.reshape(1, 64), np.array([stm], dtype=np.uint8))[0]

    active = np.nonzero(idx >= 0)[0]
    feats = idx[active]
    acc = net["bias"] + net["w1"][feats].sum(axis=0)
    acc = np.clip(acc, I16_MIN, I16_MAX)
    hidden = int(np.clip((acc * net["w2"]).sum(), I16_MIN, I16_MAX))
    value = int(np.clip(net["psqt"][feats].sum() + hidden, I16_MIN, I16_MAX))
    # C truncates integer division toward zero; Python's // floors. Match the engine.
    num = value * net["scale"]
    score = (abs(num) // 64) * (1 if num >= 0 else -1)
    return int(np.clip(score, -8000, 8000))


def board_codes(board) -> np.ndarray:
    """(64,) uint8 engine piece codes for a python-chess board."""
    import chess

    from_code = {chess.PAWN: 1, chess.KNIGHT: 2, chess.BISHOP: 3,
                 chess.ROOK: 4, chess.QUEEN: 5, chess.KING: 6}
    codes = np.zeros(64, dtype=np.uint8)
    for square in chess.SQUARES:
        piece = board.piece_at(square)
        if piece is not None:
            codes[square] = from_code[piece.piece_type] + (6 if piece.color == chess.BLACK else 0)
    return codes


def selftest() -> int:
    """Pin the feature convention and prove the .tnn round-trip is exact."""
    import chess
    import tempfile

    cases = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
        "8/P6k/8/8/8/8/7K/8 w - - 0 1",
    ]
    for fen in cases:
        board = chess.Board(fen)
        idx = feature_indices(board_codes(board).reshape(1, 64),
                              np.array([0 if board.turn == chess.WHITE else 1], dtype=np.uint8))[0]
        view = 0 if board.turn == chess.WHITE else 1
        active = sorted(int(v) for v in idx if v >= 0)
        assert active == active_features_from_chess(board, view), (fen, active[:8])

    kings = chess.Board("8/8/8/8/8/8/8/K6k w - - 0 1")
    assert active_features_from_chess(kings, 0) == [320, 760]
    assert active_features_from_chess(kings, 1) == [327, 767]

    rng = np.random.default_rng(7)
    net = Net(8, rng)
    net.w1[:] = rng.standard_normal(net.w1.shape) * 0.1
    net.psqt[:] = rng.standard_normal(FEATURES) * 2.0
    with tempfile.NamedTemporaryFile(suffix=".tnn", delete=False) as tmp:
        path = tmp.name
    net.write_tnn(path)
    q = net.quantise()
    fen = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3"
    assert score_fen(load_tnn(path), fen) == score_fen(
        {"hidden": net.hidden, "scale": net.scale,
         "bias": q["bias"].astype(np.int64), "w2": q["w2"].astype(np.int64),
         "psqt": q["psqt"].astype(np.int64), "w1": q["w1"].astype(np.int64)}, fen
    ), "quantised forward differs from the file"
    print("selftest OK (feature convention pinned, .tnn round-trip exact)")
    return 0


def active_features_from_chess(board, view: int) -> list:
    """Feature indices the engine would activate for one view (0 white, 1 black)."""
    import chess

    from_code = {chess.PAWN: 1, chess.KNIGHT: 2, chess.BISHOP: 3,
                 chess.ROOK: 4, chess.QUEEN: 5, chess.KING: 6}
    feats = []
    for square in chess.SQUARES:
        piece = board.piece_at(square)
        if piece is None:
            continue
        pt = from_code[piece.piece_type]
        colour = 0 if piece.color == chess.WHITE else 1
        is_own = colour == view
        block = pt - 1 if is_own else 6 + (pt - 1)
        rel = square if is_own else 63 - square
        feats.append(block * 64 + rel)
    return sorted(feats)


def check_fen(args) -> int:
    import chess

    net = load_tnn(args.out)
    print(f"loaded {args.out}: hidden={net['hidden']} scale={net['scale']}")
    score = score_fen(net, args.check_fen)
    print(score)
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--data")
    ap.add_argument("--out")
    ap.add_argument("--hidden", type=int, default=32)
    ap.add_argument("--epochs", type=int, default=20)
    ap.add_argument("--batch", type=int, default=256)
    ap.add_argument("--lr", type=float, default=0.002)
    ap.add_argument("--scale", type=int, default=64)
    ap.add_argument("--target-cp", type=float, default=400.0, help="magnitude of a win target")
    ap.add_argument("--val-frac", type=float, default=0.1)
    ap.add_argument("--seed", type=int, default=12345)
    ap.add_argument("--weight-decay", type=float, default=1e-5)
    ap.add_argument("--schedule", choices=("constant", "cosine"), default="cosine",
                    help="cosine decay with warmup usually generalises better than a flat lr")
    ap.add_argument("--warmup-epochs", type=int, default=5)
    ap.add_argument("--drop-draws", action="store_true",
                    help="train only on decisive positions (experiment: how much does the "
                         "draw-heavy self-play data dilute the signal?)")
    ap.add_argument("--check-fen", metavar="FEN", help="print the engine-equivalent score for a FEN")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.check_fen:
        if not args.out:
            ap.error("--out is required with --check-fen")
        return check_fen(args)
    if args.selftest:
        return selftest()
    if not args.data or not args.out:
        ap.error("--data and --out are required when training")

    board, stm, game, result = load_records(args.data)
    n = board.shape[0]
    print(f"loaded {n} positions from {args.data}")
    if n < 64:
        print("not enough data to train", file=sys.stderr)
        return 1

    if args.drop_draws:
        keep = result != 0.5
        board, stm, game, result = board[keep], stm[keep], game[keep], result[keep]
        n = board.shape[0]
        print(f"dropped draws: training on {n} decisive positions")

    feats = feature_indices(board, stm)
    target = (result * 2.0 - 1.0) * args.target_cp  # win +cp, draw 0, loss -cp

    # Split by GAME, not by position: positions from the same game share openings
    # and tactics, so a random position split flatters the model badly.
    rng = np.random.default_rng(args.seed)
    games = np.unique(game)
    rng.shuffle(games)
    val_games = set(int(g) for g in games[:max(1, int(len(games) * args.val_frac))])
    is_val = np.array([int(g) in val_games for g in game])
    train_idx = np.nonzero(~is_val)[0]
    val_idx = np.nonzero(is_val)[0]
    print(f"split by game: {len(games) - len(val_games)} train games / {len(val_games)} val games"
          f" ({len(train_idx)} / {len(val_idx)} positions)")

    net = Net(args.hidden, rng, scale=args.scale)
    base = evaluate(net, feats[train_idx], target[train_idx])
    print(f"params: {sum(p.size for p in net.params().values())}   initial train MSE: {base:.1f}")

    best = base
    best_state = {k: v.copy() for k, v in net.params().items()}
    start = time.time()
    for epoch in range(1, args.epochs + 1):
        # Shuffle by game, then by position inside each game. Positions from one
        # game are correlated, so a flat position shuffle wastes gradient steps.
        gorder = rng.permutation(np.unique(game[train_idx]))
        chunks = [train_idx[game[train_idx] == g] for g in gorder]
        for chunk in chunks:
            rng.shuffle(chunk)
        ordered = np.concatenate(chunks) if chunks else train_idx

        if args.schedule == "cosine":
            if epoch <= args.warmup_epochs:
                lr = args.lr * epoch / max(1, args.warmup_epochs)
            else:
                t = (epoch - args.warmup_epochs) / max(1, args.epochs - args.warmup_epochs)
                lr = args.lr * 0.5 * (1.0 + math.cos(math.pi * t))
        else:
            lr = args.lr

        for i in range(0, len(ordered), args.batch):
            sel = ordered[i:i + args.batch]
            score, cache = net.forward(feats[sel])
            net.backward(score, target[sel], cache, lr, decay=args.weight_decay)
        if epoch % max(1, args.epochs // 10) == 0 or epoch == args.epochs:
            tr = evaluate(net, feats[train_idx], target[train_idx])
            va = evaluate(net, feats[val_idx], target[val_idx]) if len(val_idx) else float("nan")
            flag = ""
            if va == va and va < best:
                best = va
                best_state = {k: v.copy() for k, v in net.params().items()}
                flag = " *"
            print(f"epoch {epoch:3d}  train {tr:9.1f}  val {va:9.1f}{flag}"
                  f"  ({time.time() - start:.1f}s)")

    for name, value in best_state.items():
        setattr(net, name, value)

    # Calibrate the output scale on held-out data: score = value * scale / 64, so
    # scaling the net's output is just a change of the scale field. Without this an
    # overfit net emits absurd centipawn values.
    if len(val_idx):
        raw_score, _ = net.forward(feats[val_idx], need_grad=False)
        denom = float(np.sum(raw_score.astype(np.float64) ** 2))
        a = float(np.sum(raw_score.astype(np.float64) * target[val_idx])) / denom if denom > 0 else 1.0
        a = float(np.clip(a, 0.01, 10.0))
        net.scale = max(1, int(round(net.scale * a)))
        print(f"  calibration factor {a:.4f} -> scale {net.scale}"
              f"  (val MSE {evaluate(net, feats[val_idx], target[val_idx]):.1f})")

    a, b, q_corr = pick_quantisation(net, feats[val_idx], result[val_idx], net.scale)
    net.write_tnn(args.out, a=a, b=b)
    print(f"  quantisation: w1 x{a}, w2 x{b} (int-path val correlation {q_corr:+.3f})")

    q = net.quantise(a, b)
    print(f"wrote {args.out} (hidden={args.hidden}, scale={args.scale})")
    print(f"  w1 range   [{q['w1'].min()}, {q['w1'].max()}]")
    print(f"  psqt range [{q['psqt'].min()}, {q['psqt'].max()}]")
    print(f"  w2 range   [{q['w2'].min()}, {q['w2'].max()}]")

    # Correlation between the net's score and the actual result, on held-out data.
    if len(val_idx):
        score, _ = net.forward(feats[val_idx], need_grad=False)
        corr = float(np.corrcoef(score, result[val_idx])[0, 1])
        qs = quantised_score(net.quantise(a, b), feats[val_idx], net.hidden,
                             max(1, int(net.scale * a * b)))
        icorr = float(np.corrcoef(qs, np.asarray(result[val_idx])[:len(qs)])[0, 1])
        print(f"  val correlation float {corr:+.3f} / int16 path {icorr:+.3f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())