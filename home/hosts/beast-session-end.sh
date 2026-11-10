# Ends the Claude session this runs in, for good: its process, its worktree and
# branch, the terminals attached to it, its shpool session and its beast group.
# Refuses while the worktree holds uncommitted changes or commits on no remote,
# unless --force. The teardown runs detached and starts once the session's turn
# has ended, so Claude can still reply.

usage="usage: session-end [--force], inside a Claude Code session"

if [[ ${1:-} == --teardown ]]; then
  shift
  pid=$1 job=$2 worktree=$3 repo=$4 branch=$5 shpool_name=$6 force=$7 session=$8
  for _ in $(seq 120); do
    status=$(jq -r .status "$HOME/.claude/sessions/$pid.json" 2>/dev/null || echo gone)
    [[ $status == busy ]] || break
    sleep 1
  done
  sleep 2
  if [[ -n $job ]]; then
    claude stop "$job" || true
  else
    kill "$pid" 2>/dev/null || true
  fi
  for _ in $(seq 50); do
    [[ -d /proc/$pid ]] || break
    sleep 0.2
  done
  if [[ -d /proc/$pid ]]; then
    kill -9 "$pid"
  fi
  if [[ -n $job ]]; then
    pkill -f "^claude attach $job\$" || true
    claude rm "$job" || true
  fi
  if [[ -n $worktree ]]; then
    if [[ -d $worktree ]]; then
      git -C "$repo" worktree unlock "$worktree" 2>/dev/null || true
      if [[ $force == yes ]]; then
        git -C "$repo" worktree remove --force "$worktree"
      else
        git -C "$repo" worktree remove "$worktree"
      fi
    fi
    if [[ -n $branch ]] && git -C "$repo" rev-parse --verify --quiet "refs/heads/$branch" >/dev/null; then
      git -C "$repo" branch -D "$branch"
    fi
  fi
  rm -f "$HOME/.local/state/beast-session-groups/$session"
  if [[ -n $shpool_name ]]; then
    shpool kill "$shpool_name"
  fi
  exit
fi

force=no
case ${1:-} in
  "") ;;
  --force) force=yes ;;
  *)
    echo "$usage" >&2
    exit 2
    ;;
esac
session=${CLAUDE_CODE_SESSION_ID:-}
if [[ -z $session ]]; then
  echo "$usage" >&2
  exit 2
fi

entry=""
for file in "$HOME"/.claude/sessions/*.json; do
  candidate=$(jq -c --arg s "$session" 'select(.sessionId == $s)' "$file" 2>/dev/null || true)
  if [[ -n $candidate && -d /proc/$(jq -r .pid <<<"$candidate") ]]; then
    entry=$candidate
    break
  fi
done
if [[ -z $entry ]]; then
  echo "no running process of session $session in ~/.claude/sessions" >&2
  exit 1
fi
pid=$(jq -r .pid <<<"$entry")
cwd=$(jq -r .cwd <<<"$entry")
job=$(jq -r 'if .kind == "bg" then .jobId // "" else "" end' <<<"$entry")

worktree="" repo="" branch=""
top=$(git -C "$cwd" rev-parse --show-toplevel 2>/dev/null || true)
if [[ $top == */.claude/worktrees/* ]]; then
  worktree=$top
  repo=${top%%/.claude/worktrees/*}
  branch=$(git -C "$top" branch --show-current)
  changes=$(git -C "$top" status --porcelain)
  unpushed=$(git -C "$top" log --oneline HEAD --not --remotes)
  if [[ $force == no && (-n $changes || -n $unpushed) ]]; then
    echo "Not ended: $top holds work that would be lost."
    [[ -z $changes ]] || printf 'Uncommitted:\n%s\n' "$changes"
    [[ -z $unpushed ]] || printf 'Commits on no remote:\n%s\n' "$unpushed"
    echo "Push or commit it, or run session-end --force to discard it."
    exit 1
  fi
fi

shpool_name=$(tr '\0' '\n' <"/proc/$pid/environ" | sed -n 's/^SHPOOL_SESSION_NAME=//p')

log_dir="$HOME/.local/state/session-end"
mkdir -p "$log_dir"
setsid -f "$0" --teardown "$pid" "$job" "$worktree" "$repo" "$branch" "$shpool_name" "$force" "$session" \
  </dev/null >"$log_dir/$session.log" 2>&1

echo "This session ends once this turn does."
[[ -z $worktree ]] || echo "Then $worktree${branch:+ and branch $branch} are removed."
[[ -z $job ]] || echo "Background session $job is stopped and deleted, attached terminals close."
[[ -z $shpool_name ]] || echo "Then shpool session $shpool_name is killed."
