{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.beastSessions;

  # Attaches a shpool session on beast and reattaches it after ssh itself fails
  # (exit 255), as long as beast still lists the session; any other exit ends
  # it. With --new, the first attach creates the session.
  beastAttach = pkgs.writeShellApplication {
    name = "beast-attach";
    runtimeInputs = with pkgs; [
      openssh
      jq
      coreutils
    ];
    text = ''
      fresh=0
      title=""
      while (($#)); do
        case "$1" in
          --new) fresh=1 ;;
          --title)
            title=$2
            shift
            ;;
          *) break ;;
        esac
        shift
      done
      if (($# != 1)) || [[ $1 == -* ]]; then
        echo "usage: beast-attach [--new] [--title TITLE] SESSION" >&2
        exit 2
      fi
      session=$1
      title=''${title:-$session}
      remote=$(jq -rn --arg s "$session" '$s | @json')
      opts=(-o ConnectTimeout=5 -o BatchMode=yes)
      err=$(mktemp)
      trap 'rm -f "$err"' EXIT

      settitle() { printf '\e]2;%s\a' "$1"; }

      delay=1
      pause() {
        settitle "$title (reconnecting)"
        printf '\e[?25h\e[?1000;1002;1003;1006;2004l\e[<99u\r\n'
        reason=$(tail -n 1 "$err")
        printf '\e[?7l\e[2mreconnecting to beast in %ss (enter retries now)%s\e[m\e[?7h' "$delay" "''${reason:+: $reason}"
        read -rs -n 1 -t "$delay" || true
        printf '\r\e[K'
        : >"$err"
        delay=$((delay < 15 ? delay * 2 : 30))
      }

      attempted=0
      while true; do
        settitle "$title"
        if ! list=$(ssh "''${opts[@]}" beast shpool list --json 2>"$err"); then
          pause
          continue
        fi
        if jq -e --arg s "$session" 'any(.sessions[]; .name == $s)' <<<"$list" >/dev/null; then
          if ((attempted)); then
            fresh=0
          fi
        elif ((!fresh)); then
          echo "beast-attach: no session $session on beast" >&2
          exit 1
        fi
        force=(-f)
        if ((fresh)); then
          force=()
        fi
        started=$SECONDS
        rc=0
        ssh -t "''${opts[@]}" beast shpool attach "''${force[@]}" "$remote" || rc=$?
        if ((rc != 255)); then
          exit "$rc"
        fi
        attempted=1
        if ((SECONDS - started < 10)); then
          pause
        else
          delay=1
        fi
      done
    '';
  };

  attach = lib.getExe beastAttach;
in
{
  options.profiles.beastSessions.enable = lib.mkEnableOption "beast, beast-attach and beast-restore: beast's shpool sessions in foot windows that reattach after the connection drops";

  config = lib.mkIf cfg.enable {
    home.packages = [ beastAttach ];

    programs.ssh.settings.beast = {
      ServerAliveInterval = 10;
      ServerAliveCountMax = 2;
    };

    programs.nushell.extraConfig = ''
      def beast-sessions [] {
        ^ssh -o ConnectTimeout=5 beast $"(open --raw ${../hosts/beast-shpool-sessions.nu} | decode utf-8)\nshpool-sessions | to json" | from json
      }

      def beast-window-title [session] {
        $session.title | str replace --regex ' \([a-z]+\)$' ""
      }

      def beast [] {
        let sessions = beast-sessions
        let new = "new session"
        let kill = "kill sessions…"
        let lines = if ($sessions | is-empty) { [] } else {
          let title_w = [($sessions.title | str length | math max) 40] | math min
          let ids_w = $sessions.ids | str length | math max
          let cols = (term size).columns - 4
          $sessions | each {|s|
            [
              ($s.title | str substring ..<$title_w | fill -w $title_w)
              ($s.ids | fill -w $ids_w)
              ($s.ago | str replace " ago" "" | fill -w 14)
              ($s.status | str replace "disconnected" "detached" | fill -w 8)
              ($s.dir | path basename)
            ] | str join "  " | str substring ..<$cols
          }
        }
        let n = $sessions | length
        let choice = if ($lines | is-empty) { $n } else {
          $lines | append [$new $kill] | input list --fuzzy --index "beast session (type to filter, esc for a plain shell)"
        }
        if $choice == null {
          exec ssh -t beast "hide-env SSH_CONNECTION; exec nu --login"
        } else if $choice < $n {
          let s = $sessions | get $choice
          exec ${attach} --title (beast-window-title $s) $s.name
        } else if $choice > $n {
          let names = $lines
          | input list --fuzzy --multi --index "kill which sessions (tab marks, enter kills, esc cancels)"
          | default []
          | each {|i| $sessions | get $i | get name | to json }
          if ($names | is-not-empty) { ^ssh beast shpool kill ...$names }
          beast
        } else {
          let taken = $sessions.name
          let default = 1..(($taken | length) + 1) | each {|i| $"s($i)" } | where {|n| $n not-in $taken } | first
          let name = input $"session name \(enter for ($default)\): " | str trim | str replace --all --regex '\s+' '-'
          exec ${attach} --new (if ($name | is-empty) { $default } else { $name })
        }
      }

      def beast-restore [--dry-run (-n)] {
        let windows = beast-sessions
        | where status == "disconnected"
        | where {|s| not (($s.name | str starts-with "rc-") and ($s.title | str starts-with "spare (")) }
        | each {|s|
          let title = beast-window-title $s
          [${lib.getExe config.programs.foot.package} --title $title ${attach} --title $title $s.name]
        }
        if $dry_run { return ($windows | each { each { if ($in =~ '\s') { to json } else { $in } } | str join " " }) }
        for cmd in $windows {
          ^${lib.getExe' pkgs.util-linux "setsid"} -f ...$cmd o+e> /dev/null
        }
      }
    '';
  };
}
