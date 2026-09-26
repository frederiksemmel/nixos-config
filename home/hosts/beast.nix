{
  config,
  lib,
  pkgs,
  ...
}:
let
  directory = "${config.home.homeDirectory}/repos/blackjackeye";
  transcripts = "${config.home.homeDirectory}/.claude/projects/${
    lib.replaceStrings [ "/" "." ] [ "-" "-" ] directory
  }";

  # Keeps exactly one untouched Remote Control session in the directory. Each
  # session is its own single-session `claude remote-control` instance; once
  # every running instance has a user message in its transcript, another one
  # is started.
  claude-rc-pool = pkgs.writeShellApplication {
    name = "claude-rc-pool";
    runtimeInputs = with pkgs; [
      coreutils
      gawk
      gnugrep
      jq
      procps
      systemd
    ];
    text = ''
      while true; do
        fresh=0
        for unit in $(systemctl --user list-units --plain --no-legend --state=active,activating 'claude-rc-blackjackeye@*.service' | awk '{print $1}'); do
          pid=$(systemctl --user show -p MainPID --value "$unit")
          used=0
          for child in $(pgrep -P "$pid" || true); do
            session=$(jq -r '.sessionId // empty' "$HOME/.claude/sessions/$child.json" 2>/dev/null || true)
            if [[ -n $session ]] && grep -qs '"type":"user"' "${transcripts}/$session.jsonl"; then
              used=1
            fi
          done
          if ((used == 0)); then
            fresh=$((fresh + 1))
          fi
        done
        if ((fresh == 0)); then
          systemctl --user start "claude-rc-blackjackeye@$(date +%s).service"
        fi
        sleep 10
      done
    '';
  };
in
{
  systemd.user.services."claude-rc-blackjackeye@" = {
    Unit.Description = "Claude Code Remote Control session %i in ${directory}";
    Service = {
      WorkingDirectory = directory;
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --spawn=session";
      StandardOutput = "null";
    };
  };

  systemd.user.services.claude-rc-pool = {
    Unit = {
      Description = "Keeps one fresh Claude Code Remote Control session in ${directory}";
      Wants = [ "network-online.target" ];
      After = [ "network-online.target" ];
    };
    Service = {
      ExecStart = lib.getExe claude-rc-pool;
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };
}
