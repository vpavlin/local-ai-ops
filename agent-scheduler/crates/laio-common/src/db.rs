use anyhow::Result;
use chrono::Utc;
use sqlx::{sqlite::SqlitePool, Row};
use std::path::Path;

use crate::types::{EndpointLock, RepoState, Task};

pub async fn open(db_path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = format!("sqlite://{}?mode=rwc", db_path.display());
    let pool = SqlitePool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

pub async fn insert_task(pool: &SqlitePool, t: &Task) -> Result<()> {
    sqlx::query(
        "INSERT INTO tasks (
            id, repo, type, target_url, priority, status,
            analysis_path, complexity, affected_files, orchestrator_model,
            head_sha, last_analysis_at, retry_count, created_at
        ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
    )
    .bind(&t.id).bind(&t.repo).bind(&t.r#type).bind(&t.target_url)
    .bind(t.priority).bind(&t.status)
    .bind(&t.analysis_path).bind(&t.complexity).bind(&t.affected_files)
    .bind(&t.orchestrator_model).bind(&t.head_sha).bind(t.last_analysis_at)
    .bind(t.retry_count).bind(t.created_at)
    .execute(pool).await?;
    Ok(())
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool).await?;
    Ok(row)
}

pub async fn get_task_by_url(pool: &SqlitePool, url: &str) -> Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>("SELECT * FROM tasks WHERE target_url = ?")
        .bind(url)
        .fetch_optional(pool).await?;
    Ok(row)
}

pub async fn list_tasks(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<Task>> {
    let rows = match status {
        Some(s) => sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks WHERE status = ? ORDER BY priority ASC, created_at ASC")
            .bind(s)
            .fetch_all(pool).await?,
        None => sqlx::query_as::<_, Task>(
            "SELECT * FROM tasks ORDER BY priority ASC, created_at ASC")
            .fetch_all(pool).await?,
    };
    Ok(rows)
}

pub async fn next_pending_task(pool: &SqlitePool) -> Result<Option<Task>> {
    let row = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE status = 'pending'
         ORDER BY priority ASC, created_at ASC LIMIT 1")
        .fetch_optional(pool).await?;
    Ok(row)
}

pub async fn update_task_status(pool: &SqlitePool, id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE tasks SET status = ? WHERE id = ?")
        .bind(status).bind(id)
        .execute(pool).await?;
    Ok(())
}

pub async fn mark_task_active(
    pool: &SqlitePool, id: &str, endpoint: &str, model: &str, timeout_s: i64,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "UPDATE tasks SET status='active', started_at=?, endpoint=?, model=?,
         timeout_configured=? WHERE id=?"
    )
    .bind(now).bind(endpoint).bind(model).bind(timeout_s).bind(id)
    .execute(pool).await?;
    Ok(())
}

pub async fn complete_task(pool: &SqlitePool, id: &str, exit_code: i64,
    tokens_in: Option<i64>, tokens_out: Option<i64>,
    decode_tps: Option<f64>, ttft: Option<f64>,
    pr_url: Option<&str>, summary: Option<&str>, error: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "UPDATE tasks SET
            status = CASE WHEN ? = 0 THEN 'done' ELSE 'failed' END,
            completed_at = ?,
            wall_seconds = ? - started_at,
            exit_code = ?,
            tokens_in = ?, tokens_out = ?,
            decode_tps = ?, time_to_first_token = ?,
            pr_url = ?, result_summary = ?, error = ?
         WHERE id = ?"
    )
    .bind(exit_code).bind(now).bind(now).bind(exit_code)
    .bind(tokens_in).bind(tokens_out).bind(decode_tps).bind(ttft)
    .bind(pr_url).bind(summary).bind(error).bind(id)
    .execute(pool).await?;
    Ok(())
}

pub async fn requeue_task(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query(
        "UPDATE tasks SET status='pending', started_at=NULL, completed_at=NULL,
         wall_seconds=NULL, exit_code=NULL, endpoint=NULL, model=NULL,
         error=NULL, retry_count=retry_count+1 WHERE id=?"
    )
    .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn update_analysis(
    pool: &SqlitePool, id: &str,
    path: &str, complexity: &str, affected_files: &str,
    model: &str, head_sha: Option<&str>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query(
        "UPDATE tasks SET analysis_path=?, complexity=?, affected_files=?,
         orchestrator_model=?, head_sha=?, last_analysis_at=? WHERE id=?"
    )
    .bind(path).bind(complexity).bind(affected_files)
    .bind(model).bind(head_sha).bind(now).bind(id)
    .execute(pool).await?;
    Ok(())
}

pub async fn list_tasks_by_repo_status(
    pool: &SqlitePool, repo: &str, status: &str,
) -> Result<Vec<Task>> {
    let rows = sqlx::query_as::<_, Task>(
        "SELECT * FROM tasks WHERE repo = ? AND status = ? ORDER BY created_at ASC"
    )
    .bind(repo).bind(status)
    .fetch_all(pool).await?;
    Ok(rows)
}

pub async fn update_task_priority(pool: &SqlitePool, id: &str, priority: i64) -> Result<()> {
    sqlx::query("UPDATE tasks SET priority = ? WHERE id = ?")
        .bind(priority).bind(id)
        .execute(pool).await?;
    Ok(())
}

// ── Endpoint locks ─────────────────────────────────────────────────────────────

pub async fn acquire_lock(pool: &SqlitePool, endpoint: &str, task_id: &str) -> Result<bool> {
    let now = Utc::now().timestamp();
    let res = sqlx::query(
        "INSERT OR IGNORE INTO endpoint_locks (endpoint, task_id, locked_at, heartbeat_at)
         VALUES (?, ?, ?, ?)"
    )
    .bind(endpoint).bind(task_id).bind(now).bind(now)
    .execute(pool).await?;
    Ok(res.rows_affected() == 1)
}

pub async fn release_lock(pool: &SqlitePool, endpoint: &str) -> Result<()> {
    sqlx::query("DELETE FROM endpoint_locks WHERE endpoint = ?")
        .bind(endpoint).execute(pool).await?;
    Ok(())
}

pub async fn update_heartbeat(pool: &SqlitePool, endpoint: &str) -> Result<()> {
    let now = Utc::now().timestamp();
    sqlx::query("UPDATE endpoint_locks SET heartbeat_at = ? WHERE endpoint = ?")
        .bind(now).bind(endpoint).execute(pool).await?;
    Ok(())
}

pub async fn list_locks(pool: &SqlitePool) -> Result<Vec<EndpointLock>> {
    let rows = sqlx::query_as::<_, EndpointLock>("SELECT * FROM endpoint_locks")
        .fetch_all(pool).await?;
    Ok(rows)
}

/// Returns the task IDs whose locks were expired and deleted.
pub async fn clean_stale_locks(
    pool: &SqlitePool, stale_after_s: i64, max_age_s: i64,
) -> Result<Vec<String>> {
    let now = Utc::now().timestamp();
    let stale_cutoff   = now - stale_after_s;
    let max_age_cutoff = now - max_age_s;

    let stale: Vec<String> = sqlx::query(
        "SELECT task_id FROM endpoint_locks
         WHERE heartbeat_at < ? OR locked_at < ?"
    )
    .bind(stale_cutoff).bind(max_age_cutoff)
    .fetch_all(pool).await?
    .iter()
    .map(|r| r.get::<String, _>("task_id"))
    .collect();

    if !stale.is_empty() {
        sqlx::query(
            "DELETE FROM endpoint_locks WHERE heartbeat_at < ? OR locked_at < ?"
        )
        .bind(stale_cutoff).bind(max_age_cutoff)
        .execute(pool).await?;
    }
    Ok(stale)
}

// ── Repo state ──────────────────────────────────────────────────────────────

pub async fn upsert_repo_state(pool: &SqlitePool, state: &RepoState) -> Result<()> {
    sqlx::query(
        "INSERT INTO repo_state (url, last_scanned_at, open_issues, open_prs)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET
           last_scanned_at=excluded.last_scanned_at,
           open_issues=excluded.open_issues,
           open_prs=excluded.open_prs"
    )
    .bind(&state.url).bind(state.last_scanned_at)
    .bind(state.open_issues).bind(state.open_prs)
    .execute(pool).await?;
    Ok(())
}

pub async fn list_repo_states(pool: &SqlitePool) -> Result<Vec<RepoState>> {
    let rows = sqlx::query_as::<_, RepoState>("SELECT * FROM repo_state")
        .fetch_all(pool).await?;
    Ok(rows)
}

// ── Metrics ─────────────────────────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
pub struct MetricsSummary {
    pub r#type:            String,
    pub endpoint:          Option<String>,
    pub runs:              i64,
    pub avg_wall_s:        Option<f64>,
    pub max_wall_s:        Option<i64>,
    pub avg_decode_tps:    Option<f64>,
    pub timeout_headroom:  Option<f64>,
    pub container_timeouts: i64,
}

pub async fn metrics_summary(pool: &SqlitePool) -> Result<Vec<MetricsSummary>> {
    let rows = sqlx::query_as::<_, MetricsSummary>(
        "SELECT
            type,
            endpoint,
            COUNT(*) AS runs,
            AVG(CAST(wall_seconds AS REAL)) AS avg_wall_s,
            MAX(wall_seconds) AS max_wall_s,
            AVG(decode_tps) AS avg_decode_tps,
            CAST(AVG(timeout_configured) AS REAL) / NULLIF(MAX(wall_seconds), 0) AS timeout_headroom,
            COUNT(CASE WHEN exit_code = 125 THEN 1 END) AS container_timeouts
         FROM tasks
         WHERE status IN ('done','failed')
         GROUP BY type, endpoint
         ORDER BY type, endpoint"
    )
    .fetch_all(pool).await?;
    Ok(rows)
}
