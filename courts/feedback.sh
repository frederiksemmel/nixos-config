usage() {
  cat >&2 <<'EOF'
usage: feedback-courts open
       feedback-courts list [new|reopened|needs_review|done]
       feedback-courts brief ID
       feedback-courts show ID
       feedback-courts photo ID FILE
       feedback-courts comment ID TEXT
       feedback-courts mark ID fixed COMMIT WHAT
       feedback-courts mark ID no-change WHY
       feedback-courts mark ID needs-review WHY DECIDE [--ask QUESTION (--option TEXT | --recommend TEXT)...]...
       feedback-courts deploy

The feedback framework's adapter for Courts (feedback/README.md). It reaches
the app's API on semmeltv5 over ssh with the app's own key, which never leaves
the Pi. A comment from here is signed Claude and never reopens a report.
EOF
  exit 2
}

(($# >= 1)) || usage
command=$1
shift

remote='
set -eu
header=$(mktemp)
trap "rm -f $header" EXIT
printf "Authorization: Bearer %s\n" "$(cat /var/lib/courts/key)" > "$header"
if [ -n "$3" ]; then
  printf %s "$3" | base64 -d | curl --silent --show-error --fail-with-body -X "$1" -H @"$header" -H "content-type: application/json" --data-binary @- "http://127.0.0.1:$4$2"
else
  curl --silent --show-error --fail-with-body -H @"$header" "http://127.0.0.1:$4$2"
fi
'

api() {
  local method=$1 path=$2 body=${3:-}
  local encoded=""
  [[ -z $body ]] || encoded=$(printf %s "$body" | base64 -w0)
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$COURTS_HOST" \
    "sudo bash -c $(printf %q "$remote") bash $(printf %q "$method") $(printf %q "$path") $(printf %q "$encoded") $(printf %q "$COURTS_PORT")"
}

ok() {
  jq -e '.ok' >/dev/null || {
    echo "the app said no" >&2
    exit 1
  }
}

mark() {
  api POST "/api/feedback/$1/status" "$2" | ok
}

case $command in
  open)
    api GET "/api/feedback" | jq -c '.reports[] | select(.state == "new" or .state == "reopened") | {id, state, marked_at, page, items: (.items | length)}'
    ;;
  list)
    query=""
    (($# == 0)) || query="?state=$1"
    api GET "/api/feedback$query" | jq -c '.reports[] | {id, state, outcome, page, at, items: (.items | length), answer}'
    ;;
  show)
    (($# == 1)) || usage
    api GET "/api/feedback/$1" | jq 'if .batch.photo then .batch.photo = "(a photo: feedback-courts photo ID FILE)" else . end'
    ;;
  photo)
    (($# == 2)) || usage
    api GET "/api/feedback/$1" | jq -r '.batch.photo // empty' | sed 's/^data:[^,]*,//' | base64 -d >"$2"
    echo "$2"
    ;;
  brief)
    (($# == 1)) || usage
    api GET "/api/feedback/$1" | jq -r '
      def device: if .batch.viewport.coarse then "a phone" else "a laptop" end;
      "## Report \(.id) (\(.state))\n",
      "Sent \(.at | todate) from \(.page) on \(device), \(.batch.viewport.width)×\(.batch.viewport.height) CSS px.",
      (if (.batch.note // "") != "" then "\nNote for the whole batch: \(.batch.note)" else empty end),
      (if .batch.photo then "\nA photo is attached: `feedback-courts photo \(.id) FILE`, then Read it." else empty end),
      "\nComments:\n",
      (.batch.items[] | "\(.n). **\(.comment)**\n   on \(.what) (\(.kind)) at \(.page)\n   ```json\n   \({target, rect, point, inside} | tojson)\n   ```"),
      (if (.thread | length) > 0 then "\nThread:\n", (.thread[] | "- \(.at | todate) \(if .by == "claude" then "Claude" else "Frederik" end): \(.text)") else empty end),
      (if .answer then "\nEarlier answer: \(.answer)" else empty end)'
    ;;
  comment)
    (($# == 2)) || usage
    api POST "/api/feedback/$1/comments" "$(jq -n --arg text "$2" '{text: $text, claude: true}')" | ok
    ;;
  mark)
    (($# >= 2)) || usage
    id=$1
    kind=$2
    shift 2
    case $kind in
      fixed)
        (($# == 2)) || usage
        mark "$id" "$(jq -n --arg commit "$1" --arg what "$2" '{state: "done", outcome: "fixed", commit: $commit, answer: $what}')"
        ;;
      no-change)
        (($# == 1)) || usage
        mark "$id" "$(jq -n --arg why "$1" '{state: "done", outcome: "no_change", answer: $why}')"
        ;;
      needs-review)
        (($# >= 2)) || usage
        text="$1"$'\n\n'"To decide: $2"
        shift 2
        letters=(a b c d e f g h)
        index=0
        while (($# >= 2)); do
          case $1 in
            --ask)
              text+=$'\n\n'"$2"
              index=0
              ;;
            --recommend)
              text+=$'\n'"${letters[index]}) $2 (recommended)"
              index=$((index + 1))
              ;;
            --option)
              text+=$'\n'"${letters[index]}) $2"
              index=$((index + 1))
              ;;
            *) usage ;;
          esac
          shift 2
        done
        (($# == 0)) || usage
        mark "$id" "$(jq -n --arg answer "$text" '{state: "needs_review", answer: $answer}')"
        ;;
      *) usage ;;
    esac
    ;;
  deploy)
    semmeltv-deploy "$COURTS_HOST"
    ;;
  *) usage ;;
esac
