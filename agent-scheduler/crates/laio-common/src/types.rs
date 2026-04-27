use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Pending,
    Active,
    Done,
    Failed,
    Skipped,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Active  => write!(f, "active"),
            Self::Done    => write!(f, "done"),
            Self::Failed  => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "active"  => Ok(Self::Active),
            "done"    => Ok(Self::Done),
            "failed"  => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            other     => anyhow::bail!("unknown task status: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskType {
    FixIssue,
    ReviewPr,
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FixIssue => write!(f, "fix-issue"),
            Self::ReviewPr => write!(f, "review-pr"),
        }
    }
}

impl std::str::FromStr for TaskType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "fix-issue" => Ok(Self::FixIssue),
            "review-pr" => Ok(Self::ReviewPr),
            other       => anyhow::bail!("unknown task type: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id:                 String,
    pub repo:               String,
    pub r#type:             String,
    pub target_url:         String,
    pub priority:           i64,
    pub status:             String,
    pub analysis_path:      Option<String>,
    pub complexity:         Option<String>,
    pub affected_files:     Option<String>,
    pub orchestrator_model: Option<String>,
    pub head_sha:           Option<String>,
    pub last_analysis_at:   Option<i64>,
    pub retry_count:        i64,
    pub created_at:         i64,
    pub started_at:         Option<i64>,
    pub first_heartbeat_at: Option<i64>,
    pub completed_at:       Option<i64>,
    pub wall_seconds:       Option<i64>,
    pub endpoint:           Option<String>,
    pub model:              Option<String>,
    pub timeout_configured: Option<i64>,
    pub exit_code:          Option<i64>,
    pub tokens_in:          Option<i64>,
    pub tokens_out:         Option<i64>,
    pub decode_tps:         Option<f64>,
    pub time_to_first_token: Option<f64>,
    pub pr_url:             Option<String>,
    pub result_summary:     Option<String>,
    pub error:              Option<String>,
}

impl Task {
    pub fn new(repo: String, task_type: TaskType, target_url: String, priority: i64) -> Self {
        Self {
            id:                  Uuid::new_v4().to_string(),
            repo,
            r#type:              task_type.to_string(),
            target_url,
            priority,
            status:              TaskStatus::Pending.to_string(),
            analysis_path:       None,
            complexity:          None,
            affected_files:      None,
            orchestrator_model:  None,
            head_sha:            None,
            last_analysis_at:    None,
            retry_count:         0,
            created_at:          Utc::now().timestamp(),
            started_at:          None,
            first_heartbeat_at:  None,
            completed_at:        None,
            wall_seconds:        None,
            endpoint:            None,
            model:               None,
            timeout_configured:  None,
            exit_code:           None,
            tokens_in:           None,
            tokens_out:          None,
            decode_tps:          None,
            time_to_first_token: None,
            pr_url:              None,
            result_summary:      None,
            error:               None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EndpointLock {
    pub endpoint:     String,
    pub task_id:      String,
    pub locked_at:    i64,
    pub heartbeat_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RepoState {
    pub url:             String,
    pub last_scanned_at: Option<i64>,
    pub open_issues:     Option<i64>,
    pub open_prs:        Option<i64>,
}

/// Structured output from the orchestrator LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorAnalysis {
    pub r#type:        String,
    pub complexity:    String,
    pub priority:      i64,
    pub affected_files: Vec<String>,
    pub summary:       String,
    pub approach_hint: String,
}
