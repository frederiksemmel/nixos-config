{
  config,
  lib,
  ...
}:
let
  cfg = config.profiles.semmeltv;
in
{
  options.profiles.semmeltv.enable = lib.mkEnableOption "ssh access to the SemmelTV Pis and the TMDB token for the app";

  config = lib.mkIf cfg.enable {
    age.secrets = {
      semmeltv_ssh_key = {
        file = ../../secrets/semmeltv_ssh_key.age;
        path = "${config.home.homeDirectory}/.ssh/id_semmeltv";
      };
      tmdb_token = {
        file = ../../secrets/tmdb_token.age;
        path = "${config.xdg.dataHome}/semmeltv/tmdb-token";
      };
    };

    home.sessionVariables.TMDB_TOKEN_FILE = config.age.secrets.tmdb_token.path;

    programs.ssh.settings."semmeltv*" = {
      User = "frederik";
      IdentityFile = "~/.ssh/id_semmeltv";
      IdentitiesOnly = true;
      StrictHostKeyChecking = "accept-new";
    };
  };
}
