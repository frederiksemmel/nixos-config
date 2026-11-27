usage() {
  cat >&2 <<'EOF'
usage: feedback-ramura-site open
       feedback-ramura-site brief ID
       feedback-ramura-site show ID
       feedback-ramura-site screenshot ID N FILE
       feedback-ramura-site comment ID TEXT
       feedback-ramura-site mark ID fixed COMMIT WHAT
       feedback-ramura-site mark ID no-change WHY
       feedback-ramura-site mark ID needs-review WHY DECIDE [--ask QUESTION (--option TEXT | --recommend TEXT)...]...
       feedback-ramura-site deploy

The feedback framework's adapter for Ramura's site (feedback/README.md). A
report is an issue labelled feedback on RAMURA_REPO; ID is its number. Its
batch and screenshots are on the feedback-screenshots branch. Only comments by
the repo's owner and collaborators count. A comment from here is signed Claude
and never reopens a report. mark comments on the issue itself.
EOF
  exit 2
}

(($# >= 1)) || usage
command=$1
shift

marker='<!-- feedback-claude -->'
screenshots=feedback-screenshots

# The issue's trusted comments, as {by, at, text}; by is "claude" or a login.
thread() {
  gh api --paginate "repos/$RAMURA_REPO/issues/$1/comments?per_page=100" | jq -s --arg marker "$marker" '
    add // [] | map(select(.author_association | IN("OWNER", "COLLABORATOR", "MEMBER"))
      | {by: (if (.body | contains($marker)) then "claude" else .user.login end), at: .created_at,
         text: (.body | sub("\n*<sub>Claude, from a feedback session</sub> *" + $marker; ""))})'
}

issue() {
  gh api "repos/$RAMURA_REPO/issues/$1" | jq '{number, title, state, state_reason, html_url, created_at,
    labels: [.labels[].name], author: .user.login, trusted: (.author_association | IN("OWNER", "COLLABORATOR", "MEMBER")),
    report: (.body | capture("<!-- ramura-site report=(?<r>[^ ]+) -->").r // null)}'
}

batch() {
  gh api -H "Accept: application/vnd.github.raw" "repos/$RAMURA_REPO/contents/reports/$1/batch.json?ref=$screenshots"
}

pr() {
  gh api "repos/$RAMURA_REPO/pulls?state=all&head=${RAMURA_REPO%%/*}:feedback/issue-$1" | jq '[.[] | {number, html_url, state, draft, merged_at, head: .head.ref}] | first // null'
}

say() {
  gh api "repos/$RAMURA_REPO/issues/$1/comments" -f body="$2"$'\n\n'"<sub>Claude, from a feedback session</sub> $marker" >/dev/null
}

label() {
  local id=$1 add=$2 name
  shift 2
  for name in "$@"; do
    gh api -X DELETE "repos/$RAMURA_REPO/issues/$id/labels/$name" >/dev/null 2>&1 || true
  done
  [[ -z $add ]] && return
  gh api "repos/$RAMURA_REPO/labels/$add" >/dev/null 2>&1 || gh api "repos/$RAMURA_REPO/labels" -f name="$add" -f color=ededed >/dev/null
  gh api "repos/$RAMURA_REPO/issues/$id/labels" -f "labels[]=$add" >/dev/null
}

case $command in
  open)
    if [[ ! -d $RAMURA_CHECKOUT/.git ]]; then
      gh repo clone "$RAMURA_REPO" "$RAMURA_CHECKOUT" -- --quiet >&2
    fi
    grep -qxF .claude/ "$RAMURA_CHECKOUT/.git/info/exclude" 2>/dev/null || echo .claude/ >>"$RAMURA_CHECKOUT/.git/info/exclude"
    gh api --paginate "repos/$RAMURA_REPO/issues?labels=feedback&state=all&per_page=100" |
      jq -c '.[] | select(.pull_request | not) | select(.author_association | IN("OWNER", "COLLABORATOR", "MEMBER"))
        | {number, state, created_at, comments, labels: [.labels[].name]}' |
      while read -r found; do
        id=$(jq -r .number <<<"$found")
        if [[ $(jq -r .comments <<<"$found") == 0 ]]; then
          comments='[]'
        else
          comments=$(thread "$id")
        fi
        jq -c --argjson thread "$comments" '
          . as $i
          | ($thread | map(select(.by == "claude")) | last | .at) as $claude
          | ($thread | map(select(.by != "claude")) | last | .at) as $person
          | if $claude == null then
              (if $i.state == "open" and ($i.labels | index("needs-review") | not) and ($i.labels | index("in-pr") | not)
               then {state: "new", at: $i.created_at} else empty end)
            elif $person != null and $person > $claude then {state: "reopened", at: $person}
            else empty end
          | {id: ($i.number | tostring), state, marked_at: (.at | fromdateiso8601)}' <<<"$found"
      done
    ;;
  show)
    (($# == 1)) || usage
    found=$(issue "$1")
    report=$(jq -r '.report // empty' <<<"$found")
    jq -n --argjson issue "$found" --argjson thread "$(thread "$1")" --argjson pr "$(pr "$1")" \
      --argjson batch "$(if [[ -n $report ]]; then batch "$report"; else echo null; fi)" \
      '{issue: $issue, pr: $pr, batch: $batch, thread: $thread}'
    ;;
  screenshot)
    (($# == 3)) || usage
    report=$(issue "$1" | jq -r '.report // empty')
    [[ -n $report ]] || {
      echo "issue $1 has no report" >&2
      exit 1
    }
    gh api -H "Accept: application/vnd.github.raw" "repos/$RAMURA_REPO/contents/reports/$report/$(printf %02d "$2").png?ref=$screenshots" >"$3"
    echo "$3"
    ;;
  brief)
    (($# == 1)) || usage
    "$0" show "$1" | jq -r --arg adapter "$(basename "$0")" '
      def device: if .batch.viewport.coarse then "a phone" else "a laptop" end;
      def who: if . == "claude" then "Claude" elif . == "ragassler" then "Ramura" elif . == "frederiksemmel" then "Frederik" else . end;
      "## Report \(.issue.number) (issue \(.issue.html_url), \(.issue.state), labels: \(.issue.labels | join(", ")))\n",
      "Issue opened by \(.issue.author | who), \(.issue.created_at).",
      (if .batch then "Sent from \(.batch.page) on \(device), \(.batch.viewport.width)×\(.batch.viewport.height) CSS px, about the site at commit \(.batch.site.rev // "?")." else "No batch on the screenshots branch: the issue text is the report (`gh issue view \(.issue.number) --repo ragassler/ragassler.github.io`)." end),
      (if .pr then "Draft PR so far: \(.pr.html_url) (\(.pr.state)\(if .pr.merged_at then ", merged" else "" end), branch \(.pr.head))." else "No PR yet." end),
      (if (.batch.note // "") != "" then "\nNote for the whole batch: \(.batch.note)" else empty end),
      (if .batch then "\nComments (see each one: `\($adapter) screenshot \(.issue.number) N FILE`, then Read it):\n" else empty end),
      (.batch.items[]? | "\(.n). **\(.comment)**\n   on \(.what) (\(.kind)) at \(.page)\n   ```json\n   \({target, rect, point, inside, context} | tojson)\n   ```"),
      (if (.thread | length) > 0 then "\nThread:\n", (.thread[] | "- \(.at) \(.by | who): \(.text)") else empty end)'
    ;;
  comment)
    (($# == 2)) || usage
    say "$1" "$2"
    ;;
  mark)
    (($# >= 2)) || usage
    id=$1
    kind=$2
    shift 2
    case $kind in
      fixed)
        (($# == 2)) || usage
        link=$(pr "$id" | jq -r '.html_url // empty')
        say "$id" "$2"$'\n\n'"Changed in ${link:-the branch feedback/issue-$id} ($1). It reaches the site once the pull request is merged."
        label "$id" in-pr needs-review
        gh api -X PATCH "repos/$RAMURA_REPO/issues/$id" -f state=open >/dev/null
        ;;
      no-change)
        (($# == 1)) || usage
        say "$id" "No change: $1"
        label "$id" "" needs-review in-pr
        gh api -X PATCH "repos/$RAMURA_REPO/issues/$id" -f state=closed -f state_reason=not_planned >/dev/null
        ;;
      needs-review)
        (($# >= 2)) || usage
        text="$1"$'\n\n'"**To decide:** $2"
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
              text+=$'\n'"- ${letters[index]}) $2 (recommended)"
              index=$((index + 1))
              ;;
            --option)
              text+=$'\n'"- ${letters[index]}) $2"
              index=$((index + 1))
              ;;
            *) usage ;;
          esac
          shift 2
        done
        (($# == 0)) || usage
        say "$id" "$text"$'\n\n'"Answer in a comment here (\"b\" is enough) and it is taken up again."
        label "$id" needs-review in-pr
        ;;
      *) usage ;;
    esac
    ;;
  deploy)
    echo "Nothing to deploy: GitHub Pages publishes main once Ramura merges a pull request."
    ;;
  *) usage ;;
esac
