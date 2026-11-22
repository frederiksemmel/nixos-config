state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-feedback
mkdir -p "$state"
touch "$state/handed"
repo=$HOME/repos/nixos-config
max=4

sessions=$(claude agents --json)
running=$(jq '[.[] | select((.name // "") | startswith("feedback-")) | select(.state != "done")] | length' <<<"$sessions")

start() {
  local host=$1 id=$2 kind=$3 report summary thread brief
  report=$(semmeltv-feedback "$host" show "$id") || return 1
  summary=$(jq '{host, id, when: (.at | todate), person: .person.name, page, device, message, screenshot, tv_screenshot, status,
    client: (.client | {agent, device_mode, viewport, url}), tv: {revision: .tv.revision, problems: .tv.problems}}' <<<"$report")
  thread=$(jq -r '(.comments // [])[] | "- \(.at | todate) \(if .author.kind == "claude" then "Claude" else .author.name end): \(.text)"' <<<"$report")
  brief=$(
    cat "$PROMPT"
    printf '\n# Your report\n\nHOST=%s ID=%s, %s.\n\n```json\n%s\n```\n\nThread:\n\n%s\n' \
      "$host" "$id" "$([[ $kind == reopened ]] && echo "reopened by a comment" || echo new)" \
      "$summary" "${thread:-(no comments yet)}"
  )
  cd "$repo"
  claude --bg -n "feedback-$id" -w "feedback-$id-$(date +%H%M%S)" \
    --permission-mode auto --settings "$SETTINGS" --strict-mcp-config "$brief"
}

host=semmeltv5
if ! reports=$(semmeltv-feedback "$host" list new 2>"$state/$host.error" && semmeltv-feedback "$host" list reopened 2>>"$state/$host.error"); then
  echo "$host: $(tail -n 1 "$state/$host.error")"
  exit 0
fi
while read -r id marked kind; do
  [[ -n $id ]] || continue
  if jq -e --arg name "feedback-$id" 'any(.[]; .name == $name)' <<<"$sessions" >/dev/null; then
    continue
  fi
  key="$host $id $marked $kind"
  if (($(grep -cxF "$key" "$state/handed" || true) >= 2)); then
    continue
  fi
  if ((running >= max)); then
    echo "$host $id waits: $running feedback sessions are running"
    continue
  fi
  if start "$host" "$id" "$kind" </dev/null; then
    echo "$key" >>"$state/handed"
    running=$((running + 1))
    echo "started feedback-$id for $host $id ($kind)"
  else
    echo "starting a session for $host $id failed"
  fi
done < <(jq -r '"\(.id) \(.marked_at // 0) \(.status.state)"' <<<"$reports")
