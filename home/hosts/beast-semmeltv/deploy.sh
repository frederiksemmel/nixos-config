usage() {
  cat >&2 <<'EOF'
usage: semmeltv-deploy [--now] semmeltv5 [FLAKE]

Deploys FLAKE (default: the current directory) to semmeltv5 with deploy-rs,
building on nixbuild.net. Every deploy on beast takes the same lock, so two
sessions never deploy at once; a second one waits and says who holds it.

A clean checkout whose HEAD is on origin/main deploys the newest origin/main
instead, so a session that deploys after another never rolls back the other's
work. A TV that already runs that commit is left alone. A checkout with
uncommitted changes or commits not on origin/main deploys as it is.

While anyone watches, on the TV or a phone, it deploys nothing: it lists who
watches, marks the deploy as held, and the Feedback page says so.
semmeltv-deploy-watch deploys the newest origin/main once nobody watches.
--now deploys even then, as Deploy now on the Feedback page does.
EOF
  exit 2
}

now=0
if [[ ${1:-} == --now ]]; then
  now=1
  shift
fi
(($# >= 1 && $# <= 2)) || usage
host=$1
flake=$(realpath "${2:-.}")
[[ $host == semmeltv5 ]] || usage

runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
lock=$runtime/semmeltv-deploy.lock
holder=$runtime/semmeltv-deploy.holder
builders='ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel'
session=${CLAUDE_CODE_SESSION_ID:-}

state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-deploy
held=$state/$host.held
mkdir -p "$state"

tv() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" "$@"
}

not_held() {
  rm -f "$held"
  semmeltv-feedback "$host" deploy unwait >/dev/null 2>&1 || true
}

main=""
cleanup() {
  if [[ -n $main ]]; then
    git -C "$flake" worktree remove --force "$main" || true
  fi
  rm -f "$holder"
}

exec 9>"$lock"
if ! flock --nonblock 9; then
  echo "semmeltv-deploy: waiting for the deploy lock, held by: $(cat "$holder" 2>/dev/null || echo unknown)" >&2
  flock 9
fi
trap cleanup EXIT
echo "$host from $flake, session ${session:0:8}, pid $$, since $(date '+%F %T')" >"$holder"

revision=unknown
target=$flake
if git -C "$flake" rev-parse --is-inside-work-tree >/dev/null 2>&1 &&
  [[ -z $(git -C "$flake" status --porcelain --untracked-files=no) ]]; then
  git -C "$flake" fetch --quiet origin main
  revision=$(git -C "$flake" rev-parse HEAD)
  if git -C "$flake" merge-base --is-ancestor HEAD origin/main; then
    newest=$(git -C "$flake" rev-parse origin/main)
    if [[ $newest != "$revision" ]]; then
      main=$(mktemp -d "$runtime/semmeltv-deploy-main.XXXXXX")
      git -C "$flake" worktree add --quiet --detach "$main" "$newest"
      target=$main/$(git -C "$flake" rev-parse --show-prefix)
      target=${target%/}
      echo "semmeltv-deploy: deploying origin/main ${newest:0:7}, which contains this checkout's ${revision:0:7}" >&2
      revision=$newest
    fi
  else
    missing=$(git -C "$flake" log --oneline HEAD..origin/main)
    if [[ -n $missing ]]; then
      printf 'semmeltv-deploy: HEAD has commits not on origin/main, so it deploys as it is. It lacks these commits of origin/main until main is deployed again:\n%s\n' "$missing" >&2
    fi
  fi
  if [[ $(tv cat /etc/semmeltv/revision 2>/dev/null || true) == "$revision" ]]; then
    echo "semmeltv-deploy: $host already runs ${revision:0:7}, nothing to deploy" >&2
    not_held
    exit 0
  fi
fi

if ((!now)); then
  if ! answer=$(semmeltv-feedback "$host" deploy watching 2>&1); then
    printf 'semmeltv-deploy: asking %s who watches failed, so it deploys nothing:\n%s\n' "$host" "$answer" >&2
    exit 1
  fi
  watching=$(jq -r '.watching[] | "  \(.person) watches \(.title) on \(.device)\(if .paused then ", paused" else "" end)"' <<<"$answer")
  if [[ -n $watching ]]; then
    echo "${revision:0:7} held $(date '+%F %T'), session ${session:0:8}" >"$held"
    semmeltv-feedback "$host" deploy wait >/dev/null || echo "semmeltv-deploy: telling the Feedback page the deploy is held failed" >&2
    printf 'semmeltv-deploy: held, because someone watches:\n%s\nsemmeltv-deploy-watch deploys the newest origin/main once nobody watches; Deploy now on the Feedback page deploys at once.\n' "$watching" >&2
    exit 0
  fi
fi

echo "$host ${revision:0:7} from $flake, session ${session:0:8}, pid $$, since $(date '+%F %T')" >"$holder"
echo "semmeltv-deploy: deploying ${revision:0:7} to $host" >&2

nix run "$target#deploy-rs" -- "$target#$host" --skip-checks -- --builders "$builders" --max-jobs 0
not_held
echo "semmeltv-deploy: $host now runs $(tv cat /etc/semmeltv/revision 2>/dev/null || echo 'an unknown revision')" >&2
