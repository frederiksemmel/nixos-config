usage() {
  cat >&2 <<'EOF'
usage: semmeltv-feedback HOST list [new|needs_review|done]
       semmeltv-feedback HOST show ID
       semmeltv-feedback HOST pictures ID DIR
       semmeltv-feedback HOST mark ID new
       semmeltv-feedback HOST mark ID needs-review WHY DECIDE
       semmeltv-feedback HOST mark ID fixed COMMIT WHAT
       semmeltv-feedback HOST mark ID no-change WHY

HOST is semmeltv5 or semmeltv3. Talks to the TV's API on 127.0.0.1 over ssh
with the TV's service token, which never leaves the TV.
EOF
  exit 2
}

(($# >= 2)) || usage
host=$1
command=$2
shift 2
dir=/var/lib/semmeltv/feedback

remote='
set -eu
header=$(mktemp)
trap "rm -f $header" EXIT
printf "Authorization: Bearer %s\n" "$(cat /run/agenix/semmeltv_service_token)" > "$header"
if [ -n "$3" ]; then
  printf %s "$3" | base64 -d | curl --silent --show-error --fail-with-body -X "$1" -H @"$header" -H "content-type: application/json" --data-binary @- "http://127.0.0.1:8090$2"
else
  curl --silent --show-error --fail-with-body -H @"$header" "http://127.0.0.1:8090$2"
fi
'

api() {
  local method=$1 path=$2 body=${3:-}
  local encoded=""
  [[ -z $body ]] || encoded=$(printf %s "$body" | base64 -w0)
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" \
    "sudo bash -c $(printf %q "$remote") bash $(printf %q "$method") $(printf %q "$path") $(printf %q "$encoded")"
}

case $command in
  list)
    query=""
    if (($# == 1)); then
      case $1 in
        new | needs_review | done) query="?state=$1" ;;
        *) usage ;;
      esac
    fi
    api GET "/api/feedback$query" |
      jq -c '.reports[] | {id, when, status, marked_at, marked_by, person: .person.name, device, page, message, screenshot, tv_screenshot}'
    ;;
  show)
    (($# == 1)) || usage
    api GET "/api/feedback/$1"
    ;;
  pictures)
    (($# == 2)) || usage
    mkdir -p "$2"
    names=$(api GET "/api/feedback/$1" | jq -r '.screenshot // empty, .tv_screenshot // empty')
    for name in $names; do
      scp -q -o BatchMode=yes "$host:$dir/$name" "$2/$name"
      echo "$2/$name"
    done
    ;;
  mark)
    (($# >= 2)) || usage
    id=$1
    case $2 in
      new)
        (($# == 2)) || usage
        body='{"state":"new"}'
        ;;
      needs-review)
        (($# == 4)) || usage
        body=$(jq -nc --arg why "$3" --arg decide "$4" '{state: "needs_review", why: $why, decide: $decide}')
        ;;
      fixed)
        (($# == 4)) || usage
        body=$(jq -nc --arg commit "$3" --arg what "$4" '{state: "done", resolution: {kind: "fixed", commit: $commit, what: $what}}')
        ;;
      no-change)
        (($# == 3)) || usage
        body=$(jq -nc --arg why "$3" '{state: "done", resolution: {kind: "no_change", why: $why}}')
        ;;
      *) usage ;;
    esac
    api POST "/api/feedback/$id/status" "$body"
    echo
    ;;
  *) usage ;;
esac
