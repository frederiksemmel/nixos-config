usage() {
  cat >&2 <<'EOF'
usage: semmeltv-deploy semmeltv5|semmeltv3 [FLAKE]

Deploys FLAKE (default: the current directory) to one TV with deploy-rs,
building on nixbuild.net. Every deploy on beast takes the same lock, so two
sessions never deploy at once; a second one waits and says who holds it.
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
revision=$(git -C "$flake" rev-parse --short HEAD 2>/dev/null || echo unknown)
session=${CLAUDE_CODE_SESSION_ID:-}

exec 9>"$lock"
if ! flock --nonblock 9; then
  echo "semmeltv-deploy: waiting for the deploy lock, held by: $(cat "$holder" 2>/dev/null || echo unknown)" >&2
  flock 9
fi
echo "$host $revision from $flake, session ${session:0:8}, pid $$, since $(date '+%F %T')" >"$holder"
trap 'rm -f "$holder"' EXIT
echo "semmeltv-deploy: deploying $revision to $host" >&2

if [[ $host == semmeltv5 ]]; then
  nix run "$flake#deploy-rs" -- "$flake#semmeltv5" --skip-checks -- --builders "$builders" --max-jobs 0
  exit
fi

state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-deploy
mkdir -p "$state"
ssh_opts=(-o "UserKnownHostsFile=$state/semmeltv3_known" -o StrictHostKeyChecking=accept-new -o BatchMode=yes -J frederik@semmeltv5 -i "$HOME/.ssh/id_semmeltv")
pi3() {
  ssh "${ssh_opts[@]}" frederik@192.168.1.173 "$@"
}

restart() {
  pi3 sudo systemctl start semmeltv-mount cage-tty1 || echo "semmeltv-deploy: starting cage and the mount on semmeltv3 failed" >&2
  rm -f "$holder"
}
trap restart EXIT

pi3 sudo systemctl stop cage-tty1 semmeltv-mount
for _ in $(seq 60); do
  [[ $(pi3 systemctl is-active user@1001 || true) == active ]] || break
  sleep 1
done
nix run "$flake#deploy-rs" -- "$flake#semmeltv3" --skip-checks \
  --hostname 192.168.1.173 --magic-rollback false --ssh-opts "${ssh_opts[*]}" \
  -- --builders "$builders" --max-jobs 0
