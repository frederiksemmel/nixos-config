{ config, pkgs, ... }:
{
  services.feedback-hub.projects.ramura-site = {
    adapter = pkgs.writeShellApplication {
      name = "feedback-ramura-site";
      runtimeInputs = [
        pkgs.coreutils
        pkgs.gh
        pkgs.git
        pkgs.gnugrep
        pkgs.jq
      ];
      runtimeEnv = {
        RAMURA_REPO = "ragassler/ragassler.github.io";
        RAMURA_CHECKOUT = "${config.home.homeDirectory}/repos/ragassler.github.io";
      };
      text = builtins.readFile ./feedback.sh;
    };
    repo = "${config.home.homeDirectory}/repos/ragassler.github.io";
    prompt = ./prompt.md;
  };
}
