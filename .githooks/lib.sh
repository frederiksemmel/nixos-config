# Sourced by the hooks in this directory.

gitleaks_bin() {
  local out
  out="$(nix build --no-link --print-out-paths --inputs-from . nixpkgs#gitleaks)"
  echo "$out/bin/gitleaks"
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
