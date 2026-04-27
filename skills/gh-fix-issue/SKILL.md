---
name: gh-fix-issue
description: "Fix a GitHub issue and open a pull request, then iterate through CI and code review until the PR is ready to merge. Use when the user provides a GitHub issue URL and wants it analyzed and fixed autonomously. Covers the full lifecycle: reading the issue, cloning the repo, studying contribution guidelines and code style, implementing the fix, opening a PR, waiting for CI, and addressing reviewer comments."
---

# GitHub Issue → PR

Autonomous workflow for fixing a GitHub issue and shepherding the resulting PR to merge-ready.

The main agent plans and coordinates. It spawns focused sub-agents for implementation and for shipping, so each sub-agent works with a narrow context and does not accumulate noise from earlier steps.

---

## Phase 1: Orient and plan (main agent)

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
   - CI config (`.github/workflows/`) — find exactly how tests and linters are run

4. **Understand code style** — before touching anything, read 2–3 files near the change site to internalize naming conventions, formatting, comment density, error-handling patterns, and import order. Check for config files: `.editorconfig`, `rustfmt.toml`, `.eslintrc`, `pyproject.toml`, etc.

5. **Identify test conventions** — look at existing tests near the affected code to understand the preferred style (unit vs integration, assertion library, fixture patterns).

6. **Write plan.md** — capture everything the sub-agents will need. Save to `/task/plan.md`:
   ```
   REPO: https://github.com/<owner>/<repo>
   REPO_PATH: /path/to/cloned/repo
   BRANCH: fix/<number>-<short-slug>
   ISSUE_URL: <full issue URL>

   ROOT CAUSE: <one sentence>

   CHANGE PLAN:
   - <file to change>: <what and why>
   - ...

   BUILD COMMAND: <exact command>
   TEST COMMAND: <exact command>
   LINT COMMAND: <exact command>

   TEST CONVENTIONS: <brief note>
   STYLE NOTES: <brief note>
   ```

7. **Spawn the implement sub-agent**:
   ```bash
   codex --auto-approve-everything \
     "Implement the fix described in /task/plan.md. The repo is already cloned and the branch is already checked out. See plan.md for exact file changes, build command, test command, and style notes. When all checks pass, commit and write a summary to /task/implement-result.md."
   ```

8. **On sub-agent exit**: read `/task/implement-result.md`. If it reports failure, investigate and either fix the plan or retry. Then spawn the review sub-agent:
   ```bash
   codex --auto-approve-everything \
     "Review the implementation in /task/implement-result.md against /task/plan.md, do a final verify pass, then push and open the PR. See /task/plan.md for repo details and issue URL."
   ```

---

## Phase 2: Implement (sub-agent)

*This sub-agent receives `/task/plan.md` as its primary context. It does not re-read the issue or the contribution guide — those are already distilled in plan.md.*

1. **Read plan.md** — internalize the root cause, the exact files to change, build/test/lint commands, and style notes. Do not deviate from the plan without a clear technical reason.

2. **Implement** — edit only the files listed in plan.md. Match surrounding code style exactly.

3. **Add a test** — write one test that would have caught this bug, following the test conventions in plan.md.

4. **Build → test → lint loop** — run all three checks. For each failure, fix the specific issue and re-run from the top of the loop. Do not move on until all three are green:
   - **Build**: compile/bundle (command from plan.md)
   - **Test**: full suite, not just near the change (command from plan.md)
   - **Lint**: formatter + linter in check mode (command from plan.md)

   If a pre-existing test is failing (unrelated to this change), note it in the result file — do not delete or skip it.

   Limit: if the loop has not converged after **5 iterations**, stop and write a failure summary to `/task/implement-result.md` so the main agent can investigate.

5. **Commit** — check `git log --oneline -10` and match the project's commit message style:
   ```bash
   git add <specific files>
   git commit -m "<summary>"
   ```

6. **Write `/task/implement-result.md`**:
   ```
   STATUS: success | failure
   BRANCH: <branch name>
   COMMIT: <hash>
   CHANGED FILES: <list>
   TEST RESULT: all pass | <N> failing (pre-existing: yes/no)
   SUMMARY: <one paragraph of what was done>
   NOTES: <anything the review sub-agent should know>
   ```

---

## Phase 3: Review and open PR (sub-agent)

*This sub-agent receives `/task/implement-result.md` and `/task/plan.md`. It does not re-implement anything — its job is to verify and ship.*

1. **Read plan.md and implement-result.md**. If STATUS is failure, stop immediately and report — do not push broken code.

2. **Review the diff** against the plan:
   ```bash
   git diff main..HEAD
   ```
   Check: does the change match what plan.md described? Is the code style consistent? Is the new test meaningful? If something looks wrong, fix it and re-commit before continuing.

3. **Final verify pass** — run all three checks one more time on the committed state:
   - Build
   - Full test suite
   - Lint / type-check

   **Do not push if any check fails.** Fix, commit, re-run from step 3.

4. **Push and open PR**:
   ```bash
   git push -u origin HEAD
   ```
   Check for a PR template at `.github/PULL_REQUEST_TEMPLATE.md` and use it if present. Otherwise:
   ```bash
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
   The closing keyword (`Closes`, `Fixes`, or `Resolves`) must use the **full issue URL** — `#<number>` alone only works same-repo.

   Print the PR URL. Write it to `/task/pr.md`.

5. **CI and review loop** — repeat until merge-ready:

   a. **Request Copilot review** (once, after opening):
      ```bash
      gh pr edit <number> --add-reviewer copilot-pull-request-reviewer --repo <owner>/<repo>
      ```

   b. **Wait for CI** — poll until all checks complete:
      ```bash
      gh pr checks <number> --repo <owner>/<repo>
      ```
      If a check fails: read the failure log, fix locally, run the verify pass (step 3), commit, push.

   c. **Rebase if main has advanced**:
      ```bash
      git fetch origin main && git rebase main
      git push origin HEAD --force-with-lease
      ```

   d. **Address review comments** — fetch all and go through each:
      ```bash
      gh pr view <number> --repo <owner>/<repo> --json reviewComments,reviews,comments
      gh api repos/<owner>/<repo>/pulls/<number>/comments
      ```
      For each: accept and fix, or note why it does not apply. Never silently skip.

   e. **Re-request review from everyone who commented**:
      ```bash
      gh pr review <number> --repo <owner>/<repo> --comment \
        -b "All review comments addressed. Please re-review."

      reviewers=$(gh api repos/<owner>/<repo>/pulls/<number>/reviews \
        --jq '.[].user.login' | sort -u | tr '\n' ',' | sed 's/,$//')
      gh pr edit <number> --add-reviewer "$reviewers" --repo <owner>/<repo>
      ```

---

## Notes

- Always use `gh` for GitHub operations — bare `git push` often won't authenticate.
- When rebasing: use `--force-with-lease`; fall back to `--force` only if it fails.
- If the project has no test command in docs, infer it from the CI config.
- Print the PR URL after each push.
