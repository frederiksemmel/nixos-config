# nixos-config

## Worktrees

Every change happens in its own worktree, because several Claude sessions work on
this repo at once. If the working directory is not under `.claude/worktrees/`,
call EnterWorktree before the first edit.

Landing a change, without waiting to be asked:

1. Commit on the worktree branch.
2. `git push origin HEAD:main`. If main moved, `git fetch` and
   `git rebase origin/main` first, then push again.
3. ExitWorktree with `keep` (git in the main checkout is blocked while inside a
   worktree), then `git pull --ff-only` in the main checkout.
4. On fstux, run one `nh os switch .` in the main checkout; it needs no
   password. If it fails, fix forward in a new worktree. On another host, skip
   this step.
5. Remove the worktree with `git worktree remove` and delete its branch, then
   tell the user it is pulled and switched (on another host: pushed, ready to
   switch on fstux).

No PRs or long-lived branches: this is a single-author config with no review step.
