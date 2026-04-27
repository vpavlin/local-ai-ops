use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub database:     DatabaseConfig,
    pub orchestrator: OrchestratorConfig,
    pub executors:    Vec<ExecutorConfig>,
    pub repos:        Vec<RepoConfig>,
    pub timeouts:     TimeoutsConfig,
    pub idle_guard:   IdleGuardConfig,
    pub container:    ContainerConfig,
    pub skills:       SkillsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    pub path:     PathBuf,
    pub task_dir: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrchestratorConfig {
    pub endpoint:          String,
    pub model:             String,
    #[serde(default = "default_max_tasks")]
    pub max_tasks_per_scan: usize,
    #[serde(default = "default_max_retries")]
    pub max_retries:        u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExecutorConfig {
    pub name:     String,
    pub endpoint: String,
    pub model:    String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepoConfig {
    pub url:     String,
    pub actions: Vec<String>,
    #[serde(default = "default_priority")]
    pub priority: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeoutsConfig {
    #[serde(default = "default_timeout_s")]
    pub default_s: u64,
    #[serde(default)]
    pub per_type: HashMap<String, u64>,
    #[serde(default)]
    pub per_endpoint: HashMap<String, u64>,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_s: u64,
    #[serde(default = "default_heartbeat_stale")]
    pub heartbeat_stale_after_s: u64,
    #[serde(default = "default_lock_max_age")]
    pub lock_max_age_s: u64,
}

impl TimeoutsConfig {
    pub fn for_task(&self, task_type: &str, executor_name: &str) -> u64 {
        self.per_endpoint
            .get(executor_name)
            .or_else(|| self.per_type.get(task_type))
            .copied()
            .unwrap_or(self.default_s)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IdleGuardConfig {
    #[serde(default = "default_true")]
    pub enabled:      bool,
    pub stats_file:   PathBuf,
    #[serde(default = "default_threshold")]
    pub threshold_pct: f64,
    #[serde(default = "default_min_samples")]
    pub min_samples:  usize,
    #[serde(default = "default_window_s")]
    pub window_s:     u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContainerConfig {
    #[serde(default = "default_image")]
    pub image:        String,
    #[serde(default = "default_runtime")]
    pub runtime:      String,
    #[serde(default = "default_gh_token_env")]
    pub gh_token_env: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillsConfig {
    pub dir: PathBuf,
    #[serde(default = "default_true")]
    pub auto_update: bool,
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config from {}", path.display()))?;
        let cfg: Config = serde_yaml::from_str(&text)
            .with_context(|| format!("parsing config from {}", path.display()))?;
        Ok(cfg)
    }

    pub fn default_path() -> PathBuf {
        let base = std::env::var("LAIO_CONFIG").ok().map(PathBuf::from);
        base.unwrap_or_else(|| {
            dirs_next::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("laio")
                .join("config.yaml")
        })
    }
}

fn default_max_tasks()         -> usize { 20 }
fn default_max_retries()       -> u32   { 3 }
fn default_max_concurrent()    -> usize { 1 }
fn default_priority()          -> i64   { 5 }
fn default_timeout_s()         -> u64   { 14400 }
fn default_heartbeat_interval() -> u64  { 120 }
fn default_heartbeat_stale()   -> u64   { 600 }
fn default_lock_max_age()      -> u64   { 86400 }
fn default_threshold()         -> f64   { 25.0 }
fn default_min_samples()       -> usize { 10 }
fn default_window_s()          -> u64   { 900 }
fn default_true()              -> bool  { true }
fn default_image()             -> String { "local-ai-ops-runner:latest".into() }
fn default_runtime()           -> String { "podman".into() }
fn default_gh_token_env()      -> String { "GH_TOKEN".into() }
