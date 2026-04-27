use anyhow::{Context, Result};
use clap::Parser;
use laio_common::{
    config::Config,
    db,
    lemonade::LemonadeClient,
    types::{OrchestratorAnalysis, RepoState, Task, TaskType},
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Parser)]
#[command(name = "laio-orchestrator", about = "Triage GitHub repos and queue tasks")]
struct Cli {
    #[arg(long, env = "LAIO_CONFIG")]
    config: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct GhIssue {
    number: i64,
    title: String,
    body: Option<String>,
    labels: Vec<GhLabel>,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GhPr {
    number: i64,
    title: String,
    body: Option<String>,
    author: Option<GhUser>,
    additions: Option<i64>,
    deletions: Option<i64>,
    #[serde(rename = "headRefOid")]
    head_ref_oid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = Cli::parse();
    let cfg_path = args.config.unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path).context("loading config")?;

    let db_path = shellexpand::tilde(&cfg.database.path.to_string_lossy()).into_owned();
    let pool = db::open(Path::new(&db_path)).await?;
    let client = LemonadeClient::new(&cfg.orchestrator.endpoint);
    let task_dir = PathBuf::from(
        shellexpand::tilde(&cfg.database.task_dir.to_string_lossy()).into_owned()
    );

    for repo_cfg in &cfg.repos {
        info!("scanning {}", repo_cfg.url);
        let (owner, repo) = parse_repo_url(&repo_cfg.url)?;
        let slug = format!("{owner}/{repo}");
        let mut new_tasks = 0usize;
        let mut open_issues = 0i64;
        let mut open_prs = 0i64;

        if repo_cfg.actions.iter().any(|a| a == "fix-issue") {
            match gh_issue_list(&slug).await {
                Ok(issues) => {
                    open_issues = issues.len() as i64;
                    for issue in &issues {
                        if new_tasks >= cfg.orchestrator.max_tasks_per_scan { break; }
                        let url = format!("https://github.com/{slug}/issues/{}", issue.number);
                        if db::get_task_by_url(&pool, &url).await?.is_some() { continue; }
                        match enqueue_issue(&pool, &client, &cfg, &task_dir, &repo_cfg.url, &slug, issue).await {
                            Ok(()) => new_tasks += 1,
                            Err(e) => warn!("issue #{}: {e}", issue.number),
                        }
                    }
                }
                Err(e) => warn!("gh issue list {slug}: {e}"),
            }
        }

        if repo_cfg.actions.iter().any(|a| a == "review-pr") {
            match gh_pr_list(&slug).await {
                Ok(prs) => {
                    open_prs = prs.len() as i64;
                    for pr in &prs {
                        if new_tasks >= cfg.orchestrator.max_tasks_per_scan { break; }
                        let url = format!("https://github.com/{slug}/pull/{}", pr.number);
                        let head_sha = pr.head_ref_oid.clone().unwrap_or_default();
                        if let Some(existing) = db::get_task_by_url(&pool, &url).await? {
                            if existing.status == "done"
                                && existing.head_sha.as_deref() != Some(&head_sha)
                            {
                                requeue_pr_with_new_sha(
                                    &pool, &existing, &head_sha, &cfg.orchestrator.model,
                                ).await?;
                            }
                            continue;
                        }
                        match enqueue_pr(&pool, &client, &cfg, &task_dir, &repo_cfg.url, &slug, pr).await {
                            Ok(()) => new_tasks += 1,
                            Err(e) => warn!("PR #{}: {e}", pr.number),
                        }
                    }
                }
                Err(e) => warn!("gh pr list {slug}: {e}"),
            }
        }

        // Re-analyse failed tasks that haven't hit max_retries
        let failed = db::list_tasks_by_repo_status(&pool, &repo_cfg.url, "failed").await?;
        for task in failed {
            if new_tasks >= cfg.orchestrator.max_tasks_per_scan { break; }
            if task.retry_count >= cfg.orchestrator.max_retries as i64 {
                warn!("task {} at max_retries ({}), skipping", &task.id[..8], task.retry_count);
                continue;
            }
            match reanalyse_failed(&pool, &client, &cfg, &slug, &task).await {
                Ok(()) => new_tasks += 1,
                Err(e) => warn!("re-analysis {} failed: {e}", &task.id[..8]),
            }
        }

        db::upsert_repo_state(&pool, &RepoState {
            url: repo_cfg.url.clone(),
            last_scanned_at: Some(chrono::Utc::now().timestamp()),
            open_issues: Some(open_issues),
            open_prs: Some(open_prs),
        }).await?;

        info!("repo done — {new_tasks} tasks queued, {open_issues} issues / {open_prs} PRs open");
    }

    Ok(())
}

// ── GitHub helpers ─────────────────────────────────────────────────────────────

fn parse_repo_url(url: &str) -> Result<(String, String)> {
    let path = url
        .trim_end_matches('/')
        .trim_start_matches("https://github.com/");
    let mut parts = path.splitn(2, '/');
    let owner = parts.next().context("missing owner in repo URL")?;
    let repo  = parts.next().context("missing repo in repo URL")?;
    Ok((owner.to_string(), repo.to_string()))
}

async fn gh_issue_list(slug: &str) -> Result<Vec<GhIssue>> {
    let out = Command::new("gh")
        .args([
            "issue", "list", "--repo", slug, "--state", "open",
            "--json", "number,title,body,labels", "--limit", "50",
        ])
        .output().await.context("running gh issue list")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("parsing gh issue list")
}

async fn gh_pr_list(slug: &str) -> Result<Vec<GhPr>> {
    let out = Command::new("gh")
        .args([
            "pr", "list", "--repo", slug, "--state", "open",
            "--json", "number,title,body,author,additions,deletions,headRefOid",
            "--limit", "50",
        ])
        .output().await.context("running gh pr list")?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("parsing gh pr list")
}

async fn gh_view_json(kind: &str, slug: &str, number: i64, fields: &str) -> Result<String> {
    let num = number.to_string();
    let out = Command::new("gh")
        .args([kind, "view", &num, "--repo", slug, "--json", fields])
        .output().await.context(format!("gh {kind} view {number}"))?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn extract_number_from_url(url: &str) -> Result<i64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .with_context(|| format!("no trailing number in URL: {url}"))
}

// ── Task enqueueing ────────────────────────────────────────────────────────────

async fn enqueue_issue(
    pool: &sqlx::SqlitePool,
    client: &LemonadeClient,
    cfg: &Config,
    task_dir: &Path,
    repo_url: &str,
    slug: &str,
    issue: &GhIssue,
) -> Result<()> {
    let url = format!("https://github.com/{slug}/issues/{}", issue.number);
    let labels = issue.labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ");
    let prompt = format!(
        "You are an AI engineering assistant. Analyze this GitHub issue and return JSON only.\n\
         No markdown fences, no explanation — raw JSON object only.\n\n\
         Repository: {slug}\n\
         Issue #{num}: {title}\n\n\
         Body:\n{body}\n\n\
         Labels: {labels}\n\n\
         Return this exact JSON shape:\n\
         {{\"type\":\"fix-issue\",\"complexity\":\"low\",\"priority\":5,\
         \"affected_files\":[],\"summary\":\"\",\"approach_hint\":\"\"}}",
        num   = issue.number,
        title = issue.title,
        body  = issue.body.as_deref().unwrap_or("(no body)"),
    );
    let analysis = call_and_parse_llm(client, &cfg.orchestrator.model, &prompt).await?;
    insert_task(pool, cfg, task_dir, repo_url, TaskType::FixIssue, url, &analysis, None).await?;
    info!("queued fix-issue for {slug}#{} (priority {})", issue.number, analysis.priority);
    Ok(())
}

async fn enqueue_pr(
    pool: &sqlx::SqlitePool,
    client: &LemonadeClient,
    cfg: &Config,
    task_dir: &Path,
    repo_url: &str,
    slug: &str,
    pr: &GhPr,
) -> Result<()> {
    let url = format!("https://github.com/{slug}/pull/{}", pr.number);
    let head_sha = pr.head_ref_oid.clone();
    let author = pr.author.as_ref().map(|u| u.login.as_str()).unwrap_or("unknown");
    let prompt = format!(
        "You are an AI code reviewer. Analyze this GitHub PR and return JSON only.\n\
         No markdown fences, no explanation — raw JSON object only.\n\n\
         Repository: {slug}\n\
         PR #{num}: {title}\n\
         Author: {author}\n\
         Changes: +{add} / -{del}\n\n\
         Body:\n{body}\n\n\
         Return this exact JSON shape:\n\
         {{\"type\":\"review-pr\",\"complexity\":\"low\",\"priority\":5,\
         \"affected_files\":[],\"summary\":\"\",\"approach_hint\":\"\"}}",
        num   = pr.number,
        title = pr.title,
        add   = pr.additions.unwrap_or(0),
        del   = pr.deletions.unwrap_or(0),
        body  = pr.body.as_deref().unwrap_or("(no body)"),
    );
    let analysis = call_and_parse_llm(client, &cfg.orchestrator.model, &prompt).await?;
    insert_task(pool, cfg, task_dir, repo_url, TaskType::ReviewPr, url, &analysis, head_sha).await?;
    info!("queued review-pr for {slug}#{} (priority {})", pr.number, analysis.priority);
    Ok(())
}

async fn insert_task(
    pool: &sqlx::SqlitePool,
    cfg: &Config,
    task_dir: &Path,
    repo_url: &str,
    task_type: TaskType,
    target_url: String,
    analysis: &OrchestratorAnalysis,
    head_sha: Option<String>,
) -> Result<Task> {
    let mut task = Task::new(repo_url.to_string(), task_type, target_url, analysis.priority);
    let tdir = task_dir.join(&task.id);
    tokio::fs::create_dir_all(&tdir).await?;

    let md = format!(
        "# Analysis\n\n\
         **Type**: {}\n\
         **Complexity**: {}\n\
         **Priority**: {}\n\
         **Summary**: {}\n\
         **Approach**: {}\n\
         **Affected files**: {}\n\n\
         ---\n\n```json\n{}\n```\n",
        analysis.r#type, analysis.complexity, analysis.priority,
        analysis.summary, analysis.approach_hint,
        analysis.affected_files.join(", "),
        serde_json::to_string_pretty(analysis)?,
    );
    let analysis_path = tdir.join("analysis.md");
    tokio::fs::write(&analysis_path, &md).await?;

    task.analysis_path    = Some(analysis_path.to_string_lossy().into_owned());
    task.complexity       = Some(analysis.complexity.clone());
    task.affected_files   = Some(serde_json::to_string(&analysis.affected_files)?);
    task.orchestrator_model = Some(cfg.orchestrator.model.clone());
    task.head_sha         = head_sha;

    db::insert_task(pool, &task).await?;
    Ok(task)
}

async fn requeue_pr_with_new_sha(
    pool: &sqlx::SqlitePool,
    task: &Task,
    new_sha: &str,
    model: &str,
) -> Result<()> {
    info!(
        "re-queuing PR task {} — new sha {}",
        &task.id[..8],
        &new_sha[..new_sha.len().min(8)],
    );
    if let Some(p) = &task.analysis_path {
        let note = format!(
            "\n\n---\nRe-queued {}: new commits detected (was: {})\n",
            chrono::Utc::now().to_rfc3339(),
            task.head_sha.as_deref().unwrap_or("unknown"),
        );
        let mut content = tokio::fs::read_to_string(p).await.unwrap_or_default();
        content.push_str(&note);
        let _ = tokio::fs::write(p, content).await;
    }
    db::update_analysis(
        pool, &task.id,
        task.analysis_path.as_deref().unwrap_or(""),
        task.complexity.as_deref().unwrap_or(""),
        task.affected_files.as_deref().unwrap_or(""),
        model, Some(new_sha),
    ).await?;
    db::update_task_priority(pool, &task.id, task.priority + 2).await?;
    db::requeue_task(pool, &task.id).await?;
    Ok(())
}

async fn reanalyse_failed(
    pool: &sqlx::SqlitePool,
    client: &LemonadeClient,
    cfg: &Config,
    slug: &str,
    task: &Task,
) -> Result<()> {
    info!("re-analysing failed task {}", &task.id[..8]);
    let is_issue = task.r#type == "fix-issue";
    let number = extract_number_from_url(&task.target_url)?;
    let (kind, fields) = if is_issue {
        ("issue", "number,title,body,labels,comments")
    } else {
        ("pr", "number,title,body,author,additions,deletions,headRefOid,reviews,comments")
    };
    let context_json = gh_view_json(kind, slug, number, fields).await?;
    let prompt = format!(
        "You are an AI engineering assistant re-analyzing a GitHub {kind} whose automated fix previously failed.\n\
         Previous error: {}\n\
         Return JSON only — no markdown, no explanation.\n\n\
         Context:\n{context_json}\n\n\
         Return this exact JSON shape:\n\
         {{\"type\":\"{}\",\"complexity\":\"low\",\"priority\":5,\
         \"affected_files\":[],\"summary\":\"\",\"approach_hint\":\"\"}}",
        task.error.as_deref().unwrap_or("(no error recorded)"),
        task.r#type,
    );
    let analysis = call_and_parse_llm(client, &cfg.orchestrator.model, &prompt).await?;

    if let Some(p) = &task.analysis_path {
        let note = format!(
            "\n\n---\nRe-analysis {} (retry #{}):\n\
             - Complexity: {}\n\
             - Approach: {}\n",
            chrono::Utc::now().to_rfc3339(),
            task.retry_count + 1,
            analysis.complexity, analysis.approach_hint,
        );
        let mut content = tokio::fs::read_to_string(p).await.unwrap_or_default();
        content.push_str(&note);
        let _ = tokio::fs::write(p, content).await;
    }

    db::update_analysis(
        pool, &task.id,
        task.analysis_path.as_deref().unwrap_or(""),
        &analysis.complexity,
        &serde_json::to_string(&analysis.affected_files)?,
        &cfg.orchestrator.model,
        task.head_sha.as_deref(),
    ).await?;
    db::requeue_task(pool, &task.id).await?;
    Ok(())
}

// ── LLM helpers ───────────────────────────────────────────────────────────────

async fn call_and_parse_llm(
    client: &LemonadeClient,
    model: &str,
    prompt: &str,
) -> Result<OrchestratorAnalysis> {
    let raw = client.chat(model, prompt).await?;
    parse_llm_json(&raw).with_context(|| format!("LLM response was:\n{raw}"))
}

fn parse_llm_json(raw: &str) -> Result<OrchestratorAnalysis> {
    // Try raw first
    if let Ok(a) = serde_json::from_str::<OrchestratorAnalysis>(raw.trim()) {
        return Ok(a);
    }
    // Strip optional ```json ... ``` fences
    let stripped = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    serde_json::from_str(stripped)
        .context("cannot parse LLM output as OrchestratorAnalysis JSON")
}
