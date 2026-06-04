# MM-06

Browser-based emulation of the Roland TR-606. All seven voices are **fully analog / synthesized** — there are no samples anywhere. Look in the ./documents folder for
specifications (architecture, audio-engine spec, calibration & testing strategy).

## Ergo feature plans
This project uses ergo feature plans to schedule and document tasks and epics to be implemented by the agent. This allows the agent to plan and carry out larger chunks of work.

For ergo tasks and epics, the following rules apply:

**Don't number tasks and epics:** Plans change, and when they do numbers wil no longer be consecutive.
**One task is one git commit:** Few hundred lines of code at most. All tasks must include automated and/or manual testing.
**Don't commit changes after a task/epic is completed:** The human will verify and perform the commit.
**Only register dependencies where thy actually exist:** Don't assume that related tasks or epics have dependencies, only register dependencies that are actually needed.
**When the human asks you to complete an epic, don't stop for confirmation for each task:** Just go ahead and commit tasks along the way, the human will review the epic as a whole. Do not push commits when completing epics, the human may want to perform a git reset.

Note that the human will also ask for ad hoc work to be carried out from time to time. This does not require an ergo task or epic, and can be done at any time. Only create ergo tasks and epics when specifically requested by the human.
