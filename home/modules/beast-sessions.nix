{
  config,
  lib,
  ...
}:
{
  options.profiles.beastSessions.enable = lib.mkEnableOption "the beast nushell command: Claude sessions on beast in terminal windows that reattach after the connection drops";

  config = lib.mkIf config.profiles.beastSessions.enable {
    programs.ssh.settings.beast = {
      ServerAliveInterval = 10;
      ServerAliveCountMax = 2;
    };

    programs.nushell.extraConfig = ''
      use ${./beast}/beast.nu
    '';
  };
}
