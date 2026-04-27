use chrono::{DateTime, Utc};
use laio_common::db::{ActiveRun, MetricsSummary};
use laio_common::types::{RepoState, Task};

pub struct DashboardData {
    pub generated_at:    String,
    pub status_counts:   Vec<(String, i64)>,
    pub active_runs:     Vec<ActiveRun>,
    pub recent_done:     Vec<Task>,
    pub failures:        Vec<Task>,
    pub metrics:         Vec<MetricsSummary>,
    pub repos:           Vec<RepoState>,
}

impl DashboardData {
    pub fn render(&self) -> String {
        let s = self;
        let counts = status_counts_map(&s.status_counts);
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="30">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>laio — agent scheduler</title>
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;
     background:#0f1117;color:#e2e8f0;font-size:14px;line-height:1.5}}
header{{background:#1a1d2e;border-bottom:1px solid #2d3250;
        padding:16px 24px;display:flex;align-items:center;gap:16px}}
header h1{{font-size:20px;font-weight:700;color:#7c8cf8;letter-spacing:.5px}}
header .ts{{font-size:12px;color:#64748b;margin-left:auto}}
header .refresh{{font-size:11px;color:#475569}}
main{{padding:24px;max-width:1400px;margin:0 auto;display:flex;flex-direction:column;gap:32px}}
.summary{{display:grid;grid-template-columns:repeat(5,1fr);gap:12px}}
.card{{background:#1a1d2e;border:1px solid #2d3250;border-radius:8px;
       padding:16px;text-align:center}}
.card .num{{font-size:32px;font-weight:700;line-height:1}}
.card .lbl{{font-size:12px;color:#64748b;margin-top:4px;text-transform:uppercase;letter-spacing:.5px}}
.card.pending .num{{color:#7c8cf8}}
.card.active  .num{{color:#f59e0b;animation:pulse 2s infinite}}
.card.done    .num{{color:#34d399}}
.card.failed  .num{{color:#f87171}}
.card.skipped .num{{color:#475569}}
@keyframes pulse{{0%,100%{{opacity:1}}50%{{opacity:.6}}}}
section h2{{font-size:15px;font-weight:600;color:#94a3b8;margin-bottom:12px;
            padding-bottom:8px;border-bottom:1px solid #1e2340}}
table{{width:100%;border-collapse:collapse;background:#1a1d2e;
       border:1px solid #2d3250;border-radius:8px;overflow:hidden}}
th{{background:#151827;color:#64748b;font-size:11px;text-transform:uppercase;
    letter-spacing:.5px;padding:8px 12px;text-align:left;font-weight:600}}
td{{padding:8px 12px;border-top:1px solid #1e2340;vertical-align:middle}}
tr:hover td{{background:#1e2340}}
.tag{{display:inline-block;padding:2px 8px;border-radius:9999px;
      font-size:11px;font-weight:600;letter-spacing:.3px}}
.tag.pending{{background:#1e1b4b;color:#7c8cf8}}
.tag.active {{background:#451a03;color:#f59e0b}}
.tag.done   {{background:#022c22;color:#34d399}}
.tag.failed {{background:#2d0a0a;color:#f87171}}
.tag.skipped{{background:#1e293b;color:#475569}}
.tag.fix-issue{{background:#1e2d47;color:#60a5fa}}
.tag.review-pr{{background:#1e2738;color:#a78bfa}}
a{{color:#7c8cf8;text-decoration:none}}
a:hover{{text-decoration:underline}}
.mono{{font-family:"JetBrains Mono","Fira Code",monospace;font-size:12px}}
.dim{{color:#475569}}
.warn{{color:#f59e0b}}
.err{{color:#f87171}}
.ok{{color:#34d399}}
.empty{{color:#475569;font-style:italic;padding:16px}}
</style>
</head>
<body>
<header>
  <h1>⚙ laio</h1>
  <span class="refresh">auto-refreshes every 30s</span>
  <span class="ts">Generated {generated_at}</span>
</header>
<main>

<div class="summary">
  <div class="card pending"><div class="num">{pending}</div><div class="lbl">Pending</div></div>
  <div class="card active"><div class="num">{active}</div><div class="lbl">Active</div></div>
  <div class="card done"><div class="num">{done}</div><div class="lbl">Done</div></div>
  <div class="card failed"><div class="num">{failed}</div><div class="lbl">Failed</div></div>
  <div class="card skipped"><div class="num">{skipped}</div><div class="lbl">Skipped</div></div>
</div>

<section>
  <h2>Active runs</h2>
  {active_runs_html}
</section>

<section>
  <h2>Repo health</h2>
  {repos_html}
</section>

<section>
  <h2>Recent completions (last 24 h)</h2>
  {recent_html}
</section>

<section>
  <h2>Performance metrics</h2>
  {metrics_html}
</section>

<section>
  <h2>Failures</h2>
  {failures_html}
</section>

</main>
</body>
</html>"#,
            generated_at    = s.generated_at,
            pending  = counts.get("pending").copied().unwrap_or(0),
            active   = counts.get("active").copied().unwrap_or(0),
            done     = counts.get("done").copied().unwrap_or(0),
            failed   = counts.get("failed").copied().unwrap_or(0),
            skipped  = counts.get("skipped").copied().unwrap_or(0),
            active_runs_html = render_active_runs(&s.active_runs),
            repos_html       = render_repos(&s.repos),
            recent_html      = render_completions(&s.recent_done),
            metrics_html     = render_metrics(&s.metrics),
            failures_html    = render_failures(&s.failures),
        )
    }
}

// ── Section renderers ──────────────────────────────────────────────────────────

fn render_active_runs(runs: &[ActiveRun]) -> String {
    if runs.is_empty() {
        return r#"<p class="empty">No active runs.</p>"#.into();
    }
    let now = Utc::now().timestamp();
    let mut rows = String::new();
    for r in runs {
        rows.push_str(&format!(
            "<tr>\
              <td class=\"mono\">{}</td>\
              <td>{}</td>\
              <td><a href=\"{}\" target=\"_blank\">{}</a></td>\
              <td class=\"{}\">{}</td>\
              <td class=\"{}\">{}</td>\
            </tr>",
            &r.task_id[..8],
            type_tag(&r.task_type),
            html_escape(&r.target_url), short_url(&r.target_url),
            age_class(now - r.locked_at), fmt_age(now - r.locked_at),
            age_class(now - r.heartbeat_at), fmt_age(now - r.heartbeat_at),
        ));
    }
    format!(
        "<table>\
          <thead><tr>\
            <th>Task ID</th><th>Type</th><th>URL</th>\
            <th>Lock age</th><th>Last heartbeat</th>\
          </tr></thead>\
          <tbody>{rows}</tbody>\
        </table>"
    )
}

fn render_repos(repos: &[RepoState]) -> String {
    if repos.is_empty() {
        return r#"<p class="empty">No repos scanned yet.</p>"#.into();
    }
    let now = Utc::now().timestamp();
    let mut rows = String::new();
    for r in repos {
        let scan_age = r.last_scanned_at.map(|t| fmt_age(now - t)).unwrap_or("-".into());
        rows.push_str(&format!(
            "<tr>\
              <td><a href=\"{}\" target=\"_blank\">{}</a></td>\
              <td class=\"dim\">{}</td>\
              <td>{}</td>\
              <td>{}</td>\
            </tr>",
            html_escape(&r.url), html_escape(&r.url),
            scan_age,
            r.open_issues.map(|n| n.to_string()).unwrap_or("-".into()),
            r.open_prs.map(|n| n.to_string()).unwrap_or("-".into()),
        ));
    }
    format!(
        "<table>\
          <thead><tr>\
            <th>Repo</th><th>Last scan</th><th>Open issues</th><th>Open PRs</th>\
          </tr></thead>\
          <tbody>{rows}</tbody>\
        </table>"
    )
}

fn render_completions(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return r#"<p class="empty">No completed tasks in the last 24 h.</p>"#.into();
    }
    let mut rows = String::new();
    for t in tasks {
        let wall = t.wall_seconds.map(|s| fmt_duration(s)).unwrap_or("-".into());
        let tps  = t.decode_tps.map(|v| format!("{v:.0}")).unwrap_or("-".into());
        let ts   = t.completed_at.and_then(fmt_ts_short).unwrap_or("-".into());
        rows.push_str(&format!(
            "<tr>\
              <td class=\"mono\">{}</td>\
              <td>{}</td>\
              <td><a href=\"{}\" target=\"_blank\">{}</a></td>\
              <td>{}</td>\
              <td class=\"dim\">{}</td>\
              <td class=\"dim\">{}</td>\
              <td class=\"dim\">{}</td>\
            </tr>",
            &t.id[..8],
            type_tag(&t.r#type),
            html_escape(&t.target_url), short_url(&t.target_url),
            status_tag(&t.status),
            wall, tps, ts,
        ));
    }
    format!(
        "<table>\
          <thead><tr>\
            <th>ID</th><th>Type</th><th>URL</th>\
            <th>Status</th><th>Wall time</th><th>TPS</th><th>Completed</th>\
          </tr></thead>\
          <tbody>{rows}</tbody>\
        </table>"
    )
}

fn render_metrics(rows: &[MetricsSummary]) -> String {
    if rows.is_empty() {
        return r#"<p class="empty">No metrics yet — runs complete/fail first.</p>"#.into();
    }
    let mut html = String::new();
    for r in rows {
        let headroom = r.timeout_headroom.map(|v| {
            let cls = if v < 1.5 { "warn" } else { "ok" };
            format!("<span class=\"{cls}\">{v:.2}×</span>")
        }).unwrap_or("<span class=\"dim\">-</span>".into());

        html.push_str(&format!(
            "<tr>\
              <td>{}</td>\
              <td class=\"mono\">{}</td>\
              <td>{}</td>\
              <td class=\"dim\">{}</td>\
              <td class=\"dim\">{}</td>\
              <td class=\"dim\">{}</td>\
              <td>{}</td>\
              <td class=\"{}\">{}</td>\
            </tr>",
            type_tag(&r.r#type),
            r.endpoint.as_deref().unwrap_or("-"),
            r.runs,
            r.avg_wall_s.map(|v| fmt_duration(v as i64)).unwrap_or("-".into()),
            r.max_wall_s.map(|v| fmt_duration(v)).unwrap_or("-".into()),
            r.avg_decode_tps.map(|v| format!("{v:.0} t/s")).unwrap_or("-".into()),
            headroom,
            if r.container_timeouts > 0 { "err" } else { "dim" },
            r.container_timeouts,
        ));
    }
    format!(
        "<table>\
          <thead><tr>\
            <th>Type</th><th>Endpoint</th><th>Runs</th>\
            <th>Avg wall</th><th>Max wall</th><th>Avg TPS</th>\
            <th>Timeout headroom</th><th>Timeouts</th>\
          </tr></thead>\
          <tbody>{html}</tbody>\
        </table>"
    )
}

fn render_failures(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return r#"<p class="empty">No failures. 🎉</p>"#.into();
    }
    let mut rows = String::new();
    for t in tasks {
        let ts = t.completed_at.and_then(fmt_ts_short).unwrap_or("-".into());
        let err = t.error.as_deref().unwrap_or("-");
        rows.push_str(&format!(
            "<tr>\
              <td class=\"mono\">{}</td>\
              <td>{}</td>\
              <td><a href=\"{}\" target=\"_blank\">{}</a></td>\
              <td class=\"dim\">{}</td>\
              <td class=\"err\">{}</td>\
              <td class=\"dim\">{}</td>\
            </tr>",
            &t.id[..8],
            type_tag(&t.r#type),
            html_escape(&t.target_url), short_url(&t.target_url),
            t.exit_code.map(|c| c.to_string()).unwrap_or("-".into()),
            html_escape(err),
            ts,
        ));
    }
    format!(
        "<table>\
          <thead><tr>\
            <th>ID</th><th>Type</th><th>URL</th>\
            <th>Exit</th><th>Error</th><th>At</th>\
          </tr></thead>\
          <tbody>{rows}</tbody>\
        </table>"
    )
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn status_counts_map(counts: &[(String, i64)]) -> std::collections::HashMap<&str, i64> {
    counts.iter().map(|(s, n)| (s.as_str(), *n)).collect()
}

fn status_tag(s: &str) -> String {
    format!("<span class=\"tag {s}\">{s}</span>")
}

fn type_tag(t: &str) -> String {
    format!("<span class=\"tag {t}\">{t}</span>")
}

fn short_url(url: &str) -> String {
    // Show just owner/repo#N or owner/repo/pull/N
    url.trim_start_matches("https://github.com/")
        .trim_start_matches("https://")
        .to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

fn fmt_age(secs: i64) -> String {
    fmt_duration(secs)
}

fn fmt_duration(secs: i64) -> String {
    if secs < 0 { return "-".into(); }
    if secs < 60  { return format!("{secs}s"); }
    if secs < 3600 { return format!("{}m {}s", secs / 60, secs % 60); }
    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
}

fn age_class(secs: i64) -> &'static str {
    if secs > 600 { "warn" } else { "dim" }
}

fn fmt_ts_short(ts: i64) -> Option<String> {
    DateTime::from_timestamp(ts, 0)
        .map(|dt: DateTime<Utc>| dt.format("%m-%d %H:%M").to_string())
}
