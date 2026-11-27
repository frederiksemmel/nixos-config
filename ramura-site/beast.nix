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
        pkgs.gnutar
        pkgs.gzip
        pkgs.jq
        pkgs.openssh
        pkgs.python3
      ];
      runtimeEnv = {
        RAMURA_REPO = "ragassler/ragassler.github.io";
        RAMURA_CHECKOUT = "${config.home.homeDirectory}/repos/ragassler.github.io";
        PREVIEW_HOST = "semmeltv5";
        PREVIEW_DOMAIN = "ramura.semmeltv.ch";
      };
      text = builtins.readFile ./feedback.sh;
    };
    repo = "${config.home.homeDirectory}/repos/ragassler.github.io";
    prompt = ./prompt.md;
  };
}
