{
  config,
  lib,
  ...
}:
{
  systemd.user.services.claude-remote-control-blackjackeye = {
    Unit = {
      Description = "Claude Code Remote Control session in ~/repos/blackjackeye";
      Wants = [ "network-online.target" ];
      After = [ "network-online.target" ];
    };
    Service = {
      WorkingDirectory = "%h/repos/blackjackeye";
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --spawn=worktree";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };
}
