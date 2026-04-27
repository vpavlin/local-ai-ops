use anyhow::Result;
use chrono::Utc;
use serde::Deserialize;
use std::path::Path;

use crate::config::IdleGuardConfig;

#[derive(Debug, Deserialize)]
struct Sample {
    ts:  i64,
    gpu: f64,
}

#[derive(Debug, PartialEq)]
pub enum IdleStatus {
    Idle { samples: usize, peak_pct: f64, avg_pct: f64 },
    Busy { reason: String },
}

impl IdleStatus {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle { .. })
    }
}

impl std::fmt::Display for IdleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle { samples, peak_pct, avg_pct } =>
                write!(f, "IDLE ({samples} samples, peak {peak_pct:.1}%, avg {avg_pct:.1}%)"),
            Self::Busy { reason } =>
                write!(f, "BUSY: {reason}"),
        }
    }
}

pub fn check(cfg: &IdleGuardConfig) -> Result<IdleStatus> {
    if !cfg.enabled {
        return Ok(IdleStatus::Idle { samples: 0, peak_pct: 0.0, avg_pct: 0.0 });
    }

    let path = shellexpand::tilde(
        cfg.stats_file.to_str().unwrap_or("~/.local/share/lemonade-idle/gpu.jsonl")
    );
    let path = Path::new(path.as_ref());

    if !path.exists() {
        return Ok(IdleStatus::Busy {
            reason: format!("stats file not found: {} — is the sampler running?", path.display()),
        });
    }

    let content = std::fs::read_to_string(path)?;
    let cutoff  = Utc::now().timestamp() - cfg.window_s as i64;

    let samples: Vec<Sample> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Sample>(l).ok())
        .filter(|s| s.ts >= cutoff)
        .collect();

    if samples.len() < cfg.min_samples {
        return Ok(IdleStatus::Busy {
            reason: format!(
                "only {} samples in window (need {}) — sampler may have just started",
                samples.len(), cfg.min_samples
            ),
        });
    }

    let peak = samples.iter().map(|s| s.gpu).fold(f64::NEG_INFINITY, f64::max);
    let avg  = samples.iter().map(|s| s.gpu).sum::<f64>() / samples.len() as f64;

    if peak > cfg.threshold_pct {
        return Ok(IdleStatus::Busy {
            reason: format!(
                "peak GPU {peak:.1}% exceeded threshold {:.1}% in the last {}min",
                cfg.threshold_pct, cfg.window_s / 60
            ),
        });
    }

    Ok(IdleStatus::Idle { samples: samples.len(), peak_pct: peak, avg_pct: avg })
}
