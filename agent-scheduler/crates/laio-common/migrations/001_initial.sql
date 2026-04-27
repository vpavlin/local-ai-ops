CREATE TABLE IF NOT EXISTS tasks (
    id                  TEXT PRIMARY KEY,
    repo                TEXT NOT NULL,
    type                TEXT NOT NULL,         -- fix-issue | review-pr
    target_url          TEXT NOT NULL UNIQUE,
    priority            INTEGER NOT NULL DEFAULT 5,
    status              TEXT NOT NULL DEFAULT 'pending',
    -- orchestrator output
    analysis_path       TEXT,
    complexity          TEXT,                  -- low | medium | high
    affected_files      TEXT,                  -- JSON array
    orchestrator_model  TEXT,
    head_sha            TEXT,                  -- for PR re-queue detection
    last_analysis_at    INTEGER,
    retry_count         INTEGER NOT NULL DEFAULT 0,
    -- timing
    created_at          INTEGER NOT NULL,
    started_at          INTEGER,
    first_heartbeat_at  INTEGER,
    completed_at        INTEGER,
    wall_seconds        INTEGER,
    -- execution
    endpoint            TEXT,
    model               TEXT,
    timeout_configured  INTEGER,
    exit_code           INTEGER,
    -- lemonade metrics
    tokens_in           INTEGER,
    tokens_out          INTEGER,
    decode_tps          REAL,
    time_to_first_token REAL,
    -- results
    pr_url              TEXT,
    result_summary      TEXT,
    error               TEXT
);

CREATE TABLE IF NOT EXISTS endpoint_locks (
    endpoint        TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    locked_at       INTEGER NOT NULL,
    heartbeat_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS repo_state (
    url             TEXT PRIMARY KEY,
    last_scanned_at INTEGER,
    open_issues     INTEGER,
    open_prs        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_tasks_status   ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority, created_at);
CREATE INDEX IF NOT EXISTS idx_tasks_repo     ON tasks(repo);
