{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.ssh;
  # host pattern -> directory under ~/.ssh/spadesvision holding its key pair
  identityDirs = {
    "*bern*" = "spadesvision";
    "*pfaeffikon*" = "spadesvision";
    "*zurich*" = "spadesvision";
    "*office*" = "spadesvision";
    "*badragaz*" = "spadesvision";
    "*tcs*" = "tcs-jetson";
    "sv-tcsjs-*" = "tcs-jetson";
    "sv-demo-*" = "tcs-jetson";
    "169.254.1.123" = "tcs-jetson";
    "*spadesvision.com" = "webdemo";
    "*webdemo*" = "webdemo";
  };

  identityBlocks = lib.mapAttrs (_pattern: dir: {
    IdentityFile = [
      "~/.ssh/spadesvision/${dir}/id_${dir}_spadesvision"
      "~/.ssh/spadesvision/${dir}/id_${dir}_root"
    ];
    IdentitiesOnly = true;
  }) identityDirs;
in
{
  options.profiles.ssh.enable = lib.mkEnableOption "personal ssh config (host aliases, identity files)";

  config = lib.mkIf cfg.enable {
    programs.ssh = {
      enable = true;
      enableDefaultConfig = false;
      settings = {
        "*".AddKeysToAgent = "yes";
        "github.com" = {
          User = "git";
          IdentityFile = "~/.ssh/id_github";
        };
        "git.sr.ht" = {
          User = "git";
          IdentityFile = "~/.ssh/id_sourcehut";
        };
        "beast" = {
          User = "frederik";
          IdentityFile = "~/.ssh/beast_login";
        };
        "tcs-local".HostName = "169.254.1.123";
      }
      // identityBlocks;
    };
  };
}
