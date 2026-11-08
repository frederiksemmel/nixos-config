{ lib, pkgs, ... }:

let
  address = "127.0.0.1:10300";
  server = pkgs.callPackage ../../parakeet { };
  model = pkgs.callPackage ../../parakeet/model.nix { };
in
{
  services.cage.environment.SEMMELTV_SPEECH = "tcp://${address}";

  systemd.services.semmeltv-speech = {
    description = "Parakeet speech-to-text for voice commands";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      ExecStart = "${lib.getExe server} --uri tcp://${address} --model ${model}";
      DynamicUser = true;
      Restart = "on-failure";
      RestartSec = 5;
    };
  };
}
