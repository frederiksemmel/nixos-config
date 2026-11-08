{
  config,
  lib,
  pkgs,
  ...
}:

let
  address = "127.0.0.1:10300";
  server = pkgs.callPackage ../../hailo-whisper {
    inherit (config.hardware.hailo.packages) pyhailort;
  };
  model = pkgs.callPackage ../../hailo-whisper/model.nix { };
in
{
  services.cage.environment.SEMMELTV_WHISPER = "tcp://${address}";

  systemd.services.semmeltv-whisper = {
    description = "Whisper speech-to-text with its encoder on the Hailo-8L";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-modules-load.service" ];
    serviceConfig = {
      ExecStart = "${lib.getExe server} --uri tcp://${address} --model ${model}";
      DynamicUser = true;
      RuntimeDirectory = "semmeltv-whisper";
      WorkingDirectory = "%t/semmeltv-whisper";
      Environment = "HOME=%t/semmeltv-whisper";
      Restart = "on-failure";
      RestartSec = 5;
    };
  };
}
