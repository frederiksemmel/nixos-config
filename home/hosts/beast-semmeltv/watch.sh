state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-feedback
mkdir -p "$state"
touch "$state/handed"

if systemctl --user is-active --quiet semmeltv-feedback-triage.service; then
  exit 0
fi

fresh=()
for host in semmeltv5 semmeltv3; do
  if ! reports=$(semmeltv-feedback "$host" list new 2>"$state/$host.error"); then
    echo "$host: $(tail -n 1 "$state/$host.error")"
    continue
  fi
  while read -r id marked; do
    [[ -n $id ]] || continue
    key="$host $id $marked"
    if (($(grep -cxF "$key" "$state/handed" || true) < 2)); then
      fresh+=("$key")
    fi
  done < <(jq -r '"\(.id) \(.marked_at // 0)"' <<<"$reports")
done

((${#fresh[@]})) || exit 0
printf '%s\n' "${fresh[@]}" | tee "$state/pending" >>"$state/handed"
echo "handing ${#fresh[@]} new report(s) to a triage run"
systemctl --user start --no-block semmeltv-feedback-triage.service
