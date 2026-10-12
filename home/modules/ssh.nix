{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.ssh;
  bitwardenAgent = config.profiles.bitwarden.sshAgent.enable;
  identity = path: if bitwardenAgent then "${path}.pub" else path;
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
    IdentityFile = map identity [
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
        "*".AddKeysToAgent = if bitwardenAgent then "no" else "yes";
        "github.com" = {
          User = "git";
          IdentityFile = identity "~/.ssh/id_github";
          IdentitiesOnly = true;
        };
        "git.sr.ht" = {
          User = "git";
          IdentityFile = identity "~/.ssh/id_sourcehut";
          IdentitiesOnly = true;
        };
        "beast" = {
          User = "frederik";
          IdentityFile = identity "~/.ssh/beast_login";
          IdentitiesOnly = true;
        };
        "tcs-local".HostName = "169.254.1.123";
      }
      // identityBlocks;
    };
  };
}
