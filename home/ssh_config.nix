{ lib, ... }:
let
  entries = [
    {
      name = "spadesvision-bern";
      pattern = "*bern*";
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
      name = "tcs";
      pattern = "*tcs*";
      dir = "tcs-jetson";
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
}
