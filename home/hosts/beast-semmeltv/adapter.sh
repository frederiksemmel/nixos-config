# The feedback framework's adapter for SemmelTV (feedback/README.md): `open`,
# `brief` and `deploy` here, everything else is semmeltv-feedback's own.
host=semmeltv5

case ${1:-} in
  open)
    for state in new reopened; do
      semmeltv-feedback "$host" list "$state"
    done | jq -c '{id, state: .status.state,
      marked_at: (.marked_at // (.when | sub(" UTC$"; "Z") | sub(" "; "T") | fromdateiso8601))}'
    ;;
  brief)
    report=$(semmeltv-feedback "$host" show "$2")
    jq -r '
      "## Report \(.id) (\(.status.state // "new"))\n",
      "From \(.person.name // "someone") on \(.device // "?"), page \(.page // "?"), \(.at | todate).\n",
      "```json",
      ({message, screenshot, tv_screenshot,
        client: (.client | {agent, device_mode, viewport, url}),
        marks: (.client.feedback_mode.items // [] | map({n, comment, what, page, kind, target, rect, point, inside})),
        tv: {revision: .tv.revision, problems: .tv.problems}} | tojson),
      "```",
      (if ((.comments // []) | length) > 0 then "\nThread:\n", ((.comments // [])[] | "- \(.at | todate) \(if .author.kind == "claude" then "Claude" else .author.name end): \(.text)") else empty end)' <<<"$report"
    ;;
  deploy)
    semmeltv-deploy "$host"
    ;;
  *)
    exec semmeltv-feedback "$host" "$@"
    ;;
esac
