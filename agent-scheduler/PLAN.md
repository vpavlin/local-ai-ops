# agent-scheduler — Design Plan

Autonomous agent scheduling system that uses a small MoE model to triage GitHub work,
then dispatches tasks to larger models via containerized execution — gated by GPU idle
detection so it never competes with interactive use.

---

## Architecture overview

```
┌─────────────────────────────── Bosgame (Strix Halo) ───────────────────────────────┐
│                                                                                     │
│  laio-orchestrator (nightly, 1am)                                                  │
│    calls DeepSeek-Coder-V2-Lite on K11                                             │
│    → analyses repos → writes task specs → inserts into SQLite                      │
│                                                                                     │
│  laio-dispatcher (every 30min, idle-gated)                                         │
│    cleans stale locks → selects idle executor → acquires lock                      │
│    → podman run local-ai-ops-runner → waits → collects metrics → releases lock     │
│                                                                                     │
│  SQLite (tasks.db) ──────────────────────────────────────────────────────────────  │
│                                                                                     │
└─────────────────────────────────────────────────────────────────────────────────────┘
         │ Lemonade API                    │ Lemonade API
         ▼                                ▼
┌── K11 (.125:3000) ──┐        ┌── Bosgame (.203:8000) ──┐
│  DeepSeek-Coder-V2  │        │  Qwen3.6-35B-A3B        │
│  (orchestrator)     │        │  (executor, primary)     │
│  Qwen3.6-35B-A3B    │        └──────────────────────────┘
│  (executor, backup) │
└─────────────────────┘
```

---

## Repository structure

```
agent-scheduler/
├── PLAN.md                     (this file)
├── Cargo.toml                  (workspace)
├── config.yaml.example
├── .gitignore                  (ignores config.yaml with real endpoints/tokens)
├── crates/
│   ├── laio-common/            (shared types, DB client, Lemonade client, config)
│   ├── laio-orchestrator/      (binary: triage repos, generate task specs)
│   ├── laio-dispatcher/        (binary: select executor, launch container, collect metrics)
│   └── laio-admin/             (binary: inspect tasks, query metrics, retry)
└── container/
    ├── Dockerfile
    └── run-task.sh
```

---

## Rust workspace

### Crates and key dependencies

```toml
# crates/laio-common
sqlx = { features = ["sqlite", "runtime-tokio"] }
serde = { features = ["derive"] }
serde_yaml = "*"
reqwest = { features = ["json"] }
tokio = { features = ["full"] }
uuid = { features = ["v4"] }
tracing + tracing-subscriber

# crates/laio-orchestrator (bin)
laio-common
tokio-process   # gh CLI subprocess

# crates/laio-dispatcher (bin)
laio-common
tokio-process   # podman subprocess

# crates/laio-admin (bin)
laio-common
clap = { features = ["derive"] }
comfy-table     # terminal output
```

---

## SQLite schema

```sql
CREATE TABLE tasks (
    id              TEXT PRIMARY KEY,      -- UUIDv4
    repo            TEXT NOT NULL,
    type            TEXT NOT NULL,         -- 'fix-issue' | 'review-pr'
    target_url      TEXT NOT NULL UNIQUE,  -- dedup key: one row per issue/PR
    priority        INTEGER DEFAULT 5,     -- lower = higher priority
    status          TEXT DEFAULT 'pending',-- pending|active|done|failed|skipped

    -- Orchestrator output
    analysis_path   TEXT,                  -- /var/lib/laio/tasks/<id>/analysis.md
    complexity      TEXT,                  -- low|medium|high
    affected_files  TEXT,                  -- JSON array
    orchestrator_model TEXT,

    -- Timing
    created_at      INTEGER NOT NULL,
    started_at      INTEGER,
    first_heartbeat_at INTEGER,            -- seconds from started_at to first heartbeat
    completed_at    INTEGER,
    wall_seconds    INTEGER,               -- completed_at - started_at

    -- Execution
    endpoint        TEXT,                  -- executor endpoint used
    model           TEXT,                  -- executor model used
    timeout_configured INTEGER,            -- timeout_s at time of run (for comparison)
    exit_code       INTEGER,

    -- Lemonade metrics (from /api/v1/stats after run)
    tokens_in       INTEGER,
    tokens_out      INTEGER,
    decode_tps      REAL,
    time_to_first_token REAL,

    -- Results
    pr_url          TEXT,                  -- created PR URL if applicable
    result_summary  TEXT,                  -- from /task/result.md
    error           TEXT
);

CREATE TABLE endpoint_locks (
    endpoint        TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    locked_at       INTEGER NOT NULL,
    heartbeat_at    INTEGER NOT NULL       -- updated by container every heartbeat_interval_s
);

CREATE TABLE repo_state (
    url             TEXT PRIMARY KEY,
    last_scanned_at INTEGER,
    open_issues     INTEGER,
    open_prs        INTEGER
);
```

---

## Config schema

```yaml
# config.yaml.example — copy to config.yaml and fill in endpoints/tokens
# config.yaml is gitignored

database:
  path: /var/lib/laio/tasks.db
  task_dir: /var/lib/laio/tasks     # analysis.md + result.md per task

orchestrator:
  endpoint: http://localhost:13305   # replace with K11 Lemonade URL
  model: deepseek-coder-v2-lite
  max_tasks_per_scan: 20            # cap per orchestrator run

executors:
  - name: bosgame-35b
    endpoint: http://localhost:8000  # replace with Bosgame Lemonade URL
    model: Qwen3.6-35B-A3B
    max_concurrent: 1
  - name: gmktec-35b
    endpoint: http://localhost:13305 # replace with K11 Lemonade URL
    model: Qwen3.6-35B-A3B
    max_concurrent: 1

repos:
  - url: https://github.com/example/repo
    actions: [fix-issue, review-pr]
    priority: 1

timeouts:
  default_s: 14400                  # 4h default
  per_type:
    fix-issue: 14400                # 4h
    review-pr:  3600                # 1h
  per_endpoint:
    gmktec-35b: 21600               # 6h — slower iGPU inference
  heartbeat_interval_s: 120         # container updates heartbeat every 2min
  heartbeat_stale_after_s: 600      # lock is dead if no heartbeat for 10min
  lock_max_age_s: 86400             # absolute ceiling regardless of heartbeat

idle_guard:
  enabled: true
  stats_file: ~/.local/share/lemonade-idle/gpu.jsonl
  threshold_pct: 25.0
  min_samples: 10
  window_s: 900

container:
  image: local-ai-ops-runner:latest
  runtime: podman
  gh_token_env: GH_TOKEN            # env var name that holds the token

# Prometheus (uncomment when Lemonade exposes metrics)
# prometheus:
#   endpoint: http://localhost:9090
#   model_label: model_name         # label that identifies the model
```

---

## Component design

### laio-orchestrator

1. Load config, open DB
2. For each repo in config:
   a. `gh issue list --repo <owner>/<repo> --state open --json number,title,body,labels,createdAt`
   b. `gh pr list --repo <owner>/<repo> --state open --json number,title,body,author,createdAt,additions,deletions`
   c. For each item not already in `tasks` with status pending/active:
      - Call orchestrator model (DeepSeek-Coder-V2-Lite) with prompt:
        ```
        Analyse this GitHub issue/PR. Return JSON:
        {
          "type": "fix-issue"|"review-pr",
          "complexity": "low"|"medium"|"high",
          "priority": 1-10,
          "affected_files": ["..."],
          "summary": "one sentence",
          "approach_hint": "what to look at first"
        }
        ```
      - Write `/var/lib/laio/tasks/<id>/analysis.md` (full context: issue body + JSON)
      - Insert task row (status=pending)
3. Update `repo_state`
4. Exit

**Idempotency**: skip any `target_url` already present in `tasks` regardless of status.
Re-queue only explicitly via `laio-admin retry <id>`.

---

### laio-dispatcher

1. Load config, open DB
2. **Stale lock cleanup**:
   ```sql
   DELETE FROM endpoint_locks
   WHERE heartbeat_at < (strftime('%s','now') - :stale_s)
      OR locked_at    < (strftime('%s','now') - :max_age_s)
   ```
   For each deleted lock: update corresponding task to `status='failed', error='lock expired'`
3. **Idle check** (lemonade-idle guard): if not idle, exit 0 (timer will retry)
4. **Find available executor**:
   - For each executor in config order:
     - Check `endpoint_locks` — skip if locked
     - Check Lemonade `system-stats` GPU% — skip if above threshold
     - If clear: select this executor
   - If none available: exit 0
5. **Pick task**:
   ```sql
   SELECT * FROM tasks
   WHERE status = 'pending'
   ORDER BY priority ASC, created_at ASC
   LIMIT 1
   ```
6. **Acquire lock**: INSERT into `endpoint_locks` (fail = race condition, retry next run)
7. **Update task**: `status='active', started_at=now(), endpoint=..., model=...`
8. **Launch container**:
   ```bash
   podman run --rm \
     --timeout <configured_timeout_s> \
     -v /var/lib/laio/tasks/<id>:/task \
     -e LEMONADE_URL=<endpoint> \
     -e GH_TOKEN=<token> \
     -e TASK_ID=<id> \
     -e HEARTBEAT_INTERVAL_S=<n> \
     -e HEARTBEAT_FILE=/task/heartbeat \
     local-ai-ops-runner:latest
   ```
9. **Wait** — stream heartbeat file updates to keep `endpoint_locks.heartbeat_at` current
10. **On container exit**:
    - Fetch Lemonade `/api/v1/stats` (decode_tps, tokens_in/out, time_to_first_token)
    - Read `/task/result.md` for summary and pr_url
    - Update task row with all metrics + exit_code + wall_seconds + status
    - DELETE from `endpoint_locks`

---

### laio-admin

```
laio-admin tasks list [--status pending|active|done|failed] [--repo <url>]
laio-admin tasks show <id>
laio-admin tasks retry <id>
laio-admin metrics summary           # P50/P95/P99 per type+endpoint, timeout headroom
laio-admin locks list                # show active locks + heartbeat age
laio-admin locks clear <endpoint>    # manual lock release
laio-admin config validate           # check config.yaml
```

**Useful metrics query**:
```sql
SELECT type, endpoint,
  COUNT(*) AS runs,
  AVG(wall_seconds) AS avg_s,
  MAX(wall_seconds) AS max_s,
  AVG(decode_tps)   AS avg_tps,
  CAST(timeout_configured AS REAL) / NULLIF(MAX(wall_seconds), 0) AS timeout_headroom,
  COUNT(*) FILTER (WHERE exit_code = 125) AS container_timeouts
FROM tasks WHERE status IN ('done','failed')
GROUP BY type, endpoint ORDER BY type, endpoint;
```

---

### Container (local-ai-ops-runner)

**Dockerfile** (multi-stage):
- Stage 1: install codex CLI, gh CLI, git, rust toolchain, python3
- Stage 2: copy binaries into debian-slim

**run-task.sh**:
1. Read `/task/analysis.md` — contains issue context + orchestrator JSON
2. Start heartbeat loop in background (writes timestamp to `/task/heartbeat` every `$HEARTBEAT_INTERVAL_S`)
3. Extract task type from analysis JSON
4. Invoke codex with appropriate skill:
   - `fix-issue` → `codex --skill gh-fix-issue "Fix: <target_url> — see /task/analysis.md"`
   - `review-pr` → `codex --skill gh-review-pr "Review: <target_url> — see /task/analysis.md"`
5. Capture output, extract PR URL if created
6. Write `/task/result.md`
7. Exit with codex exit code

---

## Systemd units

```
laio-orchestrator.timer   OnCalendar=*-*-* 01:00:00 (nightly at 1am)
laio-orchestrator.service Type=oneshot

laio-dispatcher.timer     OnBootSec=10min, OnUnitActiveSec=30min, RandomizedDelaySec=3min
laio-dispatcher.service   Type=oneshot
```

Both services read `LAIO_CONFIG` env var for config path (defaults to `~/.config/laio/config.yaml`).

---

## Build phases

| Phase | Scope | Deliverable |
|---|---|---|
| 1 | laio-common: config, DB schema + migrations, Lemonade client, idle guard | shared library |
| 2 | laio-orchestrator: gh subprocess, LLM call, task insertion | working orchestrator binary |
| 3 | laio-dispatcher: lock management, podman launch, metric collection | working dispatcher binary |
| 4 | Container: Dockerfile + run-task.sh + heartbeat | buildable image |
| 5 | laio-admin: task inspection, metrics queries, retry | admin CLI |
| 6 | Prometheus: swap system-stats idle check for Prometheus queries | when Lemonade metrics land |
| 7 | Systemd units + install script | deployable |

---

## Decisions

**Failed task re-analysis**: Orchestrator re-analyses `failed` tasks on each nightly run,
provided they haven't been retried more than `max_retries` (config, default 3). Extra
context (e.g. new comments on the issue since failure) is appended to the existing
analysis file rather than replacing it — preserving history. Manual `laio-admin retry`
resets the retry counter and forces immediate re-analysis.

**PR re-queue on new commits**: Orchestrator stores `head_sha` at review time. On each
scan, if a `done` PR has a new `head_sha`, it is automatically re-queued at priority+2
(slightly lower than fresh work) with a note in the analysis: "Previously reviewed at
`<old_sha>` — re-queue triggered by new commits."

**Skill updates before each run**: Dispatcher runs `git -C <skills_dir> pull --ff-only`
before launching the container. Skills dir is configurable; defaults to
`~/.codex/skills`. Failure to pull is logged but does not block the task.

**Notifications**: Out of scope for now — tracked as a future Phase 8.

---

## Dashboard

A read-only web dashboard served by `laio-admin serve`. Keeps everything in Rust, no
extra runtime dependencies.

### Quick start (while dashboard is built)

[Datasette](https://datasette.io/) gives an instant zero-effort UI:
```bash
pip install datasette && datasette /var/lib/laio/tasks.db
```
Useful for ad-hoc queries during development. Not a long-term dependency.

### laio-admin serve

```
laio-admin serve [--port 8080] [--bind 0.0.0.0]
```

Small `axum` HTTP server serving a single-page dashboard. Refreshes every 30s via
`<meta http-equiv="refresh">` (no JS framework needed).

**Stack**: `axum` + `askama` (compile-time HTML templates) + `sqlx` queries.

**Pages / panels**:

| Panel | Data source |
|---|---|
| Status summary | COUNT(*) GROUP BY status |
| Active runs | endpoint_locks JOIN tasks — shows endpoint, task, lock age, last heartbeat age |
| Recent completions (last 24h) | tasks WHERE completed_at > now-86400, sorted by completed_at DESC |
| Failure log | tasks WHERE status='failed', error, exit_code |
| Metrics table | P50/P95/P99 wall_seconds + avg decode_tps per type × endpoint |
| Repo health | repo_state: last_scanned_at, open_issues, open_prs |
| Timeout headroom | configured_timeout / max(wall_seconds) per type × endpoint — highlights when a timeout is dangerously close to the worst observed run |

**Schema additions for dashboard**:
```sql
-- track retries
ALTER TABLE tasks ADD COLUMN retry_count INTEGER DEFAULT 0;
ALTER TABLE tasks ADD COLUMN head_sha TEXT;           -- for PR re-queue detection
ALTER TABLE tasks ADD COLUMN last_analysis_at INTEGER; -- when analysis.md was last written
```

### Build phases (updated)

| Phase | Scope | Deliverable |
|---|---|---|
| 1 | laio-common: config, DB schema + migrations, Lemonade client, idle guard | shared library |
| 2 | laio-orchestrator: gh subprocess, LLM call, task insertion, retry + re-queue logic | working orchestrator binary |
| 3 | laio-dispatcher: lock management, skill pull, podman launch, metric collection | working dispatcher binary |
| 4 | Container: Dockerfile + run-task.sh + heartbeat writer | buildable image |
| 5 | laio-admin: task CLI (list, show, retry, locks) + metrics queries | admin CLI |
| 6 | laio-admin serve: axum dashboard | web dashboard |
| 7 | Systemd units + install script | deployable |
| 8 | Prometheus: swap system-stats idle check for Prometheus queries | when Lemonade metrics land |
| 9 | Notifications (ntfy.sh or similar) | future |
