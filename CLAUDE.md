# MM-06

Browser-based emulation of the Roland TR-606. All seven voices are **fully analog / synthesized** — there are no samples anywhere. Specs live in `./documents` (architecture, audio-engine spec, calibration & testing strategy).

## Ergo feature plans

This project uses ergo to schedule and document the work the agent carries out — run `ergo --help` for the CLI. The task graph lives in `.ergo/` at the repo root. Create tasks and epics only when the human asks; ad hoc work needs no task.

### Authoring tasks
- **Don't number tasks or epics.** Plans change, and numbers stop being consecutive.
- **One task is one commit** — a few hundred lines at most — and must include automated and/or manual testing.
- **Only register dependencies that actually exist.** Don't assume related tasks depend on each other.

### Completing tasks
- **Commit as soon as a task is done** — don't wait for the human to approve. **Never push**; the human may want to `git reset`.
- Mark the task `done` in ergo when its commit lands.
- When completing a whole epic, don't stop for confirmation between tasks — commit each one as you go; the human reviews the epic as a whole.

### Working in parallel (multiple agents at once)
- Each parallel agent works in its **own git worktree on its own branch** — two worktrees can't share the `main` checkout, and separate trees keep concurrent edits from colliding. Single-agent work commits straight to `main`.
- Only parallelize tasks with **no dependency edge and minimal file overlap**.
- Keep ergo bookkeeping in the **main worktree only**. `.ergo/` is a tracked file, so it diverges per worktree: the orchestrator claims a task before spawning its agent and marks it `done` when the agent reports its commit — subagents never touch `.ergo/`.
- Each agent commits its one task to its branch (no push). When the agents finish, the **orchestrator** reviews each branch, merges them into `main` (resolving the trivial overlaps, e.g. `mod` lines), and removes the spent worktrees — the human does **not** merge by hand. Still no push, so the human can `git reset` if they dislike the result.
