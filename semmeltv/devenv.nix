{ config, pkgs, ... }:
let
  buildEnv = import ./app/build-env.nix pkgs;
  parakeet = pkgs.callPackage ./parakeet { };
  parakeetModel = pkgs.callPackage ./parakeet/model.nix { };
  speechPort = toString config.processes.speech.ports.wyoming.value;
  wakePort = toString config.processes.wake.ports.wyoming.value;
  listening = port: {
    exec = "exec 3<>/dev/tcp/127.0.0.1/${port}";
    period = 1;
  };
in
{
  languages.rust = {
    enable = true;
    rustflags = buildEnv.RUSTFLAGS;
  };

  packages = with pkgs; [
    alsa-utils
    bacon
    fontconfig
    mpv
    pkg-config
    slint-lsp
    wlr-randr
  ];

  env = {
    SEMMELTV_FONT_DIR = buildEnv.SEMMELTV_FONT_DIR;
    SEMMELTV_SPEECH = "tcp://127.0.0.1:${speechPort}";
    SEMMELTV_WAKE = "tcp://127.0.0.1:${wakePort}";
    SEMMELTV_MICROPHONE = "default";
  };

  processes = {
    speech = {
      ports.wyoming.allocate = 10300;
      exec = "${pkgs.lib.getExe parakeet} --uri tcp://127.0.0.1:${speechPort} --model ${parakeetModel}";
      ready = listening speechPort;
    };
    wake = {
      ports.wyoming.allocate = 10400;
      exec = "${pkgs.lib.getExe pkgs.wyoming-openwakeword} --uri tcp://127.0.0.1:${wakePort}";
      ready = listening wakePort;
    };
  };

  enterShell = ''
    export SEMMELTV_LIBRARY="''${SEMMELTV_LIBRARY:-$HOME/pcloud/SemmelTV}"
  '';
}
