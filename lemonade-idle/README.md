# lemonade-idle

Launch AI agents only when your local GPU is free.

Polls [Lemonade Server](https://lemonade-server.ai/)'s `/api/v1/system-stats` endpoint on a configurable interval and maintains a sliding-window log of GPU utilisation. A guard script checks the window before any agent is launched — if the GPU peaked above the threshold at any point in the window, the launch is skipped.

## How it works

```
lemonade-sampler.service  →  gpu.jsonl (rolling 15min log)
                                   ↓
codex-agent.timer  →  codex-agent.service  →  idle-guard.py  →  codex ...
                                                    ↑ exits 1 if busy → skipped
```

## Setup

### 1. Install the sampler

```bash
cp sampler.py idle-guard.py ~/.local/share/lemonade-idle/
cp systemd/lemonade-sampler.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now lemonade-sampler.service
```

The sampler needs at least `--min-samples` readings before the guard will allow a launch (default: 10 samples = 5 minutes).

### 2. Configure the agent service

Copy and edit the agent service template:

```bash
cp systemd/codex-agent.service ~/.config/systemd/user/
cp systemd/codex-agent.timer   ~/.config/systemd/user/
```

Edit `~/.config/systemd/user/codex-agent.service` and replace `AGENT_COMMAND` with your actual command, e.g.:

```ini
ExecStart=/usr/bin/bash -c 'codex --skill gh-fix-issue "Fix next open issue in owner/repo"'
```

Then enable:

```bash
systemctl --user daemon-reload
systemctl --user enable --now codex-agent.timer
```

### 3. Check it's working

```bash
# Watch sampler output
journalctl --user -u lemonade-sampler.service -f

# Test the guard manually
python3 ~/.local/share/lemonade-idle/idle-guard.py
echo $?   # 0 = idle, 1 = busy

# See timer schedule
systemctl --user list-timers codex-agent.timer
```

## Configuration

All settings are environment variables — set them in the `[Service]` section of the unit files.

| Variable | Default | Description |
|---|---|---|
| `LEMONADE_URL` | `http://localhost:13305` | Lemonade Server URL |
| `SAMPLE_INTERVAL_S` | `30` | Seconds between GPU readings |
| `WINDOW_S` | `900` | Sliding window size in seconds (15min) |
| `GPU_THRESHOLD` | `25` | Max GPU % to be considered idle |
| `MIN_SAMPLES` | `10` | Minimum samples required before guard will allow a launch |
| `LEMONADE_STATS_FILE` | `~/.local/share/lemonade-idle/gpu.jsonl` | Path to the rolling stats log |

## Notes

- The default threshold of 25% accounts for GPU overhead from a model sitting loaded in VRAM — pure idle on a loaded model is rarely 0%.
- The timer fires every 30 minutes with a 2-minute random jitter. Adjust `OnUnitActiveSec` in `codex-agent.timer` to taste.
- You can run multiple agent services against the same stats file — they all share one sampler.
