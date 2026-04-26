# local-ai-ops

Tools for running AI agents on a local workstation without getting in your own way.

## Contents

### [`skills/`](skills/)

Codex CLI skills for autonomous GitHub workflows. Drop these into `~/.codex/skills/` (or set `CODEX_HOME`).

| Skill | Trigger | What it does |
|---|---|---|
| [`gh-fix-issue`](skills/gh-fix-issue/) | `Fix this GitHub issue: <URL>` | Reads the issue, clones the repo, studies code style and contribution guidelines, implements a fix, opens a PR, then iterates through CI and review until merge-ready |
| [`gh-review-pr`](skills/gh-review-pr/) | `Review this PR: <URL or number>` | Fetches diff and context, classifies findings by severity (🔴 blocker / 🟠 architecture / 🟡 quality / ✅ good), posts a structured review, and monitors for follow-up |

#### Installing skills

```bash
# Clone and symlink (picks up updates automatically)
git clone https://github.com/vpavlin/local-ai-ops.git
ln -s $(pwd)/local-ai-ops/skills/gh-fix-issue ~/.codex/skills/
ln -s $(pwd)/local-ai-ops/skills/gh-review-pr ~/.codex/skills/
```

### [`lemonade-idle/`](lemonade-idle/)

Idle-aware scheduling for [Lemonade Server](https://lemonade-server.ai/). Samples GPU utilisation on a sliding window and gates agent launches so they never compete with active inference.

- **`sampler.py`** — polls `/api/v1/system-stats` every 30s, maintains a pruned JSONL log
- **`idle-guard.py`** — exits 0 (idle) or 1 (busy) based on peak GPU across the window
- **`systemd/`** — user-level service and timer templates

See [`lemonade-idle/README.md`](lemonade-idle/README.md) for setup instructions.

## Requirements

- [Codex CLI](https://github.com/openai/codex) with `gh` CLI authenticated
- Python 3.8+
- Lemonade Server (for idle guard)
- systemd (for scheduling)
