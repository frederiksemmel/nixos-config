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
  # it. With --new, the first attach creates the session, in --dir running --cmd.
  beastAttach = pkgs.writeShellApplication {
    name = "beast-attach";
    runtimeInputs = with pkgs; [
      openssh
      jq
      coreutils
    ];
    text = ''
      quote() { jq -rn --arg s "$1" '$s | @json'; }

      fresh=0
      title=""
      create=()
      while (($#)); do
        case "$1" in
          --new) fresh=1 ;;
          --dir | --cmd)
            create+=("$1" "$(quote "$2")")
            shift
            ;;
          --title)
            title=$2
            shift
            ;;
          *) break ;;
        esac
        shift
      done
      if (($# != 1)) || [[ $1 == -* ]]; then
        echo "usage: beast-attach [--new [--dir DIR] [--cmd CMD]] [--title TITLE] SESSION" >&2
        exit 2
      fi
      session=$1
      title=''${title:-$session}
      remote=$(quote "$session")
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
        mode=(-f)
        if ((fresh)); then
          mode=("''${create[@]}")
        fi
        started=$SECONDS
        rc=0
        ssh -t "''${opts[@]}" beast shpool attach "''${mode[@]}" "$remote" || rc=$?
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
  options.profiles.beastSessions.enable = lib.mkEnableOption "beast, beast-attach, beast-new, beast-spare, beast-clean and beast-restore: beast's shpool sessions in foot windows that reattach after the connection drops";

  config = lib.mkIf cfg.enable {
    home.packages = [ beastAttach ];

    programs.ssh.settings.beast = {
      ServerAliveInterval = 10;
      ServerAliveCountMax = 2;
    };

    programs.nushell.extraConfig = ''
      def beast-remote [command] {
        ^ssh -o ConnectTimeout=5 beast $"(open --raw ${../hosts/beast-shpool-sessions.nu} | decode utf-8)\n($command) | to json" | from json
      }

      def beast-sessions [] {
        beast-remote shpool-sessions
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
          beast-new
        }
      }

      # New shpool session on beast: Claude with Remote Control in a repo, inside
      # devenv when the repo has one. Without a repo, asks for one or a plain shell.
      def beast-new [
        repo?: string       # repo under ~/repos on beast, or a unique part of its name
        --name (-n): string # session name, default <repo>-<number>
      ] {
        let repos = beast-remote shpool-repos
        let target = if $repo == null {
          let i = ["plain shell" ...$repos.name] | input list --fuzzy --index "new session in (esc cancels)"
          if $i == null { return }
          if $i == 0 { null } else { $repos | get ($i - 1) }
        } else {
          let exact = $repos | where name == $repo
          let found = if ($exact | is-empty) { $repos | where name =~ $repo } else { $exact }
          if ($found | length) != 1 {
            error make --unspanned { msg: $"($repo) matches ($found | length) of: ($repos.name | str join ' ')" }
          }
          $found | first
        }
        let taken = beast-sessions | get name
        let prefix = if $target == null { "s" } else { $"($target.name)-" }
        let default = 1..(($taken | length) + 1) | each {|i| $"($prefix)($i)" } | where {|n| $n not-in $taken } | first
        let chosen = if $name != null {
          $name
        } else if $repo != null {
          ""
        } else {
          input $"session name \(enter for ($default)\): " | str trim | str replace --all --regex '\s+' '-'
        }
        let session = if ($chosen | is-empty) { $default } else { $chosen }
        if $target == null {
          exec ${attach} --new $session
        }
        let claude = $"claude --remote-control --remote-control-session-name-prefix ($target.name)"
        let cmd = if $target.devenv { $"devenv shell --no-tui -- ($claude)" } else { $claude }
        exec ${attach} --new --dir $target.path --cmd $cmd $session
      }

      # Attaches the newest detached Remote Control spare from claude-rc-spare.
      def beast-spare [] {
        let spares = beast-sessions | where {|s| $s.spare and $s.status == "disconnected" }
        if ($spares | is-empty) {
          error make --unspanned { msg: "no detached spare on beast yet, claude-rc-spare starts one within a few seconds" }
        }
        exec ${attach} ($spares | first | get name)
      }

      # Kills the sessions matching every given filter, after confirming. Attached
      # sessions, busy Claude sessions and unused spares are kept unless included.
      def beast-clean [
        --no-message-for: duration # no message from you for this long (shells: not attached or detached for this long)
        --detached-for: duration   # detached for this long
        --older-than: duration     # started this long ago
        --repo: string             # in the repo with this exact name
        --match: string            # name or title matches this regex
        --include-attached         # also consider attached sessions
        --include-busy             # also consider Claude sessions that are working
        --dry-run (-n)             # list matches without killing
        --yes (-y)                 # kill without asking
      ] {
        if ([$no_message_for $detached_for $older_than $repo $match] | compact | is-empty) {
          error make --unspanned { msg: "give at least one filter, see beast-clean --help" }
        }
        let since = {|ms| (date now) - ($ms * 1_000_000 | into datetime) }
        let matches = beast-sessions
        | where {|s| not $s.spare }
        | where {|s| $include_attached or $s.status != "attached" }
        | where {|s| $include_busy or $s.claude != "busy" }
        | where {|s| $no_message_for == null or (do $since ($s.message | default $s.last)) >= $no_message_for }
        | where {|s| $detached_for == null or ($s.status != "attached" and $s.detached != null and (do $since $s.detached) >= $detached_for) }
        | where {|s| $older_than == null or (do $since $s.started) >= $older_than }
        | where {|s| $repo == null or $s.repo == $repo }
        | where {|s| $match == null or $s.name =~ $match or $s.title =~ $match }
        if ($matches | is-empty) {
          print "no sessions match"
          return
        }
        print ($matches | each {|s|
          {
            name: $s.name
            title: ($s.title | str substring ..<50)
            repo: $s.repo
            status: $s.status
            "last message": (if $s.message == null { "" } else { $s.message * 1_000_000 | into datetime | date humanize })
          }
        })
        if $dry_run { return }
        if $yes or ((input $"kill these ($matches | length) sessions? [y/N] ") in ["y" "Y"]) {
          ^ssh beast shpool kill ...($matches | each { $in.name | to json })
        }
      }

      def beast-restore [--dry-run (-n)] {
        let windows = beast-sessions
        | where status == "disconnected"
        | where {|s| not $s.spare }
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
