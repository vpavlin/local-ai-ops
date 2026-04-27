---
name: gh-review-pr
description: "Review a GitHub pull request as a code reviewer: fetch the diff and context, study contribution guidelines and existing code style, classify findings by severity, and post a structured review. Use when the user provides a PR URL or number and wants a thorough code review posted as a GitHub review (approve, request changes, or comment)."
---

# GitHub PR Review

Act as a code reviewer: gather full context, evaluate the change against the codebase's own standards, classify findings by severity, and post a structured review.

The main agent gathers context and writes a context file, then spawns two sub-agents: one to analyse and draft the review, one to post it and own the monitor/re-review loop. Each sub-agent works from a file, not from a long accumulated conversation history.

---

## Phase 1: Gather context (main agent)

1. **Check CI status first** — if CI is already failing, note it immediately:
   ```bash
   gh pr view <number> --repo <owner>/<repo> \
     --json number,title,body,author,headRefName,baseRefName, \
             additions,deletions,reviews,comments,reviewDecision, \
             mergeable,statusCheckRollup,isDraft
   ```
   If the PR is a draft, stop — do not review draft PRs.

2. **Read the diff** — always; it's the source of truth:
   ```bash
   gh pr diff <number> --repo <owner>/<repo>
   ```

3. **Check out locally** when the diff alone isn't enough to judge correctness (call sites, interfaces, tests not shown in the diff):
   ```bash
   gh pr checkout <number> --detach
   ```

4. **Read contribution guidelines and style** — in priority order:
   - `CONTRIBUTING.md`, `.github/CONTRIBUTING.md`
   - `AGENTS.md`, `.github/AGENTS.md`, `CLAUDE.md`
   - Formatter/linter config (`.editorconfig`, `rustfmt.toml`, `.eslintrc`, etc.)
   - `git log --oneline -10` — absorb commit message conventions

5. **Read 2–3 files neighbouring the change** — not to audit them, but to understand what "normal" looks like so deviations stand out.

6. **Write `/task/review-context.md`**:
   ```
   REPO: <owner>/<repo>
   PR: <number>
   PR_URL: <full URL>
   TITLE: <title>
   AUTHOR: <login>
   BASE: <base branch>
   ADDITIONS: <n>  DELETIONS: <n>
   CI_STATUS: passing | failing | pending | none
   IS_DRAFT: false

   DIFF_SUMMARY:
   <paste the full diff, or a faithful summary if it is very large>

   STYLE NOTES:
   <naming, formatting, error-handling, import order conventions>

   CODEBASE PATTERNS:
   <how the codebase handles the same problem elsewhere — relevant context>

   GUIDELINES:
   <key rules from CONTRIBUTING / AGENTS / CLAUDE if any>
   ```

7. **Spawn the analyse sub-agent**:
   ```bash
   codex --auto-approve-everything \
     "Analyse the pull request described in /task/review-context.md. Classify every finding by severity, draft the full review body, and write the result to /task/review-draft.md."
   ```

8. **On sub-agent exit**: read `/task/review-draft.md`. If the sub-agent flagged blockers or could not complete analysis, decide whether to re-spawn or escalate. Then spawn the post sub-agent:
   ```bash
   codex --auto-approve-everything \
     "Post the review from /task/review-draft.md on the PR described in /task/review-context.md, then monitor CI and author responses until the PR is merged, closed, or needs human judgement."
   ```

---

## Phase 2: Analyse and draft (sub-agent)

*This sub-agent receives `/task/review-context.md`. It does not re-fetch GitHub — everything it needs is in that file.*

1. **Read review-context.md** in full before forming any opinions.

2. **Triage every finding** into one of four buckets:

   | Level | Meaning | Action |
   |---|---|---|
   | 🔴 **Blocker** | Breaks builds, crashes, data loss, security hole, incorrect behaviour | Must fix before merge |
   | 🟠 **Architecture** | Violates codebase patterns, wrong abstraction, hidden coupling | Should fix; discuss if contested |
   | 🟡 **Bug / Quality** | Functional issue, missing test, silent failure, poor error message | Fix or explicitly accept the risk |
   | ✅ **Good** | Correct, minimal, well-tested — worth naming specifically | Acknowledge; builds trust |

   For each finding ask:
   - What breaks if this ships as-is?
   - Is it inconsistent with how the rest of the codebase does the same thing?
   - Would a test catch it?

3. **Choose the review action**:
   - `--approve` — no blockers, no architecture concerns
   - `--request-changes` — any 🔴 or 🟠 finding
   - `--comment` — observations only, no formal decision yet

4. **Draft the review body**:

   **Approve body** — short and specific, name what you verified:
   > Correct and minimal. `parse_account_id` now strips both prefixes across all three code paths, and the new test covers the previously-failing case. Approve.

   **Request-changes body** — numbered list, bold title per concern:
   ```
   Three concerns before merging:

   **1. Silent failure on missing config (🔴 Blocker)**
   When `config.toml` is absent, `load_config()` returns `Default::default()` silently.
   A user with no config file gets no error and wrong behaviour. Should return `Err` or
   emit a compile-time diagnostic.

   **2. New helper duplicates existing `util::strip_prefix` (🟠 Architecture)**
   `parse_account_id` in `ffi_codegen.rs:142` re-implements prefix stripping. The same
   logic already lives in `util.rs:87`. Extract a call to the existing function.

   **3. No test for the error path (🟡 Quality)**
   Happy-path coverage is good. A test that passes an invalid account ID and asserts
   a useful error message would prevent regressions.

   The core approach is sound — the prefix detection logic is correct and the changes
   are minimal. Just the above before merge.
   ```
   Rules: problem → impact → concrete fix. End with a positive note and a clear path to approval.

   **Comment-only body** — use `**Blocker:**` / `**Minor:**` prefixes, code fences with language tags.

5. **Write `/task/review-draft.md`**:
   ```
   ACTION: approve | request-changes | comment
   CONCERNS_COUNT: <n blockers, n architecture, n quality>

   REVIEW_BODY:
   <full review text, ready to post verbatim>
   ```

---

## Phase 3: Post and monitor (sub-agent)

*This sub-agent receives `/task/review-draft.md` and `/task/review-context.md`. It does not re-analyse the diff — its job is to post and own the loop.*

1. **Read review-draft.md**. Extract ACTION and REVIEW_BODY.

2. **Post the review** — always execute this command, do not just display it:
   ```bash
   gh pr review <number> --repo <owner>/<repo> \
     --<action> \
     --body "$(cat <<'EOF'
   <review body from draft>
   EOF
   )"
   ```

3. **Monitor CI**:
   ```bash
   gh pr checks <number> --repo <owner>/<repo>
   ```
   If CI was already failing when context was gathered, call it out in the review as a 🔴 Blocker.

4. **Poll for author response** — check for new commits or reply comments:
   ```bash
   gh pr view <number> --repo <owner>/<repo> --json reviews,comments,commits,statusCheckRollup
   ```

5. **When the author pushes a fix** — fetch the new diff for the changed areas only:
   ```bash
   gh pr diff <number> --repo <owner>/<repo>
   ```
   Verify each concern from the draft was addressed. Post a follow-up review:
   - All resolved → `--approve`
   - Something remains → `--request-changes` or `--comment` listing what's still open

6. **Re-request review from everyone who left a comment** after each follow-up:
   ```bash
   reviewers=$(gh api repos/<owner>/<repo>/pulls/<number>/reviews \
     --jq '.[].user.login' | sort -u | tr '\n' ',' | sed 's/,$//')
   gh pr edit <number> --add-reviewer "$reviewers" --repo <owner>/<repo>
   ```

7. **Keep watching** until the PR is merged, closed, or a situation requires human judgement (e.g. contested architecture decision, the author is unresponsive for 48h, CI is broken at the repo level).

---

## Notes

- Read the diff before the PR description — the description says what was intended, the diff says what actually happened.
- When the PR description is missing or vague, note it as a 🟡 quality issue.
- If CI is already failing when you start, call it out as a 🔴 Blocker immediately.
- Never approve a draft PR.
