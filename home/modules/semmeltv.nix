{
  config,
  lib,
  ...
}:
let
  cfg = config.profiles.semmeltv;
in
{
  options.profiles.semmeltv.enable = lib.mkEnableOption "ssh access to the SemmelTV Pis and the TMDB and MDBList keys for library imports";

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
      mdblist_api_key = {
        file = ../../secrets/mdblist_api_key.age;
        path = "${config.xdg.dataHome}/semmeltv/mdblist-api-key";
      };
    };

    home.sessionVariables = {
      TMDB_TOKEN_FILE = config.age.secrets.tmdb_token.path;
      MDBLIST_API_KEY_FILE = config.age.secrets.mdblist_api_key.path;
    };

    programs.ssh.settings."semmeltv*" = {
      User = "frederik";
      IdentityFile = "~/.ssh/id_semmeltv";
      IdentitiesOnly = true;
      StrictHostKeyChecking = "accept-new";
    };
  };
}
