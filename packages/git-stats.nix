{
  lib,
  writeShellApplication,
  symlinkJoin,
  git,
  git-who,
  gource,
}:
let
  readIgnores = ''
    # Patterns from the global, repo and local ignore files, one per line.
    # GIT_STATS_IGNORE=0 turns them off.
    stats_ignores() {
      [[ ''${GIT_STATS_IGNORE:-1} == 0 ]] && return
      local top common f
      top=$(git -C "$1" rev-parse --show-toplevel)
      common=$(git -C "$1" rev-parse --path-format=absolute --git-common-dir)
      for f in "''${XDG_CONFIG_HOME:-$HOME/.config}/git-stats/ignore" "$top/.statsignore" "$common/info/statsignore"; do
        [[ -f $f ]] && sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' -e '/^#/d' -e '/^$/d' "$f"
      done
      return 0
    }
  '';

  gitWho = writeShellApplication {
    name = "git-who";
    runtimeInputs = [ git ];
    text = ''
      ${readIgnores}
      real=${lib.getExe git-who}

      for a in "$@"; do
        case $a in -h | -help | --help | -version | --version) exec "$real" "$@" ;; esac
      done
      if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        exec "$real" "$@"
      fi

      prefix=$(git rev-parse --show-prefix)
      mapfile -t pats < <(stats_ignores .)
      cd "$(git rev-parse --show-toplevel)"

      args=()
      paths=()
      seen_sep=0
      for a in "$@"; do
        if ((seen_sep)); then
          case $a in
          :!*) paths+=(":!$prefix''${a#:!}") ;;
          :*) paths+=("$a") ;;
          *) paths+=("$prefix$a") ;;
          esac
        elif [[ $a == -- ]]; then
          seen_sep=1
        else
          args+=("$a")
        fi
      done
      ((''${#paths[@]})) || paths=(.)
      for p in "''${pats[@]}"; do paths+=(":!$p"); done
      exec "$real" "''${args[@]}" -- "''${paths[@]}"
    '';
  };

  gourceWrapped = writeShellApplication {
    name = "gource";
    runtimeInputs = [ git ];
    text = ''
      ${readIgnores}
      real=${lib.getExe gource}

      args=("$@")
      repo=.
      n=''${#args[@]}
      if ((n)); then
        last=''${args[n - 1]}
        prev=""
        ((n > 1)) && prev=''${args[n - 2]}
        if [[ $prev != -* && -d $last ]]; then
          repo=$last
          unset 'args[n-1]'
        elif [[ $last == - || ($prev != -* && -f $last) ]]; then
          exec "$real" "$@"
        fi
      fi
      if ! git -C "$repo" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        exec "$real" "$@"
      fi

      rev=HEAD
      rest=()
      set -- "''${args[@]}"
      while (($#)); do
        case $1 in
        --git-branch) rev=$2; shift 2 ;;
        *) rest+=("$1"); shift ;;
        esac
      done

      excludes=()
      while IFS= read -r p; do excludes+=(":(top,exclude)$p"); done < <(stats_ignores "$repo")

      log=$(mktemp --suffix=.gource.log)
      trap 'rm -f "$log"' EXIT
      git -C "$repo" log --pretty=format:user:%aN%n%ct --reverse --raw --encoding=UTF-8 \
        --no-renames --no-show-signature "$rev" -- ':/' "''${excludes[@]}" >"$log"
      "$real" --log-format git "''${rest[@]}" "$log"
    '';
  };
in
symlinkJoin {
  name = "git-stats";
  paths = [
    gitWho
    gourceWrapped
  ];
}
