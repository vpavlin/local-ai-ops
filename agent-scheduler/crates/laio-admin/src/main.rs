mod dashboard;

use anyhow::{Context, Result};
use axum::{extract::State, response::Html, routing::get, Router};
use clap::{Parser, Subcommand};
use dashboard::DashboardData;
use laio_common::{config::Config, db};
use sqlx::sqlite::SqlitePool;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "laio-admin", about = "Inspect tasks, query metrics, manage locks")]
struct Cli {
    #[arg(long, env = "LAIO_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Task management
    Tasks {
        #[command(subcommand)]
        action: TasksCmd,
    },
    /// Metrics summary
    Metrics {
        #[command(subcommand)]
        action: MetricsCmd,
    },
    /// Endpoint lock management
    Locks {
        #[command(subcommand)]
        action: LocksCmd,
    },
    /// Validate config.yaml
    Config,
    /// Serve the web dashboard
    Serve {
        #[arg(long, default_value = "8080")]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
}

#[derive(Subcommand)]
enum TasksCmd {
    /// List tasks
    List {
        #[arg(long, help = "Filter by status: pending|active|done|failed|skipped")]
        status: Option<String>,
        #[arg(long, help = "Filter by repo URL")]
        repo: Option<String>,
    },
    /// Show a single task
    Show {
        id: String,
    },
    /// Retry a failed task (resets retry counter)
    Retry {
        id: String,
    },
    /// Mark a task as skipped
    Skip {
        id: String,
    },
}

#[derive(Subcommand)]
enum MetricsCmd {
    /// P50/P95/P99 wall time + decode throughput per type × endpoint
    Summary,
}

#[derive(Subcommand)]
enum LocksCmd {
    /// List active locks
    List,
    /// Manually release a lock
    Clear {
        endpoint: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();
    let cfg_path = args.config.unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path).context("loading config")?;
    let db_path = shellexpand::tilde(&cfg.database.path.to_string_lossy()).into_owned();
    let pool = db::open(Path::new(&db_path)).await?;

    match args.cmd {
        Cmd::Tasks { action } => tasks_cmd(action, &pool, &cfg).await?,
        Cmd::Metrics { action } => metrics_cmd(action, &pool).await?,
        Cmd::Locks { action } => locks_cmd(action, &pool).await?,
        Cmd::Config => config_cmd(&cfg),
        Cmd::Serve { port, bind } => serve_cmd(pool, &bind, port).await?,
    }
    Ok(())
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

async fn tasks_cmd(
    action: TasksCmd,
    pool: &sqlx::sqlite::SqlitePool,
    _cfg: &Config,
) -> Result<()> {
    match action {
        TasksCmd::List { status, repo } => {
            let tasks = db::list_tasks(pool, status.as_deref()).await?;
            let tasks: Vec<_> = if let Some(r) = &repo {
                tasks.into_iter().filter(|t| t.repo.contains(r.as_str())).collect()
            } else {
                tasks
            };
            if tasks.is_empty() {
                println!("(no tasks)");
                return Ok(());
            }
            println!("{:<8}  {:<10}  {:<10}  {:<8}  {}", "ID", "STATUS", "TYPE", "PRIORITY", "TARGET");
            println!("{}", "-".repeat(80));
            for t in &tasks {
                println!(
                    "{:<8}  {:<10}  {:<10}  {:<8}  {}",
                    &t.id[..8], t.status, t.r#type, t.priority, t.target_url,
                );
            }
            println!("\n{} task(s)", tasks.len());
        }

        TasksCmd::Show { id } => {
            let task = db::get_task(pool, &id).await?
                .with_context(|| format!("task {id} not found"))?;
            println!("ID:          {}", task.id);
            println!("Repo:        {}", task.repo);
            println!("Type:        {}", task.r#type);
            println!("Status:      {}", task.status);
            println!("Priority:    {}", task.priority);
            println!("Target:      {}", task.target_url);
            println!("Complexity:  {}", task.complexity.as_deref().unwrap_or("-"));
            println!("Retries:     {}", task.retry_count);
            println!("Created:     {}", fmt_ts(task.created_at));
            if let Some(t) = task.started_at   { println!("Started:     {}", fmt_ts(t)); }
            if let Some(t) = task.completed_at { println!("Completed:   {}", fmt_ts(t)); }
            if let Some(s) = task.wall_seconds  { println!("Wall time:   {}s", s); }
            if let Some(e) = task.endpoint      { println!("Endpoint:    {e}"); }
            if let Some(m) = task.model         { println!("Model:       {m}"); }
            if let Some(c) = task.exit_code     { println!("Exit code:   {c}"); }
            if let Some(p) = task.pr_url        { println!("PR URL:      {p}"); }
            if let Some(s) = task.result_summary{ println!("Summary:     {s}"); }
            if let Some(e) = task.error         { println!("Error:       {e}"); }
            if let Some(d) = task.decode_tps    { println!("Decode TPS:  {:.1}", d); }
            if let Some(t) = task.tokens_in     { println!("Tokens in:   {t}"); }
            if let Some(t) = task.tokens_out    { println!("Tokens out:  {t}"); }
            if let Some(p) = task.analysis_path { println!("Analysis:    {p}"); }
        }

        TasksCmd::Retry { id } => {
            let task = db::get_task(pool, &id).await?
                .with_context(|| format!("task {id} not found"))?;
            db::update_task_priority(pool, &task.id, task.priority).await?;
            db::requeue_task(pool, &task.id).await?;
            // Reset retry counter
            sqlx::query("UPDATE tasks SET retry_count = 0 WHERE id = ?")
                .bind(&task.id).execute(pool).await?;
            println!("task {id} re-queued (retry counter reset)");
        }

        TasksCmd::Skip { id } => {
            db::update_task_status(pool, &id, "skipped").await?;
            println!("task {id} marked skipped");
        }
    }
    Ok(())
}

// ── Metrics ────────────────────────────────────────────────────────────────────

async fn metrics_cmd(action: MetricsCmd, pool: &sqlx::sqlite::SqlitePool) -> Result<()> {
    match action {
        MetricsCmd::Summary => {
            let rows = db::metrics_summary(pool).await?;
            if rows.is_empty() {
                println!("(no completed tasks yet)");
                return Ok(());
            }
            println!(
                "{:<12}  {:<16}  {:>5}  {:>8}  {:>8}  {:>9}  {:>8}  {:>8}",
                "TYPE", "ENDPOINT", "RUNS", "AVG_S", "MAX_S", "AVG_TPS", "HB_ROOM", "TIMEOUTS"
            );
            println!("{}", "-".repeat(90));
            for r in &rows {
                println!(
                    "{:<12}  {:<16}  {:>5}  {:>8}  {:>8}  {:>9}  {:>8}  {:>8}",
                    r.r#type,
                    r.endpoint.as_deref().unwrap_or("-"),
                    r.runs,
                    r.avg_wall_s.map(|v| format!("{v:.0}")).unwrap_or("-".into()),
                    r.max_wall_s.map(|v| v.to_string()).unwrap_or("-".into()),
                    r.avg_decode_tps.map(|v| format!("{v:.1}")).unwrap_or("-".into()),
                    r.timeout_headroom.map(|v| format!("{v:.2}x")).unwrap_or("-".into()),
                    r.container_timeouts,
                );
            }
        }
    }
    Ok(())
}

// ── Locks ──────────────────────────────────────────────────────────────────────

async fn locks_cmd(action: LocksCmd, pool: &sqlx::sqlite::SqlitePool) -> Result<()> {
    match action {
        LocksCmd::List => {
            let locks = db::list_locks(pool).await?;
            if locks.is_empty() {
                println!("(no active locks)");
                return Ok(());
            }
            let now = chrono::Utc::now().timestamp();
            println!("{:<30}  {:<8}  {:>10}  {:>14}", "ENDPOINT", "TASK", "LOCKED_AGE", "HEARTBEAT_AGE");
            println!("{}", "-".repeat(70));
            for l in &locks {
                println!(
                    "{:<30}  {:<8}  {:>9}s  {:>13}s",
                    l.endpoint,
                    &l.task_id[..8],
                    now - l.locked_at,
                    now - l.heartbeat_at,
                );
            }
        }

        LocksCmd::Clear { endpoint } => {
            db::release_lock(pool, &endpoint).await?;
            println!("lock for {endpoint} released");
        }
    }
    Ok(())
}

// ── Config validation ──────────────────────────────────────────────────────────

fn config_cmd(cfg: &Config) {
    println!("Config OK");
    println!("  DB path:      {}", cfg.database.path.display());
    println!("  Task dir:     {}", cfg.database.task_dir.display());
    println!("  Orchestrator: {} @ {}", cfg.orchestrator.model, cfg.orchestrator.endpoint);
    println!("  Executors ({}):", cfg.executors.len());
    for e in &cfg.executors {
        println!("    - {} ({}) @ {}", e.name, e.model, e.endpoint);
    }
    println!("  Repos ({}):", cfg.repos.len());
    for r in &cfg.repos {
        println!("    - {} {:?}", r.url, r.actions);
    }
    println!("  Idle guard: enabled={} threshold={:.0}% window={}s",
        cfg.idle_guard.enabled, cfg.idle_guard.threshold_pct, cfg.idle_guard.window_s);
}

fn fmt_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

// ── Dashboard server ───────────────────────────────────────────────────────────

async fn serve_cmd(pool: SqlitePool, bind: &str, port: u16) -> Result<()> {
    let addr = format!("{bind}:{port}");
    let app = Router::new()
        .route("/", get(dashboard_handler))
        .with_state(pool);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .with_context(|| format!("binding to {addr}"))?;
    println!("laio dashboard running at http://{addr}");
    axum::serve(listener, app).await.context("axum server")?;
    Ok(())
}

async fn dashboard_handler(State(pool): State<SqlitePool>) -> Html<String> {
    match build_dashboard(&pool).await {
        Ok(data) => Html(data.render()),
        Err(e)   => Html(format!("<pre>Error: {e}</pre>")),
    }
}

async fn build_dashboard(pool: &SqlitePool) -> Result<DashboardData> {
    let generated_at = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let (status_counts, active_runs, recent_done, failures, metrics, repos) = tokio::try_join!(
        db::count_by_status(pool),
        db::active_runs(pool),
        db::recent_completions(pool, 86400),
        db::recent_failures(pool),
        db::metrics_summary(pool),
        db::list_repo_states(pool),
    )?;
    Ok(DashboardData { generated_at, status_counts, active_runs, recent_done, failures, metrics, repos })
}
