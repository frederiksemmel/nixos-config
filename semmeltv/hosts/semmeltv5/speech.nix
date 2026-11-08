{ lib, pkgs, ... }:

let
  address = "127.0.0.1:10300";
  led = "/sys/class/leds/ACT";
  server = pkgs.callPackage ../../parakeet { };
  model = pkgs.callPackage ../../parakeet/model.nix { };
in
{
  services.cage.environment = {
    SEMMELTV_SPEECH = "tcp://${address}";
    SEMMELTV_LED = led;
  };

  systemd.services.semmeltv-led = {
    description = "Hand the green activity LED to SemmelTV's voice control";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = ''
      echo none > ${led}/trigger
      echo 0 > ${led}/brightness
      chgrp video ${led}/brightness
      chmod g+w ${led}/brightness
    '';
  };

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
