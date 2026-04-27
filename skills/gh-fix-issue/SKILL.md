---
name: gh-fix-issue
description: "Fix a GitHub issue and open a pull request, then iterate through CI and code review until the PR is ready to merge. Use when the user provides a GitHub issue URL and wants it analyzed and fixed autonomously. Covers the full lifecycle: reading the issue, cloning the repo, studying contribution guidelines and code style, implementing the fix, opening a PR, waiting for CI, and addressing reviewer comments."
---

# GitHub Issue → PR

Autonomous workflow for fixing a GitHub issue and shepherding the resulting PR to merge-ready.

## Phase 1: Orient

1. **Read the issue** — extract repo, issue number, description, and any linked code or reproduction steps:
   ```bash
   gh issue view <number> --repo <owner>/<repo> --comments
   ```

2. **Clone (shallow)**:
   ```bash
   git clone --depth=1 https://github.com/<owner>/<repo>.git && cd <repo>
   git checkout -b fix/<number>-<short-slug>
   ```

3. **Study contribution guidelines** — read in priority order, stopping when you have enough:
   - `CONTRIBUTING.md`, `.github/CONTRIBUTING.md`
   - `AGENTS.md`, `.github/AGENTS.md`, `CLAUDE.md`
   - `README.md` (build/test section)
   - CI config (`.github/workflows/`) — find how tests and linters are run

4. **Understand code style** — before touching anything, read 2–3 files near the change site to internalize naming conventions, formatting, comment density, error-handling patterns, and import order. Check for formatter/linter config: `.editorconfig`, `rustfmt.toml`, `.eslintrc`, `pyproject.toml`, etc.

5. **Identify test conventions** — look at existing tests near the affected code to understand the preferred style (unit vs integration, assertion library, fixture patterns).

## Phase 2: Fix

1. **Understand root cause** — read the relevant source files. State the root cause in one sentence before writing any code.

2. **Plan** — describe the minimal change. Prefer surgical edits over refactors.

3. **Implement** — edit only required files. Match the style of surrounding code exactly.

4. **Test** — run the project's own test command (from the contribution guide or CI config). All existing tests must pass. Add one test that would have caught this bug.

5. **Lint/format** — run whatever formatter or linter the project uses.

6. **Commit** — check `git log --oneline -10` first and match the project's commit message style:
   ```bash
   git add <specific files>
   git commit -m "<summary>"
   ```

## Phase 3: Verify — must be green before any push

Run each check on the committed state. **Do not push if any step fails — go back to Phase 2 and fix.**

1. **Build** — compile / bundle the project:
   - Rust: `cargo build`
   - Node: `npm run build` or `yarn build`
   - Python: no-op unless the project has an explicit build step
   - Other: infer from CI config or Makefile

2. **Full test suite** — run every test, not just the ones near the change:
   - Rust: `cargo test`
   - Node: `npm test`
   - Python: `pytest` or `python -m pytest`
   - Other: infer from CI config
   
   All tests must pass. If a pre-existing test is failing (unrelated to this change), note it explicitly in the PR body but do not suppress it.

3. **Lint / type-check** — run the formatter and linter in check mode (do not auto-fix at this stage; if they need changes, go back to Phase 2 step 5):
   - Rust: `cargo clippy -- -D warnings` and `cargo fmt --check`
   - Node/TS: `npm run lint` and `npm run typecheck` (or equivalent)
   - Python: `ruff check .` or `flake8`

If any check fails: fix, amend the commit (`git commit --amend --no-edit` or a new commit), then re-run this entire phase from step 1.

## Phase 4: Open PR

Check for a PR template at `.github/PULL_REQUEST_TEMPLATE.md` and use it if present. Otherwise use a heredoc to preserve newlines and embed the full issue URL so GitHub auto-closes the issue on merge (works for same-repo and cross-repo):

```bash
git push -u origin HEAD
gh pr create --repo <owner>/<repo> \
  --title "<title>" \
  --body "$(cat <<'EOF'
Closes <full-issue-url>

## What

- 

## How

- 
EOF
)"
```

The closing keyword (`Closes`, `Fixes`, or `Resolves`) must be followed by the **full issue URL** — `#<number>` alone only works when the PR and issue are in the same repo.

Print the PR URL.

## Phase 5: CI and review loop

Repeat until merge-ready:

1. **Request Copilot review** (once, after opening):
   ```bash
   gh pr edit <number> --add-reviewer copilot-pull-request-reviewer --repo <owner>/<repo>
   ```

2. **Wait for CI** — poll until all checks complete:
   ```bash
   gh pr checks <number> --repo <owner>/<repo>
   ```
   If a check fails: read the failure log, fix, commit, push.

3. **Rebase if main has advanced**:
   ```bash
   git fetch origin main && git rebase main
   git push origin HEAD --force-with-lease
   ```

4. **Address review comments** — fetch all and go through each one:
   ```bash
   gh pr view <number> --repo <owner>/<repo> --json reviewComments,reviews,comments
   gh api repos/<owner>/<repo>/pulls/<number>/comments
   ```
   For each: accept and fix, or note why it doesn't apply. Never silently skip.

5. **Notify and re-request review from everyone who commented**:
   ```bash
   gh pr review <number> --repo <owner>/<repo> --comment \
     -b "All review comments addressed. Please re-review."

   # Collect all reviewers who left comments or reviews, then re-request
   reviewers=$(gh api repos/<owner>/<repo>/pulls/<number>/reviews \
     --jq '.[].user.login' | sort -u | tr '\n' ',' | sed 's/,$//')
   gh pr edit <number> --add-reviewer "$reviewers" --repo <owner>/<repo>
   ```

## Notes

- Always use `gh` for GitHub operations — bare `git push` often won't authenticate.
- When rebasing: use `--force-with-lease`; fall back to `--force` only if it fails.
- If the project has no test command in docs, infer it from the CI config.
- Print the PR URL after each push.
