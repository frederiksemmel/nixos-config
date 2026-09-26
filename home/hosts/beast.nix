{
  config,
  lib,
  pkgs,
  ...
}:
let
  remoteControl = repo: {
    Unit.Description = "Claude Code Remote Control in ~/repos/${repo}";
    Service = {
      WorkingDirectory = "%h/repos/${repo}";
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --spawn=worktree";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Each session's own worktree's devenv shell, snapshotted once; Claude sources
  # CLAUDE_ENV_FILE before every Bash call.
  devenvEnv = pkgs.writeShellApplication {
    name = "claude-devenv-env";
    runtimeInputs = [ pkgs.devenv ];
    text = ''
      cd "$CLAUDE_PROJECT_DIR"
      [ -f devenv.nix ] || exit 0
      devenv shell -- bash -c 'export -p' \
        | grep '^declare -x ' \
        | grep -Ev '^declare -x (PWD|OLDPWD|SHLVL|_)=' >> "$CLAUDE_ENV_FILE"
    '';
  };
in
{
  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  systemd.user.services.claude-remote-control-blackjackeye = remoteControl "blackjackeye";
  systemd.user.services.claude-remote-control-nixos-config = remoteControl "nixos-config";

  programs.claude-code.settings.hooks.SessionStart = [
    {
      matcher = "startup";
      hooks = [
        {
          type = "command";
          command = lib.getExe devenvEnv;
          # The first shell in a fresh worktree builds the venv and the sv-rs extensions.
          timeout = 1800;
        }
      ];
    }
  ];
}
