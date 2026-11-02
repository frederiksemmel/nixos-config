# nixos-config

## Worktrees

Every change happens in its own worktree, because several Claude sessions work on
this repo at once. If the working directory is not under `.claude/worktrees/`,
call EnterWorktree before the first edit.

Landing a change, without waiting to be asked:

1. Validate it with one `nh os build .` from the worktree at the end of the task.
2. Commit on the worktree branch.
3. `git push origin HEAD:main`. If main moved, `git fetch` and
   `git rebase origin/main` first, then push again.
4. Tell the user it is on origin/main. Worktree sessions cannot touch the main
   checkout, and `nh os switch` builds from there, so the user runs
   `git pull --ff-only` in it before switching.

No PRs or long-lived branches: this is a single-author config with no review step.
