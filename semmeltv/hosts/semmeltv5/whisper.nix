{ config, ... }:
{
  services.cage.environment.SEMMELTV_WHISPER =
    config.services.wyoming.faster-whisper.servers.semmeltv.uri;

  services.wyoming.faster-whisper.servers.semmeltv = {
    enable = true;
    uri = "tcp://127.0.0.1:10300";
    sttLibrary = "faster-whisper";
    model = "base-int8";
    language = "auto";
    zeroconf.enable = false;
  };
}
