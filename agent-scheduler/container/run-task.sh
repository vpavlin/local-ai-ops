#!/usr/bin/env bash
set -euo pipefail

# Required env:
#   TASK_ID            — task UUID
#   TASK_TYPE          — fix-issue | review-pr
#   TARGET_URL         — full GitHub issue/PR URL
#   LEMONADE_URL       — Lemonade server endpoint
#   GH_TOKEN           — GitHub token for gh CLI
#   HEARTBEAT_FILE     — path to write heartbeat timestamps (/task/heartbeat)
#   HEARTBEAT_INTERVAL_S — seconds between heartbeats

TASK_DIR="/task"
ANALYSIS_FILE="${TASK_DIR}/analysis.md"
RESULT_FILE="${TASK_DIR}/result.md"

echo "[run-task] task=${TASK_ID} type=${TASK_TYPE} url=${TARGET_URL}"

# ── Configure gh CLI ──────────────────────────────────────────────────────────
echo "${GH_TOKEN}" | gh auth login --with-token

# ── Configure Codex to use Lemonade (OpenAI-compatible) ──────────────────────
mkdir -p ~/.codex
cat > ~/.codex/config.json <<EOF
{
  "provider": "openai",
  "baseURL": "${LEMONADE_URL}/v1",
  "model": "${MODEL:-Qwen3.6-35B-A3B}"
}
EOF

# ── Start heartbeat loop ──────────────────────────────────────────────────────
heartbeat_loop() {
    while true; do
        date +%s > "${HEARTBEAT_FILE}"
        sleep "${HEARTBEAT_INTERVAL_S}"
    done
}
heartbeat_loop &
HEARTBEAT_PID=$!
trap 'kill ${HEARTBEAT_PID} 2>/dev/null || true' EXIT

# ── Build Codex prompt from analysis.md ──────────────────────────────────────
if [[ -f "${ANALYSIS_FILE}" ]]; then
    ANALYSIS=$(cat "${ANALYSIS_FILE}")
else
    ANALYSIS="No analysis available. Work from the URL directly."
fi

case "${TASK_TYPE}" in
    fix-issue)
        SKILL="gh-fix-issue"
        PROMPT="Fix the GitHub issue at ${TARGET_URL}

Analysis from orchestrator:
${ANALYSIS}"
        ;;
    review-pr)
        SKILL="gh-review-pr"
        PROMPT="Review the GitHub PR at ${TARGET_URL}

Analysis from orchestrator:
${ANALYSIS}"
        ;;
    *)
        echo "[run-task] ERROR: unknown task type: ${TASK_TYPE}" >&2
        exit 2
        ;;
esac

# ── Run Codex ─────────────────────────────────────────────────────────────────
CODEX_OUTPUT_FILE="${TASK_DIR}/codex-output.txt"
set +e
codex --skill "${SKILL}" --auto-approve-everything "${PROMPT}" 2>&1 | tee "${CODEX_OUTPUT_FILE}"
CODEX_EXIT=${PIPESTATUS[0]}
set -e

# ── Extract PR URL from output ────────────────────────────────────────────────
PR_URL=""
if [[ -f "${CODEX_OUTPUT_FILE}" ]]; then
    PR_URL=$(grep -oP 'https://github\.com/[^/]+/[^/]+/pull/[0-9]+' "${CODEX_OUTPUT_FILE}" | tail -1 || true)
fi

# ── Write result.md ───────────────────────────────────────────────────────────
{
    [[ -n "${PR_URL}" ]] && echo "PR_URL=${PR_URL}"
    echo "SUMMARY=Codex exit code ${CODEX_EXIT} for ${TASK_TYPE} on ${TARGET_URL}"
} > "${RESULT_FILE}"

echo "[run-task] done, exit=${CODEX_EXIT}"
exit "${CODEX_EXIT}"
