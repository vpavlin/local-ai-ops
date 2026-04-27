use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct SystemStats {
    pub cpu_percent: f64,
    pub gpu_percent: f64,
    pub memory_gb:   f64,
    pub vram_gb:     Option<f64>,
    pub npu_percent: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InferenceStats {
    pub time_to_first_token: f64,
    pub tokens_per_second:   f64,
    pub input_tokens:        i64,
    pub output_tokens:       i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    pub status:       String,
    pub model_loaded: Option<String>,
}

pub struct LemonadeClient {
    client:   Client,
    base_url: String,
}

impl LemonadeClient {
    pub fn new(base_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("building reqwest client");
        Self { client, base_url: base_url.trim_end_matches('/').to_string() }
    }

    pub async fn system_stats(&self) -> Result<SystemStats> {
        self.client
            .get(format!("{}/api/v1/system-stats", self.base_url))
            .send().await
            .context("GET system-stats")?
            .json::<SystemStats>().await
            .context("parsing system-stats")
    }

    pub async fn inference_stats(&self) -> Result<InferenceStats> {
        self.client
            .get(format!("{}/api/v1/stats", self.base_url))
            .send().await
            .context("GET stats")?
            .json::<InferenceStats>().await
            .context("parsing inference stats")
    }

    pub async fn health(&self) -> Result<HealthResponse> {
        self.client
            .get(format!("{}/api/v1/health", self.base_url))
            .send().await
            .context("GET health")?
            .json::<HealthResponse>().await
            .context("parsing health")
    }

    /// Calls the OpenAI-compatible `/v1/chat/completions` endpoint.
    /// Uses a long timeout (2h) since LLM inference can take a while.
    pub async fn chat(&self, model: &str, prompt: &str) -> Result<String> {
        #[derive(Deserialize)]
        struct Choice { message: Msg }
        #[derive(Deserialize)]
        struct Msg { content: String }
        #[derive(Deserialize)]
        struct Resp { choices: Vec<Choice> }

        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 2048
        });
        let resp: Resp = self.client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .timeout(Duration::from_secs(7200))
            .json(&body)
            .send().await
            .context("POST chat/completions")?
            .json().await
            .context("parsing chat/completions response")?;
        resp.choices.into_iter().next()
            .map(|c| c.message.content)
            .context("empty choices in chat response")
    }

    /// Returns true if the GPU is below `threshold_pct` (server is idle).
    pub async fn is_idle(&self, threshold_pct: f64) -> bool {
        match self.system_stats().await {
            Ok(s)  => s.gpu_percent < threshold_pct,
            Err(e) => {
                tracing::warn!("lemonade unreachable at {}: {e} — treating as idle", self.base_url);
                true
            }
        }
    }
}
