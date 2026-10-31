{
  flake,
  config,
  ...
}:
{
  imports = with flake.inputs; [
    agenix.homeManagerModules.default
    ../modules/theme.nix
    ../modules/dev.nix
    ../modules/desktop.nix
    ../modules/ssh.nix
    ../modules/bitwarden.nix
  ];

  # Building the option docs leaks nixpkgs' store path into options.json without
  # string context, which Determinate Nix warns about on every rebuild.
  # https://github.com/nix-community/home-manager/issues/7935
  manual.manpages.enable = false;

  profiles.dev.enable = true;
  profiles.desktop.enable = true;
  profiles.ssh.enable = true;
  profiles.bitwarden.enable = true;

  home.username = "frederik";
  home.homeDirectory = "/home/${config.home.username}";
  home.language.base = "en_US.UTF-8";

  programs.git.settings.user = {
    name = "Frederik Semmel";
    email = "frederiksemmel@gmail.com";
  };

  age.secrets = {
    spadesvision_gdrive_token.file = ../../secrets/spadesvision_gdrive_token.age;
    spadesvision_gdrive_client_secret.file = ../../secrets/spadesvision_gdrive_client_secret.age;
    pcloud_token.file = ../../secrets/pcloud_token.age;
  };
  age.identityPaths = [ "${config.home.homeDirectory}/.ssh/id_github" ];

  programs.rclone.remotes.spadesvision = {
    config = {
      type = "drive";
      client_id = "113233231770-v4b6rfq5ajc3imv0vs0g3fnbl2q8rd84.apps.googleusercontent.com";
      scope = "drive";
      team_drive = "0AH2EQdMAdZriUk9PVA";
      root_folder_id = "";
    };
    secrets = {
      client_secret = config.age.secrets.spadesvision_gdrive_client_secret.path;
      token = config.age.secrets.spadesvision_gdrive_token.path;
    };
  };

  # The whole account as an on-demand drive: files download only when opened,
  # into a size-capped cache. ~/Scans alone is kept locally, by scans-sync.
  programs.rclone.remotes.pcloud = {
    config = {
      type = "pcloud";
      hostname = "api.pcloud.com";
    };
    secrets.token = config.age.secrets.pcloud_token.path;
    mounts."" = {
      enable = true;
      mountPoint = "${config.home.homeDirectory}/pcloud";
      options = {
        vfs-cache-max-size = "10G";
        vfs-cache-max-age = "720h";
      };
    };
  };
}
