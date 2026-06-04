---
description: Implement one or more ergo tasks (and anything they unblock), fanning out to parallel worktree-isolated agents
argument-hint: [task-id...] | [epic-id] | (blank = drain all implementable tasks)
allowed-tools: Agent, Bash(ergo:*), Bash(git worktree:*), Bash(git branch:*), Bash(git merge:*), Bash(git switch:*), Bash(git checkout:*), Bash(git status:*), Bash(git log:*), Bash(git diff:*), Bash(cargo test:*), Bash(hostname:*), Read, Grep, Glob
---

## Context

- Current branch: !`git branch --show-current`
- Working tree status: !`git status --short`
- Ready tasks: !`ergo --json list --ready`
- Full task graph: !`ergo --json list --all`
- Existing worktrees: !`git worktree list`

Target (task IDs, an epic ID, or blank to drain everything): $ARGUMENTS

## Your task

You are the **orchestrator**. Implement the requested ergo work, running independent tasks in parallel as fresh worktree-isolated subagents and integrating each onto `main` yourself. Follow `CLAUDE.md` → *Ergo feature plans*. All ergo bookkeeping happens here in the **main** worktree; subagents never touch `.ergo/`. Never push.

Crucially, this runs in **waves**: finishing one task can unblock others, and those must get implemented too. Keep going until the requested scope is fully drained — don't stop after the first batch.

### 1. Resolve the scope

The scope is the universe of tasks you're allowed to implement. From the argument:
- **blank** → every not-yet-`done` task in the graph (drain the whole plan).
- **an epic ID** → all of that epic's child tasks (implement the entire epic).
- **task IDs** (6-char) → exactly those, and nothing else. (If one depends on another in the list, that's fine — the wave loop will order them; if it depends on something *outside* the list that isn't `done`, it can never run — report it and move on.)

If the scope is empty, say so and stop.

### 2. Run in waves (the loop)

Repeat until done:

**a. Find this wave's ready set** — the in-scope tasks that are ready *right now* (state `todo`, all dependencies `done`): intersect `ergo --json list --ready` with the scope.

- If the ready set is **non-empty** → run a wave (steps b–f), then loop back to (a).
- If it's **empty**:
  - If every in-scope task is `done` → finished, go to step 3.
  - If in-scope tasks remain but none are ready → they're blocked by an `error`/`blocked` task or by out-of-scope unfinished work. **Stop** (no progress is possible) and report what's stuck and why.

**b. Decide parallel vs. serial.** Ready tasks have no unmet dependency, but can still collide on files. Read each body (`ergo --json show <id>`) and judge file overlap: independent + low-overlap run in parallel; heavy-overlap (same regions of the same file) run serially. A trivial overlap (e.g. both add a `mod` line to `lib.rs`) is fine — you resolve it at merge. State the wave's plan before spawning.

**c. Claim + spawn**, per task in the wave, from the main worktree:
1. Claim it: `ergo claim <id> --agent opus@<short-host>`.
2. **If the wave has only one task**, skip worktrees: spawn one fresh subagent that works in the **main** worktree and commits to `main` directly.
   **If the wave has 2+ tasks**, give each its own worktree + branch off current `main`:
   `git worktree add -b task/<id> .claude/worktrees/<id> HEAD`
3. Spawn a fresh subagent (the `Agent` tool — clean context, **no** `isolation`, since you manage the worktree). The prompt must be self-contained; the subagent knows nothing of this conversation. Include:
   - The absolute path it must treat as its repo root (the worktree path, or the main repo for a solo task), and the rule to edit **only** files under that path.
   - The full task body from `ergo --json show <id>` (goal / scope / testing / done-when).
   - Spec pointers to read (e.g. `documents/audio-engine-spec.md §3.1`, `documents/calibration-and-testing-strategy.md §3`).
   - Rules: implement the task; add the automated and/or manual tests it requires; run `cargo test` from inside that path and confirm green; then make **exactly one** commit (on `task/<id>` for a worktree task, or on `main` for a solo task) matching the message style in `git log`. **Do not push. Do not run `ergo` or touch `.ergo/`.**
   - Ask it to report: commit SHA, one-line summary, test results, blockers.

   Spawn a multi-task wave in one message (multiple `Agent` calls, `run_in_background` for 3+) so they run concurrently.

**d. Collect**, as each subagent returns:
- **Success** (committed, tests green) → `ergo set <id> '{"state":"done"}'`.
- **Failed/blocked** → `ergo set <id> '{"state":"error"}'` (or `blocked`), keep its worktree for inspection, report what went wrong. A failed task may leave its dependents permanently unready — that's expected; the loop will detect it in step (a).

**e. Integrate onto main** (skip for a solo task that already committed to `main`):
1. Merge each successful branch in dependency order: `git merge --no-ff task/<id>`.
2. Resolve trivial overlaps (e.g. the `mod` list in `lib.rs`) and complete the merge.
3. Run the **full** `cargo test` once on `main`. If it fails, fix the integration here, or revert the offending merge and re-open that task as `error`.
4. Remove spent worktrees/branches: `git worktree remove .claude/worktrees/<id>` then `git branch -d task/<id>`.

**f. Loop back to (a)** — completing this wave may have unblocked more in-scope tasks.

### 3. Report

Summarize across all waves: which tasks landed (with the commit SHAs now on `main`), which didn't and why, the final post-merge test result, and what (if anything) remains and its state (`ergo --json list --all`). Nothing was pushed, so note the human can `git reset` if unhappy.
