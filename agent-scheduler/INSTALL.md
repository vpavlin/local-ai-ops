# Installing laio on Bosgame

Step-by-step guide for setting up the full agent-scheduler stack on Bosgame
(AMD Strix Halo, running Lemonade Server with Qwen3.6-35B-A3B at `:8000`).

---

## Prerequisites

Install these if not already present:

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# GitHub CLI
sudo dnf install gh          # Fedora/RHEL
# or: sudo apt install gh    # Debian/Ubuntu
# Verify: gh --version

# Podman (rootless)
sudo dnf install podman
# Verify: podman info

# Node.js 22 (for Codex CLI inside containers)
# Only needed if building the container image locally
sudo dnf install nodejs

# Codex CLI (inside containers — installs during image build)
# To run Codex manually on the host for testing:
npm install -g @openai/codex
```

---

## 1. Clone the repo

```bash
git clone https://github.com/vpavlin/local-ai-ops.git ~/local-ai-ops
cd ~/local-ai-ops/agent-scheduler
```

---

## 2. Configure

```bash
mkdir -p ~/.config/laio
cp config.yaml.example ~/.config/laio/config.yaml
```

Edit `~/.config/laio/config.yaml`:

```yaml
database:
  path: ~/.local/share/laio/tasks.db
  task_dir: ~/.local/share/laio/tasks

orchestrator:
  # K11 (GMKTec) runs DeepSeek-Coder-V2-Lite for triage
  endpoint: "http://192.168.0.125:13305"
  model: deepseek-coder-v2-lite
  max_tasks_per_scan: 20
  max_retries: 3

executors:
  - name: bosgame-35b
    endpoint: "http://127.0.0.1:8000"   # local Lemonade on Bosgame
    model: Qwen3.6-35B-A3B
    max_concurrent: 1
  - name: gmktec-35b
    endpoint: "http://192.168.0.125:13305"
    model: Qwen3.6-35B-A3B
    max_concurrent: 1

repos:
  - url: https://github.com/YOUR_ORG/YOUR_REPO
    actions: [fix-issue, review-pr]
    priority: 5
  # Add more repos here

timeouts:
  default_s: 14400        # 4h
  per_type:
    fix-issue: 14400
    review-pr:  3600
  per_endpoint:
    gmktec-35b: 21600     # 6h — iGPU is slower
  heartbeat_interval_s: 120
  heartbeat_stale_after_s: 600
  lock_max_age_s: 86400

idle_guard:
  enabled: true
  stats_file: ~/.local/share/lemonade-idle/gpu.jsonl
  threshold_pct: 25.0
  min_samples: 10
  window_s: 900

container:
  image: local-ai-ops-runner:latest
  runtime: podman
  gh_token_env: GH_TOKEN

skills:
  dir: ~/.codex/skills
  auto_update: true
```

---

## 3. Set up GitHub token

Create a token at https://github.com/settings/tokens with `repo` scope.

```bash
mkdir -p ~/.config/environment.d
cat > ~/.config/environment.d/laio.conf <<'EOF'
GH_TOKEN=ghp_YOUR_TOKEN_HERE
EOF
chmod 600 ~/.config/environment.d/laio.conf
```

Also authenticate the gh CLI (one-time):

```bash
gh auth login
# Choose: GitHub.com → HTTPS → paste the token
```

---

## 4. Build and install the binaries

```bash
cd ~/local-ai-ops/agent-scheduler
./install.sh
```

This will:
- Build release binaries (`laio-orchestrator`, `laio-dispatcher`, `laio-admin`)
- Install them to `~/.local/bin/`
- Install the systemd user units
- Enable the timers

Verify the binaries work:

```bash
laio-admin config
# Should print: Config OK, then your endpoints and repos

laio-admin tasks list
# Should print: (no tasks) — DB just created
```

---

## 5. Start the idle-guard sampler

The dispatcher won't launch tasks unless the GPU has been idle for 15 minutes.
The sampler records GPU% every 30 seconds.

```bash
# Install the sampler
mkdir -p ~/.local/share/lemonade-idle ~/.config/systemd/user
cp ~/local-ai-ops/lemonade-idle/sampler.py ~/.local/share/lemonade-idle/
cp ~/local-ai-ops/lemonade-idle/lemonade-sampler.service ~/.config/systemd/user/

# Edit the service to point at your Lemonade URL if needed
# Default is http://localhost:13305 — change to http://localhost:8000 for Bosgame
sed -i 's|localhost:13305|localhost:8000|g' ~/.config/systemd/user/lemonade-sampler.service

systemctl --user daemon-reload
systemctl --user enable --now lemonade-sampler.service
systemctl --user status lemonade-sampler.service

# Watch it fill up (needs 10+ samples before dispatcher will run)
tail -f ~/.local/share/lemonade-idle/gpu.jsonl
```

---

## 6. Build the container image

```bash
cd ~/local-ai-ops/agent-scheduler/container
podman build -t local-ai-ops-runner:latest .
# This pulls Node 22 and installs gh + codex — takes a few minutes

# Verify
podman run --rm local-ai-ops-runner:latest --version 2>/dev/null || true
podman images | grep local-ai-ops-runner
```

---

## 7. Set up Codex skills

The container needs access to the gh-fix-issue and gh-review-pr skills.

```bash
# Skills should already be in ~/.codex/skills/ from the local-ai-ops repo
ls ~/.codex/skills/
# Should show: gh-fix-issue/  gh-review-pr/

# If not, copy them:
mkdir -p ~/.codex/skills
cp -r ~/local-ai-ops/skills/* ~/.codex/skills/
```

The skills directory is mounted into the container at runtime (via config `skills.dir`).
With `auto_update: true`, the dispatcher runs `git pull --ff-only` on it before each task.

---

## 8. Check the timers

```bash
systemctl --user list-timers | grep laio
# Should show:
#   laio-orchestrator.timer  Mon 2026-04-28 01:00:00 UTC  ...
#   laio-dispatcher.timer    in 27min                     ...

systemctl --user status laio-orchestrator.timer laio-dispatcher.timer
```

---

## 9. First manual run (optional but recommended)

Run the orchestrator now to populate the task queue without waiting for 1am:

```bash
LAIO_CONFIG=~/.config/laio/config.yaml laio-orchestrator
# Watch it scan your repos and queue tasks

laio-admin tasks list
# Should show pending tasks with priorities from the LLM triage
```

Run the dispatcher once to test a task end-to-end:

```bash
# Make sure GPU has been idle for ~15min first, or temporarily disable:
# idle_guard: { enabled: false }  in config.yaml

LAIO_CONFIG=~/.config/laio/config.yaml laio-dispatcher
# Watch it select an executor, launch the container, monitor heartbeat
```

---

## 10. Start the dashboard

```bash
laio-admin serve --bind 0.0.0.0 --port 8080
# Open http://bosgame:8080 in a browser

# Or as a background systemd service (optional):
cat > ~/.config/systemd/user/laio-dashboard.service <<'EOF'
[Unit]
Description=laio web dashboard
After=network-online.target

[Service]
ExecStart=%h/.local/bin/laio-admin serve --bind 0.0.0.0 --port 8080
Environment=LAIO_CONFIG=%h/.config/laio/config.yaml
Restart=on-failure

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now laio-dashboard.service
```

---

## Useful commands

```bash
# See what's in the queue
laio-admin tasks list
laio-admin tasks list --status failed

# Inspect a specific task (use first 8 chars of ID from list)
laio-admin tasks show <id>

# Retry a failed task
laio-admin tasks retry <id>

# Check active locks (if something seems stuck)
laio-admin locks list
laio-admin locks clear <endpoint>

# Performance metrics (after some tasks complete)
laio-admin metrics summary

# View logs
journalctl --user -u laio-orchestrator.service -f
journalctl --user -u laio-dispatcher.service -f
```

---

## Troubleshooting

**Dispatcher exits with "idle guard: BUSY"**
The sampler hasn't collected enough samples yet, or the GPU is genuinely busy.
Check `tail ~/.local/share/lemonade-idle/gpu.jsonl` — you need ≥10 samples in the
last 15 minutes all under 25%.

**"lock expired" errors in task table**
A previous container was killed before it could write a heartbeat.
Run `laio-admin locks list` to confirm no orphan locks, then `laio-admin tasks retry <id>`.

**Container image not found**
Run step 6 (build the image). Verify with `podman images | grep local-ai-ops-runner`.

**gh auth errors inside container**
Make sure `GH_TOKEN` is set in `~/.config/environment.d/laio.conf` and the systemd
unit loads it (user units pick up `environment.d` automatically on modern systemd).
Test: `systemctl --user show-environment | grep GH_TOKEN`.

**Lemonade not reachable**
Check Lemonade is running: `curl http://localhost:8000/api/v1/health`
Update the endpoint in `config.yaml` if the port differs.
