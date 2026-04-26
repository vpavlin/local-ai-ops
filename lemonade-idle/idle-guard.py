#!/usr/bin/env python3
"""
Exits 0 if Lemonade Server GPU has been below THRESHOLD% for the entire
sliding window. Exits 1 (busy) otherwise.

Usage: idle-guard.py [--threshold N] [--window-s N] [--min-samples N]
"""
import json
import os
import sys
import time
import argparse

STATS_FILE = os.environ.get("LEMONADE_STATS_FILE",
             os.path.expanduser("~/.local/share/lemonade-idle/gpu.jsonl"))

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--threshold",   type=float, default=float(os.environ.get("GPU_THRESHOLD",   "25")))
    p.add_argument("--window-s",    type=int,   default=int(os.environ.get("WINDOW_S",          "900")))
    p.add_argument("--min-samples", type=int,   default=int(os.environ.get("MIN_SAMPLES",       "10")))
    args = p.parse_args()

    now    = int(time.time())
    cutoff = now - args.window_s

    if not os.path.exists(STATS_FILE):
        print("BUSY: no stats file yet — sampler not running?", file=sys.stderr)
        sys.exit(1)

    samples = []
    with open(STATS_FILE) as f:
        for line in f:
            try:
                e = json.loads(line)
                if e["ts"] >= cutoff:
                    samples.append(e)
            except Exception:
                pass

    if len(samples) < args.min_samples:
        print(f"BUSY: only {len(samples)} samples in window (need {args.min_samples}) — too early to decide", file=sys.stderr)
        sys.exit(1)

    peak = max(s["gpu"] for s in samples)
    if peak > args.threshold:
        print(f"BUSY: peak GPU was {peak:.1f}% in the last {args.window_s//60}min (threshold {args.threshold}%)", file=sys.stderr)
        sys.exit(1)

    avg = sum(s["gpu"] for s in samples) / len(samples)
    print(f"IDLE: {len(samples)} samples, peak {peak:.1f}%, avg {avg:.1f}% — below {args.threshold}%", file=sys.stderr)
    sys.exit(0)

if __name__ == "__main__":
    main()
