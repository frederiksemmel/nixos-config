repo=$HOME/repos/nixos-config
state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-deploy-watch
runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
mkdir -p "$state"

# Says a problem once, not every 20 seconds while it lasts.
say() {
  local key=$1 text=$2
  if [[ $(cat "$state/$key.said" 2>/dev/null || true) != "$text" ]]; then
    echo "$text"
    printf '%s' "$text" >"$state/$key.said"
  fi
}

if ! fetched=$(git -C "$repo" fetch --quiet origin main 2>&1); then
  say fetch "fetching main failed: $fetched"
else
  say fetch ""
fi
main=$(git -C "$repo" rev-parse origin/main)
commits=$state/main-$main.json
if [[ ! -s $commits ]]; then
  rm -f "$state"/main-*.json
  git -C "$repo" log -100 --format='%x1e%H%x1f%ct%x1f%s%x1f%b%x1f' --name-only origin/main |
    jq -Rs '{commits: [split("\u001e")[1:][] | split("\u001f") | {
        sha: .[0],
        at: (.[1] | tonumber),
        subject: .[2],
        reports: ([(.[2] + "\n" + .[3]) | scan("[0-9]{8}-[0-9]{6}-[0-9a-f]{4}")] | unique),
        tv: (.[4] | split("\n") | any(test("^(semmeltv/|secrets/|flake\\.(nix|lock)$)")))
      }]}' >"$commits.new"
  mv "$commits.new" "$commits"
fi

deploy() {
  local host=$1 checkout log
  if ! semmeltv-feedback "$host" deploy running >/dev/null; then
    echo "$host: a deploy was asked for, but telling the TV it runs failed"
    return
  fi
  checkout=$(mktemp -d "$runtime/semmeltv-deploy-watch.XXXXXX")
  log=$state/$host.log
  git -C "$repo" worktree add --quiet --detach "$checkout" origin/main
  echo "$host: deploying origin/main ${main:0:7}, asked on its Feedback page"
  if semmeltv-deploy --now "$host" "$checkout" >"$log" 2>&1; then
    cat "$log"
    for _ in $(seq 12); do
      semmeltv-feedback "$host" deploy finished >/dev/null 2>&1 && break
      sleep 10
    done
  else
    cat "$log"
    semmeltv-feedback "$host" deploy failed "$(grep -v '^\s*$' "$log" | tail -n 4 | cut -c1-300)" >/dev/null || true
  fi
  git -C "$repo" worktree remove --force "$checkout" || true
}

# A deploy semmeltv-deploy held because someone watched: once nobody does,
# deploy main. A failed one is not tried again every 20 seconds: it shows on
# the Feedback page, where Deploy now tries again.
deploy_held() {
  local host=$1 checkout log
  checkout=$(mktemp -d "$runtime/semmeltv-deploy-watch.XXXXXX")
  log=$state/$host.log
  git -C "$repo" worktree add --quiet --detach "$checkout" origin/main
  echo "$host: nobody watches, deploying origin/main ${main:0:7}, held since $(cat "$held")"
  if semmeltv-deploy "$host" "$checkout" >"$log" 2>&1; then
    cat "$log"
  else
    cat "$log"
    rm -f "$held"
    semmeltv-feedback "$host" deploy unwait >/dev/null || true
    semmeltv-feedback "$host" deploy failed "$(grep -v '^\s*$' "$log" | tail -n 4 | cut -c1-300)" >/dev/null || true
  fi
  git -C "$repo" worktree remove --force "$checkout" || true
}

host=semmeltv5
held=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-deploy/$host.held
if ! answer=$(semmeltv-feedback "$host" deploy state 2>&1); then
  say "$host" "$host: asking it about deploys failed: $(tail -n 1 <<<"$answer")"
  exit 0
fi
if ! jq -e --arg main "$main" '.main == $main' <<<"$answer" >/dev/null &&
  ! answer=$(semmeltv-feedback "$host" deploy main "$commits" 2>&1); then
  say "$host" "$host: telling it main failed: $(tail -n 1 <<<"$answer")"
  exit 0
fi
say "$host" ""
if jq -e '.asked != null and .run.state != "running"' <<<"$answer" >/dev/null; then
  deploy "$host"
elif jq -e '.run.state == "running"' <<<"$answer" >/dev/null; then
  if jq -e '.revision == .main' <<<"$answer" >/dev/null; then
    semmeltv-feedback "$host" deploy finished >/dev/null
  else
    semmeltv-feedback "$host" deploy failed "Beast stopped before the deploy finished. Deploy now tries again." >/dev/null
  fi
elif [[ -e $held ]]; then
  if jq -e '.pending == []' <<<"$answer" >/dev/null; then
    rm -f "$held"
    semmeltv-feedback "$host" deploy unwait >/dev/null || true
  elif ! watching=$(semmeltv-feedback "$host" deploy watching 2>&1); then
    say "$host" "$host: asking who watches failed: $(tail -n 1 <<<"$watching")"
  elif jq -e '.watching == []' <<<"$watching" >/dev/null; then
    deploy_held "$host"
  elif jq -e '.waiting == null' <<<"$answer" >/dev/null; then
    semmeltv-feedback "$host" deploy wait >/dev/null || true
  fi
fi
