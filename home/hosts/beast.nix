{
  config,
  lib,
  pkgs,
  ...
}:
let
  # Keeps one untouched `claude --remote-control` session in a detached shpool
  # session named rc-<n>, so one is always waiting in the app. A session counts
  # as used once its transcript has a user message; then another is started.
  # Also drops shpool sessions whose process has exited, e.g. after /exit: shpool
  # keeps those listed, and `shpool kill` only removes one with a live process.
  claude-rc-spare = pkgs.writeShellApplication {
    name = "claude-rc-spare";
    runtimeInputs = with pkgs; [
      coreutils
      gnugrep
      jq
      shpool
    ];
    text = ''
      dir="$HOME/repos/blackjackeye"
      while true; do
        for name in $(shpool list --json | jq -r '.sessions[].name'); do
          if ! grep -qszxF "SHPOOL_SESSION_NAME=$name" /proc/[0-9]*/environ; then
            shpool attach --background --cmd "sleep 60" "$name"
            shpool kill "$name"
          fi
        done
        names=$(shpool list --json | jq -r '.sessions[].name | select(startswith("rc-"))')
        fresh=0
        for name in $names; do
          used=0
          for f in "$HOME"/.claude/sessions/*.json; do
            pid=$(jq -r .pid "$f" 2>/dev/null) || continue
            [[ -r /proc/$pid/environ ]] || continue
            grep -qxzF "SHPOOL_SESSION_NAME=$name" "/proc/$pid/environ" || continue
            cwd=$(jq -r .cwd "$f")
            session=$(jq -r .sessionId "$f")
            if grep -qs '"type":"user"' "$HOME/.claude/projects/''${cwd//[^A-Za-z0-9]/-}/$session.jsonl"; then
              used=1
            fi
          done
          if ((used == 0)); then
            fresh=$((fresh + 1))
          fi
        done
        if ((fresh == 0)); then
          n=1
          while grep -qx "rc-$n" <<< "$names"; do
            n=$((n + 1))
          done
          shpool attach --background --dir "$dir" --cmd "devenv shell --no-tui -- claude --remote-control" "rc-$n"
        fi
        sleep 10
      done
    '';
  };
in
{
  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  services.pueue.enable = true;

  services.shpool = {
    enable = true;
    settings = {
      prompt_prefix = "";
      motd = "never";
      forward_env = [
        "PATH"
        "NIX_PATH"
        "NIX_PROFILES"
        "NIX_USER_PROFILE_DIR"
        "NIXPKGS_CONFIG"
        "LOCALE_ARCHIVE"
        "LANG"
        "TZDIR"
        "TERMINFO_DIRS"
        "XDG_CONFIG_DIRS"
        "XDG_DATA_DIRS"
        "INFOPATH"
        "LIBEXEC_PATH"
        "EDITOR"
        "PAGER"
      ];
    };
  };
  systemd.user.services.shpool.Unit.X-SwitchMethod = "keep-old";

  systemd.user.services.claude-rc-spare = {
    Unit = {
      Description = "Reaps exited shpool sessions and keeps one fresh Claude Code Remote Control session";
      Requires = [ "shpool.socket" ];
      After = [ "shpool.socket" ];
    };
    Service = {
      ExecStart = lib.getExe claude-rc-spare;
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # nixos-config uses the server mode instead, so sessions can be spawned from
  # the app, each in its own worktree.
  systemd.user.services.claude-remote-control-nixos-config = {
    Unit = {
      Description = "Claude Code Remote Control server in ~/repos/nixos-config";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      WorkingDirectory = "%h/repos/nixos-config";
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --remote-control-session-name-prefix nixos-config --spawn=worktree --no-create-session-in-dir";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };

  programs.nushell.extraConfig = ''
    def shpool-session-name [pid] {
      open --raw $"/proc/($pid)/environ" | split row (char nul) | where $it starts-with "SHPOOL_SESSION_NAME=" | first | str substring 20..
    }

    def shpool-sessions [] {
      let sessions = shpool list --json | from json | get sessions
      let daemon = ^pgrep -u $env.USER -x shpool | lines | first
      let shells = ^pgrep -P $daemon | lines | each {|pid|
        try {
          let fg = open --raw $"/proc/($pid)/stat" | split row ") " | last | split row " " | get 5
          { name: (shpool-session-name $pid), fg: $fg }
        }
      } | compact
      let claudes = glob $"($env.HOME)/.claude/sessions/*.json" | each {|f|
        try {
          let s = open $f
          let transcript = $"($env.HOME)/.claude/projects/($s.cwd | str replace --all --regex '[^A-Za-z0-9]' '-')/($s.sessionId).jsonl"
          let titles = try { ^grep -E '"type":"(custom|ai)-title"' $transcript | lines | each { from json } } catch { [] }
          let prompt = try {
            ^grep -m1 '"type":"user"' $transcript | from json | get message.content
            | if ($in | describe | str starts-with "string") { $in } else { $in | where type == "text" | get text | str join " " }
            | lines | str trim | compact --empty
            | where {|l| not ($l starts-with "<" or $l starts-with "Another Claude session sent a message") }
            | first
          } catch { null }
          let title = $titles | where type == "custom-title" | get -o customTitle | last
            | default ($titles | where type == "ai-title" | get -o aiTitle | last)
            | default $prompt
            | default "spare"
          { name: (shpool-session-name $s.pid), title: $title, status: $s.status, cwd: $s.cwd }
        }
      } | compact
      $sessions | each {|s|
        let claude = $claudes | where name == $s.name | get -o 0
        let fg = $shells | where name == $s.name | get -o 0.fg
        let cwd = if $claude != null { $claude.cwd } else if $fg != null { try { $"/proc/($fg)/cwd" | path expand } } else { null }
        let what = if $claude != null {
          $"claude: ($claude.title) \(($claude.status)\)"
        } else if $fg != null {
          open --raw $"/proc/($fg)/cmdline" | split row (char nul) | compact --empty | each { path basename } | str join " "
        } else { "" }
        let last = [$s.started_at_unix_ms $s.last_connected_at_unix_ms $s.last_disconnected_at_unix_ms] | compact | math max
        {
          name: $s.name
          status: ($s.status | str lowercase)
          last: $last
          ago: ($last * 1_000_000 | into datetime | date humanize)
          dir: ($cwd | default "")
          what: $what
        }
      } | sort-by last --reverse
    }

    def shpool-pick [] {
      let sessions = shpool-sessions
      let new = "new session"
      let kill = "kill sessions…"
      let lines = if ($sessions | is-empty) { [] } else {
        let name_w = $sessions.name | str length | math max
        let dir_w = $sessions.dir | path basename | str length | math max
        let cols = (term size).columns - 4
        $sessions | each {|s|
          [
            ($s.name | fill -w $name_w)
            ($s.status | str replace "disconnected" "detached" | fill -w 8)
            ($s.ago | str replace " ago" "" | fill -w 14)
            ($s.dir | path basename | fill -w $dir_w)
            $s.what
          ] | str join "  " | str substring ..<$cols
        }
      }
      let choice = if ($lines | is-empty) { $new } else {
        $lines | append [$new $kill] | input list --fuzzy "shpool session (type to filter, esc for a plain shell)"
      }
      if $choice == $kill {
        $lines
        | input list --fuzzy --multi "kill which sessions (tab marks, enter kills, esc cancels)"
        | default []
        | each {|l| shpool kill ($l | split row " " | first) }
        | ignore
        shpool-pick
      } else if $choice == $new {
        let taken = $sessions.name
        let default = 1..(($taken | length) + 1) | each {|i| $"s($i)" } | where {|n| $n not-in $taken } | first
        let name = input $"session name \(enter for ($default)\): " | str trim | str replace --all --regex '\s+' '-'
        exec shpool attach (if ($name | is-empty) { $default } else { $name })
      } else if $choice != null {
        exec shpool attach --force ($choice | split row " " | first)
      }
    }

    if $nu.is-interactive and ($env.SSH_CONNECTION? != null) and ($env.SHPOOL_SESSION_NAME? == null) {
      shpool-pick
    }
  '';
}
