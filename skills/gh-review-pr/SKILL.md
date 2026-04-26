---
name: gh-review-pr
description: "Review a GitHub pull request as a code reviewer: fetch the diff and context, study contribution guidelines and existing code style, classify findings by severity, and post a structured review. Use when the user provides a PR URL or number and wants a thorough code review posted as a GitHub review (approve, request changes, or comment)."
---

# GitHub PR Review

Act as a code reviewer: gather full context, evaluate the change against the codebase's own standards, classify findings by severity, and post a structured review.

## Phase 1: Gather context

1. **Fetch PR metadata**:
   ```bash
   gh pr view <number> --repo <owner>/<repo> \
     --json number,title,body,author,headRefName,baseRefName,\
   additions,deletions,files,reviews,comments,reviewDecision,\
   mergeable,statusCheckRollup,isDraft
   ```

2. **Read the diff** — always do this; it's the source of truth:
   ```bash
   gh pr diff <number> --repo <owner>/<repo>
   ```

3. **Check out locally** when the diff alone isn't enough to judge correctness (e.g. call sites, tests, interfaces not shown in the diff):
   ```bash
   gh pr checkout <number> --detach
   ```

4. **Read contribution guidelines and style** — same priority order as when fixing an issue:
   - `CONTRIBUTING.md`, `.github/CONTRIBUTING.md`
   - `AGENTS.md`, `.github/AGENTS.md`, `CLAUDE.md`
   - Formatter/linter config (`.editorconfig`, `rustfmt.toml`, `.eslintrc`, etc.)
   - `git log --oneline -10` to absorb commit message conventions

5. **Read 2–3 files neighbouring the change** — not to audit them, but to internalize what "normal" looks like in this codebase so deviations stand out.

## Phase 2: Classify findings

Triage every finding into one of four buckets before writing the review:

| Level | Meaning | Action |
|---|---|---|
| 🔴 **Blocker** | Breaks builds, crashes, data loss, security hole, incorrect behaviour | Must fix before merge |
| 🟠 **Architecture** | Violates codebase patterns, wrong abstraction, hidden coupling | Should fix; discuss if contested |
| 🟡 **Bug / Quality** | Functional issue, missing test, silent failure, poor error message | Fix or explicitly accept the risk |
| ✅ **Good** | Correct, minimal, well-tested — worth naming specifically | Acknowledge; builds trust |

Ask yourself for each finding:
- What breaks if this ships as-is?
- Is it inconsistent with how the rest of the codebase does the same thing?
- Would a test catch it?

## Phase 3: Post the review

**Always execute the review by running the `gh pr review` command — do not just display it or describe what you would post.**

Choose the flag based on your findings:
- `--approve` — no blockers, no architecture concerns
- `--request-changes` — any 🔴 or 🟠 finding
- `--comment` — observations only, no formal decision yet

Use a heredoc to preserve formatting:

```bash
gh pr review <number> --repo <owner>/<repo> --approve --body "$(cat <<'EOF'
<review body here>
EOF
)"
```

### Approve body

Keep it short and specific — name what you actually verified:

> Correct and minimal. `parse_account_id` now strips both prefixes across all three code paths, and the new test covers the previously-failing case. Approve.

### Request-changes body

Structure as numbered concerns with bold titles:

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
a useful error message is returned would prevent regressions.

The core approach is sound — the prefix detection logic is correct and the changes are minimal. Just the above before merge.
```

Rules:
- Numbered list, **bold title** per concern, severity marker
- Problem → impact on user/system → concrete fix
- End with a positive note and a clear path to approval

### Comment-only body

Use `**Blocker:**` / `**Minor:**` prefixes. Include code fences with language tags for examples. Same structure as request-changes but lighter tone.

## Phase 4: Monitor and re-review

After posting:

1. **Watch CI**:
   ```bash
   gh pr checks <number> --repo <owner>/<repo>
   ```

2. **Poll for author response** — check for new commits or reply comments:
   ```bash
   gh pr view <number> --repo <owner>/<repo> --json reviews,comments,commits
   ```

3. **When the author pushes a fix** — re-read the diff for the changed areas only, verify each concern was addressed, then post a follow-up review (approve if resolved, otherwise comment on what remains).

4. **Keep watching** until the PR is merged, closed, or you flag that human judgement is needed.

## Notes

- Read the diff before the PR description — the description says what was intended, the diff says what actually happened.
- When the PR description is missing or vague, note it as a 🟡 quality issue.
- If CI is already failing when you start, call it out as a 🔴 Blocker immediately — don't review the code until it's green.
- Never approve a draft PR.
