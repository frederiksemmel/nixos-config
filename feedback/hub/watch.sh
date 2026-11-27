state=${XDG_STATE_HOME:-$HOME/.local/state}/feedback-hub
mkdir -p "$state"
touch "$state/handed"
common=$(jq -r .common "$CONFIG")
sessions=$(claude agents --json)
now=$(date +%s)

busy() {
  jq -e --arg prefix "feedback-$1-" \
    'any(.[]; ((.name // "") | startswith($prefix)) and .state != "done")' <<<"$sessions" >/dev/null
}

legacy() {
  jq -e --arg name "feedback-$1" 'any(.[]; .name == $name and .state != "done")' <<<"$sessions" >/dev/null
}

while read -r project; do
  name=$(jq -r .name <<<"$project")
  adapter=$(jq -r .adapter <<<"$project")
  repo=$(jq -r .repo <<<"$project")
  prompt=$(jq -r .prompt <<<"$project")
  quiet=$(jq -r .quiet <<<"$project")
  settings=$(jq -r '.settings // empty' <<<"$project")
  if busy "$name"; then
    continue
  fi
  if ! open=$("$adapter" open 2>"$state/$name.error"); then
    echo "$name: $(tail -n 1 "$state/$name.error")"
    continue
  fi
  ids=()
  keys=()
  newest=0
  while read -r report; do
    [[ -n $report ]] || continue
    id=$(jq -r .id <<<"$report")
    marked=$(jq -r '.marked_at // 0 | floor' <<<"$report")
    status=$(jq -r .state <<<"$report")
    if legacy "$id"; then
      continue
    fi
    key="$name $id $marked $status"
    if (($(grep -cxF "$key" "$state/handed" || true) >= 2)); then
      continue
    fi
    ids+=("$id")
    keys+=("$key")
    ((marked > newest)) && newest=$marked
  done <<<"$open"
  ((${#ids[@]})) || continue
  if ((now - newest < quiet)); then
    echo "$name: ${#ids[@]} open, waiting until the newest is ${quiet}s old"
    continue
  fi
  brief=$(
    sed -e "s|{{adapter}}|$(basename "$adapter")|g" -e "s|{{project}}|$name|g" "$common" "$prompt"
    printf '\n# Your reports\n\n%s report(s) for %s: %s\n' "${#ids[@]}" "$name" "${ids[*]}"
    for id in "${ids[@]}"; do
      printf '\n'
      "$adapter" brief "$id" || printf '## %s\n\n(%s brief %s failed: read it with %s show %s)\n' "$id" "$adapter" "$id" "$adapter" "$id"
    done
  )
  session="feedback-$name-$(date +%m%d-%H%M)"
  args=(--bg -n "$session" -w "$session" --permission-mode auto --strict-mcp-config)
  [[ -z $settings ]] || args+=(--settings "$settings")
  if (cd "$repo" && claude "${args[@]}" "$brief") </dev/null; then
    printf '%s\n' "${keys[@]}" >>"$state/handed"
    echo "$name: started $session for ${ids[*]}"
  else
    echo "$name: starting a session for ${ids[*]} failed"
  fi
done < <(jq -c '.projects[]' "$CONFIG")
