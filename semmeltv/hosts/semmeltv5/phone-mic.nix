{
  config,
  lib,
  pkgs,
  ...
}:

let
  port = 8095;
  eventsPort = 8096;
  server = pkgs.callPackage ../../phone-mic { };
  tailscale = lib.getExe config.services.tailscale.package;
in
{
  boot.kernelModules = [ "snd-aloop" ];
  boot.extraModprobeConfig = "options snd-aloop id=Loopback pcm_substreams=1";

  services.cage.environment = {
    SEMMELTV_MICROPHONE = "plughw:Loopback,1,0";
    SEMMELTV_VOICE_FEEDBACK = "http://127.0.0.1:${toString eventsPort}/events";
  };

  systemd.services.semmeltv-phone-mic = {
    description = "Web page that streams a phone's microphone to SemmelTV";
    wantedBy = [ "multi-user.target" ];
    after = [ "systemd-modules-load.service" ];
    serviceConfig = {
      ExecStart = "${lib.getExe server} --port ${toString port} --events-port ${toString eventsPort} --device plughw:Loopback,0,0 --aplay ${pkgs.alsa-utils}/bin/aplay";
      DynamicUser = true;
      SupplementaryGroups = [ "audio" ];
      Restart = "on-failure";
      RestartSec = 5;
    };
  };

  systemd.services.semmeltv-phone-mic-https = {
    description = "Serve the phone microphone page over HTTPS on the tailnet";
    wantedBy = [ "multi-user.target" ];
    wants = [ "tailscaled.service" ];
    after = [ "tailscaled.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${tailscale} serve --bg --https=443 http://127.0.0.1:${toString port}";
      ExecStop = "${tailscale} serve --https=443 off";
    };
  };
}
