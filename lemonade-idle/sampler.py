#!/usr/bin/env python3
"""
Polls Lemonade Server /api/v1/system-stats every INTERVAL seconds and
appends readings to a JSONL file. Prunes entries older than MAX_AGE_S
on each write to keep the file small.
"""
import json
import os
import time
import urllib.request
import urllib.error

LEMONADE_URL = os.environ.get("LEMONADE_URL", "http://localhost:13305")
INTERVAL     = int(os.environ.get("SAMPLE_INTERVAL_S", "30"))
MAX_AGE_S    = int(os.environ.get("WINDOW_S", "900"))          # 15 minutes
OUT_FILE     = os.environ.get("LEMONADE_STATS_FILE",
               os.path.expanduser("~/.local/share/lemonade-idle/gpu.jsonl"))

def fetch_gpu():
    try:
        with urllib.request.urlopen(f"{LEMONADE_URL}/api/v1/system-stats", timeout=5) as r:
            return json.loads(r.read())["gpu_percent"]
    except Exception:
        return None

def prune_and_append(entry):
    cutoff = entry["ts"] - MAX_AGE_S
    lines = []
    if os.path.exists(OUT_FILE):
        with open(OUT_FILE) as f:
            for line in f:
                try:
                    if json.loads(line)["ts"] >= cutoff:
                        lines.append(line.rstrip())
                except Exception:
                    pass
    lines.append(json.dumps(entry))
    with open(OUT_FILE, "w") as f:
        f.write("\n".join(lines) + "\n")

if __name__ == "__main__":
    print(f"Sampling {LEMONADE_URL} every {INTERVAL}s, window {MAX_AGE_S}s → {OUT_FILE}")
    while True:
        gpu = fetch_gpu()
        if gpu is not None:
            entry = {"ts": int(time.time()), "gpu": gpu}
            prune_and_append(entry)
            print(f"{time.strftime('%H:%M:%S')}  gpu={gpu:.1f}%", flush=True)
        else:
            print(f"{time.strftime('%H:%M:%S')}  unreachable", flush=True)
        time.sleep(INTERVAL)
