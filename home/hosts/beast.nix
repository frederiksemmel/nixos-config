{ lib, pkgs, ... }:
let
  # Keeps one untouched `claude --remote-control` session in a detached shpool
  # session named rc-<n>, so one is always waiting in the app. A session counts
  # as used once its transcript has a user message; then another is started.
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
      Description = "Keeps one fresh Claude Code Remote Control session in shpool";
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

  programs.nushell.extraConfig = ''
    if $nu.is-interactive and ($env.SSH_CONNECTION? != null) and ($env.SHPOOL_SESSION_NAME? == null) {
      let sessions = shpool list --json | from json | get sessions | sort-by last_connected_at_unix_ms --reverse
      let new = "new session"
      let choice = if ($sessions | is-empty) { $new } else {
        $sessions
        | each {|s| $"($s.name) \(($s.status | str lowercase)\)" }
        | append $new
        | input list "shpool session (esc for a plain shell)"
      }
      if $choice == $new {
        let taken = $sessions.name
        let name = 1..(($taken | length) + 1) | each {|i| $"s($i)" } | where {|n| $n not-in $taken } | first
        exec shpool attach $name
      } else if $choice != null {
        exec shpool attach --force ($choice | split row " " | first)
      }
    }
  '';
}
