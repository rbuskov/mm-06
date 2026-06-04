---
description: Commit the current task's changes to main following the MM-09 workflow conventions
argument-hint: [optional note about the task / what to emphasize]
allowed-tools: Bash(git status:*), Bash(git diff:*), Bash(git log:*), Bash(git add:*), Bash(git commit:*), Bash(git branch:*)
---

## Context

- Current branch: !`git branch --show-current`
- Working tree status: !`git status --short`
- Change size vs HEAD: !`git diff HEAD --stat`
- Unstaged changes: !`git diff`
- Already-staged changes: !`git diff --cached`
- Recent commits (match this message style): !`git log -3 --format='--- %h%n%s%n%n%b'`

Note from me about this task (may be empty): $ARGUMENTS

## Your task

Create a single commit for the current task, following the MM-09 conventions. Do the whole thing yourself — don't stop to ask for approval.

### 1. Sanity-check the scope

The changes above should map onto **one self-contained Ergo task** — one logical unit of work, not several batched together and not half of one. Glance at the diff and the file list:

- If it looks like more than one task got mixed in, say so and stage/commit only the files belonging to the task at hand, leaving the rest for a separate commit.
- Tasks are sized so the diff lands roughly in the **50–250 line** range. If the change is well past ~250 lines, mention it briefly — it's a hint the task probably should have been split — but still go ahead and commit it. Don't block on it.

### 2. Stage the changes

Stage the files that belong to this task (`git add` the specific paths, or `git add -A` if everything in the tree is part of this one task). Don't stage unrelated edits.

### 3. Write the commit message

This is the part that matters most — the history should be genuinely useful to someone reading it later (including future you).

- **Summary line:** one concise line in the imperative mood, matching the style of the recent commits shown above.
- **Body:** roughly **20–40 lines** explaining the commit. Cover *what* changed and, more importantly, *why* — the reasoning behind the approach, and any notable decisions or trade-offs you made. Wrap the body at ~72 columns. Don't pad it to hit the line count; if a change is genuinely small the body can be shorter, but for a normal task-sized commit aim for that range.
- If I gave a note above, fold it in — it tells you what to emphasize or which task this is.
- Don't add boilerplate or a Claude/co-author trailer unless the existing log shows that's the house style.

Write the message via a file or a heredoc rather than a pile of `-m` flags, so the body keeps its formatting.

### 4. Commit to main

This project commits **directly to `main`** — no feature branches. Confirm you're on `main` (see the branch above); if you're not, that's unexpected for this workflow, so flag it before committing.

Commit, then show me the resulting `git log -1 --stat` so I can see what landed.
