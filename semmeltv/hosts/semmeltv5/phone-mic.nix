{
  boot.kernelModules = [ "snd-aloop" ];
  boot.extraModprobeConfig = "options snd-aloop id=Loopback pcm_substreams=1 index=-2";

  services.cage.environment.SEMMELTV_MICROPHONE = "plughw:Loopback,1,0";

  services.semmeltv.api.phoneMicrophone = "plughw:Loopback,0,0";
}
