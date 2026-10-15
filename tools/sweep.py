"""Hyper-parameter sweep for the Tessera NNUE trainer.

Runs a handful of configurations on the same split and prints a table, so the
choice of hidden width / learning rate / schedule is measured rather than guessed.

    python tools/sweep.py --data selfplay.bin --outdir sweep --configs quick
"""

import argparse
import io
import re
import subprocess
import sys
import time
from pathlib import Path

CONFIGS = {
    "quick": [
        {"hidden": 16, "lr": 0.002, "epochs": 22, "batch": 512, "weight-decay": 1e-4, "schedule": "constant"},
        {"hidden": 32, "lr": 0.002, "epochs": 22, "batch": 512, "weight-decay": 1e-4, "schedule": "constant"},
        {"hidden": 32, "lr": 0.002, "epochs": 22, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine"},
        {"hidden": 64, "lr": 0.002, "epochs": 22, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine"},
    ],
    "wide": [
        {"hidden": 32, "lr": 0.002, "epochs": 80, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine"},
        {"hidden": 64, "lr": 0.002, "epochs": 80, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine"},
        {"hidden": 128, "lr": 0.0015, "epochs": 80, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine"},
        {"hidden": 64, "lr": 0.002, "epochs": 80, "batch": 512, "weight-decay": 1e-4, "schedule": "cosine",
         "drop-draws": True},
    ],
}

TRAIN = str(Path(__file__).with_name("train.py"))


def run_config(cfg, data, outdir):
    out = Path(outdir) / (f"h{cfg['hidden']}_{cfg['schedule']}"
                          f"{'_nodraws' if cfg.get('drop-draws') else ''}.tnn")
    out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [sys.executable, TRAIN, "--data", data, "--out", str(out),
           "--hidden", str(cfg["hidden"]), "--lr", str(cfg["lr"]),
           "--epochs", str(cfg["epochs"]), "--batch", str(cfg["batch"]),
           "--weight-decay", str(cfg["weight-decay"]), "--schedule", cfg["schedule"]]
    if cfg.get("drop-draws"):
        cmd.append("--drop-draws")
    start = time.time()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    text = proc.stdout
    mse = re.search(r"val ([0-9.]+)\s*\*?\s*\(", text)
    corr = re.search(r"val correlation score vs result: ([+-][0-9.]+)", text)
    scale = re.search(r"calibration factor [0-9.]+ -> scale (\d+)", text)
    last = re.findall(r"val\s+([0-9.]+)", text)
    return {
        "hidden": cfg["hidden"],
        "schedule": cfg["schedule"],
        "nodraws": bool(cfg.get("drop-draws")),
        "val_mse": float(mse.group(1)) if mse else (float(last[-1]) if last else float("nan")),
        "corr": float(corr.group(1)) if corr else float("nan"),
        "scale": scale.group(1) if scale else "?",
        "secs": round(time.time() - start, 1),
        "net": str(out),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--outdir", default="sweep")
    ap.add_argument("--configs", default="quick", choices=sorted(CONFIGS))
    args = ap.parse_args()

    rows = [run_config(cfg, args.data, args.outdir) for cfg in CONFIGS[args.configs]]
    print()
    print(f"{'hidden':>7} {'schedule':>9} {'nodraws':>8} {'val mse':>12} {'corr':>7} {'scale':>6} {'secs':>7}")
    for r in rows:
        print(f"{r['hidden']:>7} {r['schedule']:>9} {str(r['nodraws']):>8} "
              f"{r['val_mse']:>12.1f} {r['corr']:>+7.3f} {r['scale']:>6} {r['secs']:>7}")
    best = max(rows, key=lambda r: r["corr"])
    print(f"\nbest by correlation: hidden={best['hidden']} {best['schedule']} -> {best['net']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
