use anyhow::{Context, Result};
use clap::Parser;
use laio_common::{
    config::{Config, ExecutorConfig},
    db,
    idle_guard,
    lemonade::LemonadeClient,
    types::Task,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "laio-dispatcher", about = "Select executor, launch task container, collect metrics")]
struct Cli {
    #[arg(long, env = "LAIO_CONFIG")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Cli::parse();
    let cfg_path = args.config.unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path).context("loading config")?;

    let db_path = shellexpand::tilde(&cfg.database.path.to_string_lossy()).into_owned();
    let pool = db::open(Path::new(&db_path)).await?;

    // ── 1. Stale lock cleanup ────────────────────────────────────────────────
    let stale_ids = db::clean_stale_locks(
        &pool,
        cfg.timeouts.heartbeat_stale_after_s as i64,
        cfg.timeouts.lock_max_age_s as i64,
    ).await?;
    for id in &stale_ids {
        warn!("stale lock for task {} — marking failed", &id[..id.len().min(8)]);
        let _ = db::complete_task(
            &pool, id, 1,
            None, None, None, None, None, None,
            Some("lock expired (stale heartbeat or max age)"),
        ).await;
    }

    // ── 2. Idle guard ────────────────────────────────────────────────────────
    let idle = idle_guard::check(&cfg.idle_guard)?;
    if !idle.is_idle() {
        info!("{idle} — exiting (timer will retry)");
        return Ok(());
    }
    info!("{idle}");

    // ── 3. Find available executor ───────────────────────────────────────────
    let Some((executor, lemon)) = find_executor(&cfg, &pool).await? else {
        info!("no executor available — exiting");
        return Ok(());
    };
    info!("selected executor {} ({})", executor.name, executor.endpoint);

    // ── 4. Pick next pending task ────────────────────────────────────────────
    let Some(task) = db::next_pending_task(&pool).await? else {
        info!("no pending tasks — exiting");
        return Ok(());
    };
    info!("dispatching task {} ({}) — {}", &task.id[..8], task.r#type, task.target_url);

    // ── 5. Acquire endpoint lock ─────────────────────────────────────────────
    if !db::acquire_lock(&pool, &executor.endpoint, &task.id).await? {
        warn!("lock race on {} — another dispatcher won, exiting", executor.endpoint);
        return Ok(());
    }

    // ── 6. Mark task active ──────────────────────────────────────────────────
    let timeout_s = cfg.timeouts.for_task(&task.r#type, &executor.name);
    db::mark_task_active(&pool, &task.id, &executor.endpoint, &executor.model, timeout_s as i64).await?;

    // ── 7. Update skills ─────────────────────────────────────────────────────
    pull_skills(&cfg.skills.dir).await;

    // ── 8. Run container ─────────────────────────────────────────────────────
    let task_dir = PathBuf::from(
        shellexpand::tilde(&cfg.database.task_dir.to_string_lossy()).into_owned()
    ).join(&task.id);
    tokio::fs::create_dir_all(&task_dir).await?;

    let gh_token = std::env::var(&cfg.container.gh_token_env)
        .with_context(|| format!("${} not set", cfg.container.gh_token_env))?;

    let exit_code = run_container(
        &cfg, &pool, &task, &executor, &task_dir, &gh_token, timeout_s,
    ).await;

    // ── 9. Collect metrics and finalize task ─────────────────────────────────
    let inference = lemon.inference_stats().await.ok();
    let (pr_url, summary) = read_result_md(&task_dir).await;

    let error_msg = if exit_code == 0 { None } else { Some("container exited with non-zero code") };
    db::complete_task(
        &pool, &task.id, exit_code,
        inference.as_ref().map(|s| s.input_tokens),
        inference.as_ref().map(|s| s.output_tokens),
        inference.as_ref().map(|s| s.tokens_per_second),
        inference.as_ref().map(|s| s.time_to_first_token),
        pr_url.as_deref(),
        summary.as_deref(),
        error_msg,
    ).await?;
    db::release_lock(&pool, &executor.endpoint).await?;

    if exit_code == 0 {
        info!("task {} done", &task.id[..8]);
    } else {
        warn!("task {} failed (exit {})", &task.id[..8], exit_code);
    }
    Ok(())
}

// ── Executor selection ─────────────────────────────────────────────────────────

async fn find_executor(
    cfg: &Config,
    pool: &sqlx::sqlite::SqlitePool,
) -> Result<Option<(ExecutorConfig, LemonadeClient)>> {
    let locks = db::list_locks(pool).await?;
    let locked: HashSet<&str> = locks.iter().map(|l| l.endpoint.as_str()).collect();

    for executor in &cfg.executors {
        if locked.contains(executor.endpoint.as_str()) {
            info!("executor {} locked — skipping", executor.name);
            continue;
        }
        let client = LemonadeClient::new(&executor.endpoint);
        if !client.is_idle(cfg.idle_guard.threshold_pct).await {
            info!("executor {} GPU busy — skipping", executor.name);
            continue;
        }
        return Ok(Some((executor.clone(), client)));
    }
    Ok(None)
}

// ── Container execution ────────────────────────────────────────────────────────

async fn run_container(
    cfg: &Config,
    pool: &sqlx::sqlite::SqlitePool,
    task: &Task,
    executor: &ExecutorConfig,
    task_dir: &Path,
    gh_token: &str,
    timeout_s: u64,
) -> i64 {
    let heartbeat_path = task_dir.join("heartbeat");
    let pool2 = pool.clone();
    let endpoint2 = executor.endpoint.clone();
    let hb_path2 = heartbeat_path.clone();
    let hb_interval = cfg.timeouts.heartbeat_interval_s;

    let hb_task = tokio::spawn(async move {
        let mut last = String::new();
        loop {
            sleep(Duration::from_secs(hb_interval / 2)).await;
            if let Ok(content) = tokio::fs::read_to_string(&hb_path2).await {
                let ts = content.trim().to_string();
                if ts != last {
                    last = ts;
                    let _ = db::update_heartbeat(&pool2, &endpoint2).await;
                }
            }
        }
    });

    let mut cmd = tokio::process::Command::new(&cfg.container.runtime);
    cmd.arg("run").arg("--rm")
       .arg("--timeout").arg(timeout_s.to_string())
       .arg("-v").arg(format!("{}:/task:z", task_dir.display()))
       .arg("-e").arg(format!("LEMONADE_URL={}", executor.endpoint))
       .arg("-e").arg(format!("GH_TOKEN={gh_token}"))
       .arg("-e").arg(format!("TASK_ID={}", task.id))
       .arg("-e").arg(format!("TASK_TYPE={}", task.r#type))
       .arg("-e").arg(format!("TARGET_URL={}", task.target_url))
       .arg("-e").arg(format!("HEARTBEAT_INTERVAL_S={hb_interval}"))
       .arg("-e").arg("HEARTBEAT_FILE=/task/heartbeat")
       .arg(&cfg.container.image);

    info!("running: {} run ... {}", cfg.container.runtime, cfg.container.image);
    let exit_code = match cmd.status().await {
        Ok(s)  => s.code().unwrap_or(1) as i64,
        Err(e) => { error!("container launch failed: {e}"); 1 }
    };

    hb_task.abort();
    exit_code
}

// ── Helpers ────────────────────────────────────────────────────────────────────

async fn pull_skills(skills_dir: &Path) {
    let dir = shellexpand::tilde(&skills_dir.to_string_lossy()).into_owned();
    match tokio::process::Command::new("git")
        .args(["-C", &dir, "pull", "--ff-only"])
        .output().await
    {
        Ok(out) if out.status.success() => info!("skills updated from git"),
        Ok(out) => warn!("git pull skills: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(e)  => warn!("git pull skills failed: {e}"),
    }
}

/// Reads `result.md` written by the container.
/// Expected format (key=value lines):
///   PR_URL=https://github.com/...
///   SUMMARY=one-line summary
async fn read_result_md(task_dir: &Path) -> (Option<String>, Option<String>) {
    let Ok(content) = tokio::fs::read_to_string(task_dir.join("result.md")).await else {
        return (None, None);
    };
    let mut pr_url = None;
    let mut summary_parts = Vec::new();
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("PR_URL=") {
            pr_url = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("SUMMARY=") {
            summary_parts.push(v.trim().to_string());
        }
    }
    let summary = if summary_parts.is_empty() { None } else { Some(summary_parts.join(" ")) };
    (pr_url, summary)
}
