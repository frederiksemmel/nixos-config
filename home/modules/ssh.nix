{
  config,
  lib,
  ...
}:
let
  cfg = config.profiles.ssh;
  bitwardenAgent = config.profiles.bitwarden.sshAgent.enable;
  identity = path: if bitwardenAgent then "${path}.pub" else path;
  # The Spades Vision host blocks name customer sites, so they live in an
  # agenix secret. Its two variants differ only in IdentityFile naming the key
  # (spadesvision_ssh_config.age) or its .pub (spadesvision_ssh_config_agent.age),
  # like `identity` does; edit both together.
  spadesvisionConfig = "${config.home.homeDirectory}/.ssh/spadesvision_config";
in
{
  options.profiles.ssh.enable = lib.mkEnableOption "personal ssh config (host aliases, identity files)";

  config = lib.mkIf cfg.enable {
    age.secrets.spadesvision_ssh_config = {
      file =
        if bitwardenAgent then
          ../../secrets/spadesvision_ssh_config_agent.age
        else
          ../../secrets/spadesvision_ssh_config.age;
      path = spadesvisionConfig;
    };

    age.secrets.semmeltv_ssh_key = {
      file = ../../secrets/semmeltv_ssh_key.age;
      path = "${config.home.homeDirectory}/.ssh/id_semmeltv";
    };

    programs.ssh = {
      enable = true;
      enableDefaultConfig = false;
      # ssh skips an Include whose file is missing, e.g. before agenix ran.
      includes = [ spadesvisionConfig ];
      settings = {
        "*" = {
          AddKeysToAgent = if bitwardenAgent then "no" else "yes";
          SendEnv = [
            "TERM"
            "COLORTERM"
          ];
        };
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
        "semmeltv*" = {
          User = "frederik";
          IdentityFile = "~/.ssh/id_semmeltv";
          IdentitiesOnly = true;
          StrictHostKeyChecking = "accept-new";
        };
      };
    };
  };
}
