# Sourced by the hooks in this directory.

# Secret matches that are not secrets.
kingfisher_skip=(
  # semmeltv browser-test fixtures key movies by title-year slugs
  '^[a-z0-9]+(-[a-z0-9]+)*-\d{4}$'
)

# Scans this repo for secrets, offline (--no-validate keeps matches from being
# sent to their services); the arguments pick the changes to scan. Fails on a
# finding (exit 200 or 205) and on any error.
kingfisher_scan() {
  local hook="$1" out r status=0
  shift
  out="$(nix build --no-link --print-out-paths --inputs-from . nixpkgs#kingfisher)"
  local args=(scan . --no-validate --no-update-check --redact --confidence low --quiet)
  for r in "${kingfisher_skip[@]}"; do
    args+=(--skip-regex "$r")
  done
  "$out/bin/kingfisher" "${args[@]}" "$@" || status=$?
  case "$status" in
    0) return 0 ;;
    200 | 205) echo "$hook: kingfisher found what looks like a secret (above)." >&2 ;;
    *) echo "$hook: kingfisher failed with exit code $status." >&2 ;;
  esac
  return 1
}

# Reads NUL-separated paths on stdin; fails listing any that sit inside a
# hidden directory, except the ones this repo tracks on purpose.
check_hidden_dirs() {
  local hook="$1" f
  local hidden=()
  while IFS= read -r -d '' f; do
    case "$f" in
      .githooks/* | .claude/skills/*) ;;
      .*/* | */.*/*) hidden+=("$f") ;;
    esac
  done
  if [ "${#hidden[@]}" -gt 0 ]; then
    echo "$hook: these paths are inside a hidden directory, which this repo doesn't commit:" >&2
    printf '  %s\n' "${hidden[@]}" >&2
    echo "Unstage them (git rm --cached) and gitignore the directory, or add it to the allowlist in .githooks/lib.sh." >&2
    return 1
  fi
}
