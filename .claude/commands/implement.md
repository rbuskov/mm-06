---
description: Implement one or more ergo tasks, fanning out to parallel worktree-isolated agents
argument-hint: [task-id...] | [epic-id] | (blank = all ready tasks)
allowed-tools: Agent, Bash(ergo:*), Bash(git worktree:*), Bash(git branch:*), Bash(git merge:*), Bash(git switch:*), Bash(git checkout:*), Bash(git status:*), Bash(git log:*), Bash(git diff:*), Bash(cargo test:*), Bash(hostname:*), Read, Grep, Glob
---

## Context

- Current branch: !`git branch --show-current`
- Working tree status: !`git status --short`
- Ready tasks: !`ergo --json list --ready`
- Full task graph: !`ergo --json list --all`
- Existing worktrees: !`git worktree list`

Target (task IDs, an epic ID, or blank for all ready): $ARGUMENTS

## Your task

You are the **orchestrator**. Implement the requested ergo task(s), running independent ones in parallel as worktree-isolated subagents, then integrate everything onto `main` yourself. Follow `CLAUDE.md` → *Ergo feature plans*. All ergo bookkeeping happens here in the **main** worktree; subagents never touch `.ergo/`. Never push.

### 1. Resolve the target set

From the argument:
- **blank** → every task in *Ready tasks* above.
- **task IDs** (6-char) → exactly those. Each must be ready (state `todo` with all deps `done`); skip and report any that aren't.
- **an epic ID** → its child tasks that are ready (`ergo --json show <epic>` for children).

If the set is empty, say so and stop.

### 2. Decide what runs in parallel

Ready tasks have no *unmet dependency*, but they can still collide on files. Read each task body (`ergo --json show <id>`) and judge file overlap:
- **Independent + low overlap** → run in parallel (separate worktrees).
- **Heavy overlap** (would edit the same regions of the same file) → run those serially, one worktree reused or one after another.
- A trivial overlap (e.g. both add a `mod` line to `lib.rs`) is fine to parallelize — you resolve it at merge.

State the batch plan before spawning: which tasks run in parallel, which serially, and why.

**Single task:** skip worktrees entirely — implement it directly here on `main`, run its tests, commit, mark it `done`. Done.

### 3. Claim + spawn (per parallel task)

For each task in the parallel batch, **in the main worktree**:

1. Claim it: `ergo claim <id> --agent opus@<short-host>`.
2. Create its worktree + branch off current `main`:
   `git worktree add -b task/<id> .claude/worktrees/<id> HEAD`
3. Spawn a fresh subagent (the `Task` tool — clean context, **no** `isolation`, since you made the worktree). Give it a self-contained prompt — it knows nothing of this conversation. Include:
   - The absolute worktree path `…/.claude/worktrees/<id>`, and the instruction to do **all** its work there (treat it as the repo root; never edit files outside it).
   - The full task body from `ergo --json show <id>` (goal / scope / testing / done-when).
   - Spec pointers it should read (e.g. `documents/audio-engine-spec.md §3.1`, `documents/calibration-and-testing-strategy.md §3`).
   - Rules: implement the task; add the automated and/or manual tests the task requires; run `cargo test` (from inside the worktree) and confirm green; then make **exactly one** commit on the current branch (`task/<id>`) following the message style in `git log`. **Do not push. Do not run `ergo` or touch `.ergo/`.**
   - Ask it to report back: the commit SHA, a one-line summary, test results, and any blockers.

Spawn the parallel batch in one message (multiple `Task` calls) so they run concurrently. Prefer `run_in_background` for batches of 3+.

### 4. Collect results

As each subagent returns:
- **Success** (committed, tests green) → `ergo set <id> '{"state":"done"}'`.
- **Failed / blocked** → `ergo set <id> '{"state":"error"}'` (keeps the claim) or `blocked`, and keep its worktree for inspection. Report what went wrong.

### 5. Integrate onto main

Once the batch is in, merge yourself — the human does **not**:
1. Merge each successful branch into `main` in dependency order: `git merge --no-ff task/<id>`.
2. Resolve trivial overlaps (e.g. a `mod` list in `lib.rs`) and complete the merge.
3. After all merges, run the **full** `cargo test` once on `main`. If it fails, fix the integration here (or revert the offending merge and re-open that task as `error`).
4. Remove spent worktrees and branches: `git worktree remove .claude/worktrees/<id>` then `git branch -d task/<id>`.

### 6. Report

Summarize: which tasks landed (with commit SHAs now on `main`), which didn't and why, the post-merge test result, and what's newly ready in the graph (`ergo --json list --ready`). Nothing was pushed, so note the human can `git reset` if unhappy.
