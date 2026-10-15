#!/usr/bin/env python3
"""Turn self-play PGNs into Tessera NNUE training data.

Two modes:

  gen_data.py --pgn games.pgn --out data.bin
      Replays every game with python-chess and writes one 40-byte record per
      sampled position:
          bytes  0..31  board, 4 bits per square, square 0 = a1 (engine order)
          byte  32      side to move (0 = white)
          byte  33      castling rights mask (WK WQ BK BQ)
          byte  34      en-passant square (255 = none)
          byte  35      game id (mod 256) so validation can split by game, not by
                         position -- positions from one game share openings and
                         tactics, and a random position split leaks badly
          bytes 36..39  float32 result from the side to move's point of view
                                (1.0 win, 0.5 draw, 0.0 loss)

  gen_data.py --features "<fen>"
      Prints the active feature indices for a position, using exactly the same
      convention as the engine's `feat()` in engine/bb.ter. Used to cross-check
      the two implementations.

Feature convention (must match engine/bb.ter):
      piece codes 1..6 = P N B R Q K (white), 7..12 = the black pieces
      own piece  -> block  pt - 1,          square sq
      enemy piece-> block  6 + (pt - 1),    square 63 - sq
      index = block * 64 + square
"""

import argparse
import struct
import sys

import chess
import chess.pgn

RECORD_SIZE = 40

# Piece code used by the engine's Pos: 1..6 for white, negated for black.
PIECE_CODE = {
    chess.PAWN: 1,
    chess.KNIGHT: 2,
    chess.BISHOP: 3,
    chess.ROOK: 4,
    chess.QUEEN: 5,
    chess.KING: 6,
}


def encode_board(board: chess.Board) -> bytes:
    """32 bytes: four bits per square, a1 first (matching the engine's sq)."""
    out = bytearray(32)
    for square in chess.SQUARES:
        piece = board.piece_at(square)
        code = 0
        if piece is not None:
            code = PIECE_CODE[piece.piece_type]
            if piece.color == chess.BLACK:
                code += 6
        if square % 2 == 0:
            out[square // 2] |= code
        else:
            out[square // 2] |= code << 4
    return bytes(out)


def encode_castling(board: chess.Board) -> int:
    """4-bit mask: WK=1, WQ=2, BK=4, BQ=8 (same bit order as the engine)."""
    field = board.fen().split(" ")[2]
    mask = 0
    if "K" in field:
        mask |= 1
    if "Q" in field:
        mask |= 2
    if "k" in field:
        mask |= 4
    if "q" in field:
        mask |= 8
    return mask


def castling_fen(mask: int) -> str:
    text = ""
    if mask & 1:
        text += "K"
    if mask & 2:
        text += "Q"
    if mask & 4:
        text += "k"
    if mask & 8:
        text += "q"
    return text or "-"


def encode_ep(board: chess.Board) -> int:
    return board.ep_square if board.ep_square is not None else 255


def record(board: chess.Board, result_from_stm: float, game_id: int = 0) -> bytes:
    return (
        encode_board(board)
        + bytes([0 if board.turn == chess.WHITE else 1, encode_castling(board),
                 encode_ep(board), game_id & 0xFF])
        + struct.pack("<f", result_from_stm)
    )


def active_features(board: chess.Board) -> list[int]:
    """The 768-sparse features the engine would activate, per view."""
    feats = []
    for square in chess.SQUARES:
        piece = board.piece_at(square)
        if piece is None:
            continue
        pt = PIECE_CODE[piece.piece_type]
        own_white = piece.color == chess.WHITE
        for view in (0, 1):  # 0 = white view, 1 = black view
            is_own = (own_white and view == 0) or (not own_white and view == 1)
            block = pt - 1 if is_own else 6 + (pt - 1)
            rel = square if is_own else 63 - square
            feats.append(block * 64 + rel)
    return sorted(feats)


def game_result_pov(game: chess.pgn.Game, board: chess.Board) -> float:
    """Game result from the point of view of the side to move."""
    outcome = game.headers.get("Result", "*")
    if outcome == "1/2-1/2":
        return 0.5
    if outcome == "1-0":
        white_won = True
    elif outcome == "0-1":
        white_won = False
    else:
        return 0.5
    white_to_move = board.turn == chess.WHITE
    won = white_won if white_to_move else not white_won
    return 1.0 if won else 0.0


def convert(args) -> int:
    games = 0
    game_index = 0
    kept = 0
    skipped_games = 0
    records: list[bytes] = []
    results = {1.0: 0, 0.5: 0, 0.0: 0}

    with open(args.pgn, "r", encoding="utf-8", errors="replace") as handle:
        while True:
            game = chess.pgn.read_game(handle)
            if game is None:
                break
            games += 1
            termination = game.headers.get("Termination", "normal")
            if args.skip_time_forfeits and termination not in ("normal", "adjudication"):
                skipped_games += 1
                continue

            board = game.board()
            plies = 0
            game_records = []
            for move in game.mainline_moves():
                if plies >= args.min_plies:
                    game_records.append(record(board, game_result_pov(game, board), game_index))
                board.push(move)
                plies += 1

            if not game_records:
                skipped_games += 1
                continue
            kept += 1
            game_index += 1
            records.extend(game_records)

    for rec in records:
        results[struct.unpack_from("<f", rec, 36)[0]] += 1

    with open(args.out, "wb") as out:
        for rec in records:
            out.write(rec)

    total = len(records)
    print(f"games parsed      : {games}")
    print(f"games used        : {kept} (skipped {skipped_games})")
    print(f"positions written : {total}")
    if total:
        print(f"  wins (stm)      : {results[1.0]}")
        print(f"  draws           : {results[0.5]}")
        print(f"  losses (stm)    : {results[0.0]}")
        print(f"  bytes           : {total * RECORD_SIZE}")
    return 0


def dump_features(args) -> int:
    board = chess.Board(args.features)
    for idx in active_features(board):
        print(idx)
    return 0



def decode_record(rec: bytes) -> chess.Board:
    """Inverse of record(): rebuild a python-chess Board from a 40-byte record."""
    board = chess.Board.empty()
    for square in chess.SQUARES:
        byte = rec[square // 2]
        code = (byte & 0x0F) if square % 2 == 0 else (byte >> 4)
        if code == 0:
            continue
        color = chess.WHITE if code <= 6 else chess.BLACK
        base = code if code <= 6 else code - 6
        for piece_type, value in PIECE_CODE.items():
            if value == base:
                board.set_piece_at(square, chess.Piece(piece_type, color))
                break
    board.set_castling_fen(castling_fen(rec[33]))
    if rec[34] != 255:
        board.ep_square = rec[34]
    # set_castling_fen resets the side to move, so restore it afterwards.
    board.turn = chess.WHITE if rec[32] == 0 else chess.BLACK
    board.halfmove_clock = 0
    board.fullmove_number = 1
    return board


def dump(args) -> int:
    with open(args.out, "rb") as handle:
        data = handle.read()
    count = min(args.dump, len(data) // RECORD_SIZE)
    for i in range(count):
        rec = data[i * RECORD_SIZE:(i + 1) * RECORD_SIZE]
        result = struct.unpack_from("<f", rec, 36)[0]
        print(f"{decode_record(rec).fen()}  result={result}")
    return 0

def selftest() -> int:
    """Round-trip the record encoder and pin the feature convention."""
    cases = [
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
        "8/P6k/8/8/8/8/7K/8 w - - 0 1",
    ]
    for fen in cases:
        board = chess.Board(fen)
        back = decode_record(record(board, 0.5))
        assert back.fen().split(" ")[0] == board.fen().split(" ")[0], (fen, back.fen())
        assert back.turn == board.turn, fen
        assert castling_fen(encode_castling(board)) == board.fen().split(" ")[2], fen
        assert back.ep_square == board.ep_square, fen

    # Feature convention pinned against engine/bb.ter: white king a1, black king h1.
    kings = chess.Board("8/8/8/8/8/8/8/K6k w - - 0 1")
    assert active_features(kings) == [320, 327, 760, 767], active_features(kings)

    # A pawn on e2 (sq 12) is white-owned in the white view: block 0, rel 12.
    start = chess.Board()
    assert 12 in active_features(start)
    assert 52 in active_features(start)  # e7 black pawn -> enemy block 6, rel 63-52

    print(f"selftest OK ({len(cases)} round-trips, feature convention pinned)")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--pgn")
    ap.add_argument("--out", default="data.bin")
    ap.add_argument("--min-plies", type=int, default=8)
    ap.add_argument("--skip-time-forfeits", action="store_true", default=True)
    ap.add_argument("--keep-time-forfeits", dest="skip_time_forfeits", action="store_false")
    ap.add_argument("--dump", type=int, default=0, help="print FENs of the first N records")
    ap.add_argument("--features", metavar="FEN")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()

    if args.selftest:
        return selftest()
    if args.features:
        return dump_features(args)
    if args.dump:
        return dump(args)
    if not args.pgn:
        ap.error("either --pgn or --features is required")
    return convert(args)


if __name__ == "__main__":
    sys.exit(main())
