{
  lib,
  writeShellApplication,
  symlinkJoin,
  coreutils,
  gawk,
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
    runtimeInputs = [
      coreutils
      gawk
      git
    ];
    text = ''
      ${readIgnores}
      real=${lib.getExe' git-who "git-who"}

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
    runtimeInputs = [
      coreutils
      gawk
      git
    ];
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
      start=""
      rest=()
      set -- "''${args[@]}"
      while (($#)); do
        case $1 in
        --git-branch) rev=$2; shift 2 ;;
        --start-date) start=$2; shift 2 ;;
        --start-date=*) start=''${1#*=}; shift ;;
        *) rest+=("$1"); shift ;;
        esac
      done

      paths=(':/')
      while IFS= read -r p; do paths+=(":(top,exclude)$p"); done < <(stats_ignores "$repo")
      g() { git -C "$repo" -c core.quotePath=false "$@"; }

      log=$(mktemp --suffix=.gource.log)
      trap 'rm -f "$log"' EXIT

      since=()
      if [[ -n $start ]]; then
        epoch=$(date -d "$start" +%s)
        since=(--since="@$epoch")
        if base=$(g rev-list -1 --before="@$epoch" "$rev") && [[ -n $base ]]; then
          # Every file in the tree at the start date, credited to whoever
          # changed it last, so the animation opens on the repo as it was.
          awk -F '\t' -v ts="$((epoch - 1))" -v fallback="$(g log -1 --format=%aN "$base")" '
            FNR == 1 { pass++ }
            pass == 1 && /^\x01/ { author = substr($0, 2); next }
            pass == 1 && NF >= 2 && !($NF in last) { last[$NF] = author; next }
            pass == 2 && NF { printf "%s|%s|A|/%s\n", ts, ($0 in last ? last[$0] : fallback), $0 }
          ' <(g log --no-merges --no-renames --name-status --format=$'\x01%aN' "$base" -- "''${paths[@]}") \
            <(g diff --name-only --no-renames 4b825dc642cb6eb9a060e54bf8d69288fbee4904 "$base" -- "''${paths[@]}") >>"$log"
        fi
      fi

      g log --reverse --no-merges --no-renames --name-status --format=$'\x01%ct|%aN' "''${since[@]}" "$rev" -- "''${paths[@]}" |
        awk -F '\t' '
          /^\x01/ { head = substr($0, 2); next }
          NF >= 2 { t = substr($1, 1, 1); if (t != "A" && t != "D") t = "M"; printf "%s|%s|/%s\n", head, t, $NF }
        ' >>"$log"

      "$real" --log-format custom "''${rest[@]}" "$log"
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
