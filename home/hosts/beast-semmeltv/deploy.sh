usage() {
  cat >&2 <<'EOF'
usage: semmeltv-deploy semmeltv5|semmeltv3 [FLAKE]

Deploys FLAKE (default: the current directory) to one TV with deploy-rs,
building on nixbuild.net. Every deploy on beast takes the same lock, so two
sessions never deploy at once; a second one waits and says who holds it.

A clean checkout whose HEAD is on origin/main deploys the newest origin/main
instead, so a session that deploys after another never rolls back the other's
work. A TV that already runs that commit is left alone. A checkout with
uncommitted changes or commits not on origin/main deploys as it is.

semmeltv3 is reached over the LAN through semmeltv5, and cage and the mount
are stopped first, because the Pi 3 fails a switch that restarts cage.
EOF
  exit 2
}

(($# >= 1 && $# <= 2)) || usage
host=$1
flake=$(realpath "${2:-.}")
case $host in
  semmeltv5 | semmeltv3) ;;
  *) usage ;;
esac

runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
lock=$runtime/semmeltv-deploy.lock
holder=$runtime/semmeltv-deploy.holder
builders='ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel'
session=${CLAUDE_CODE_SESSION_ID:-}

state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-deploy
mkdir -p "$state"
ssh_opts=(-o "UserKnownHostsFile=$state/semmeltv3_known" -o StrictHostKeyChecking=accept-new -o BatchMode=yes -J frederik@semmeltv5 -i "$HOME/.ssh/id_semmeltv")
pi3() {
  ssh "${ssh_opts[@]}" frederik@192.168.1.230 "$@"
}
tv() {
  if [[ $host == semmeltv5 ]]; then
    ssh -o BatchMode=yes -o ConnectTimeout=10 semmeltv5 "$@"
  else
    pi3 "$@"
  fi
}

main=""
pi3_stopped=0
cleanup() {
  if ((pi3_stopped)); then
    pi3 sudo systemctl start semmeltv-mount cage-tty1 || echo "semmeltv-deploy: starting cage and the mount on semmeltv3 failed" >&2
  fi
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
    exit 0
  fi
fi

echo "$host ${revision:0:7} from $flake, session ${session:0:8}, pid $$, since $(date '+%F %T')" >"$holder"
echo "semmeltv-deploy: deploying ${revision:0:7} to $host" >&2

if [[ $host == semmeltv5 ]]; then
  nix run "$target#deploy-rs" -- "$target#semmeltv5" --skip-checks -- --builders "$builders" --max-jobs 0
else
  pi3_stopped=1
  pi3 sudo systemctl stop cage-tty1 semmeltv-mount
  for _ in $(seq 60); do
    [[ $(pi3 systemctl is-active user@1001 || true) == active ]] || break
    sleep 1
  done
  nix run "$target#deploy-rs" -- "$target#semmeltv3" --skip-checks \
    --hostname 192.168.1.230 --magic-rollback false --ssh-opts "${ssh_opts[*]}" \
    -- --builders "$builders" --max-jobs 0
fi
echo "semmeltv-deploy: $host now runs $(tv cat /etc/semmeltv/revision 2>/dev/null || echo 'an unknown revision')" >&2
