{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.ssh;
  entries = [
    {
      name = "spadesvision-bern";
      pattern = "*bern*";
      dir = "spadesvision";
    }
    {
      name = "spadesvision-pfaeffikon";
      pattern = "*pfaeffikon*";
      dir = "spadesvision";
    }
    {
      name = "spadesvision-zurich";
      pattern = "*zurich*";
      dir = "spadesvision";
    }
    {
      name = "spadesvision-office";
      pattern = "*office*";
      dir = "spadesvision";
    }
    {
      name = "spadesvision-badragaz";
      pattern = "*badragaz*";
      dir = "spadesvision";
    }
    {
      name = "tcs";
      pattern = "*tcs*";
      dir = "tcs-jetson";
    }
    {
      name = "ags";
      pattern = "*ags*";
      dir = "tcs-jetson";
    }
    {
      name = "webdemo";
      pattern = "*spadesvision.com";
      dir = "webdemo";
    }
    {
      name = "webdemo";
      pattern = "*webdemo*";
      dir = "webdemo";
    }
  ];

  asBlocks = lib.foldl' (
    acc: e:
    acc
    // {
      "${e.name}" = {
        host = e.pattern;
        identityFile = [
          "~/.ssh/spadesvision/${e.dir}/id_${e.dir}_spadesvision"
          "~/.ssh/spadesvision/${e.dir}/id_${e.dir}_root"
        ];
        identitiesOnly = true;
      };
    }
  ) { };
in
{
  options.profiles.ssh.enable = lib.mkEnableOption "personal ssh config (host aliases, identity files)";

  config = lib.mkIf cfg.enable {
    programs.ssh = {
      enable = true;
      enableDefaultConfig = false;
      matchBlocks = {
        "*".addKeysToAgent = "yes";
        "github.com" = {
          user = "git";
          identityFile = "~/.ssh/id_github";
        };
        "git.sr.ht" = {
          user = "git";
          identityFile = "~/.ssh/id_sourcehut";
        };
      }
      // asBlocks entries;
    };
  };
}
