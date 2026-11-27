{ pkgs, ... }:
{
  services.feedback-hub.projects.courts = {
    adapter = pkgs.writeShellApplication {
      name = "feedback-courts";
      runtimeInputs = [
        pkgs.coreutils
        pkgs.gnused
        pkgs.jq
        pkgs.openssh
      ];
      runtimeEnv = {
        COURTS_HOST = "semmeltv5";
        COURTS_PORT = "8096";
      };
      excludeShellChecks = [ "SC2016" ];
      text = builtins.readFile ./feedback.sh;
    };
    prompt = ./prompt.md;
  };
}
