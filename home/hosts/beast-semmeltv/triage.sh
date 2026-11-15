state=${XDG_STATE_HOME:-$HOME/.local/state}/semmeltv-feedback
name="semmeltv-feedback-triage-$(date +%Y%m%d-%H%M)"
reports=$(awk '{ print "- " $2 " on " $1 ($4 == "reopened" ? ", reopened by a comment" : ", new") }' "$state/pending")

cd "$HOME/repos/nixos-config"
echo "$name: triaging"
echo "$reports"
{
  cat "$PROMPT"
  printf '\n# This run\n\nRun name: %s\n\nNew reports handed to this run:\n\n%s\n' "$name" "$reports"
} | "$CLAUDE" -p --name "$name" --permission-mode auto --strict-mcp-config
