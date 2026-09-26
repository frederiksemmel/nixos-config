{ config, lib, ... }:
{
  systemd.user.services.claude-remote-control = {
    Unit.Description = "Claude Code Remote Control in ~/repos/blackjackeye";
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
