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

  tasks."semmeltv:warm-library" = {
    description = "List the pCloud library folder in one call so the app loads in seconds";
    before = [ "devenv:enterShell" ];
    exec = ''
      socket="$XDG_RUNTIME_DIR/rclone-pcloud.sock"
      library="''${SEMMELTV_LIBRARY:-$HOME/pcloud/SemmelTV}"
      case "$library" in
        "$HOME/pcloud/"*) ;;
        *) exit 0 ;;
      esac
      if [ ! -S "$socket" ]; then
        echo "no rclone rc socket at $socket; the library loads folder by folder"
        exit 0
      fi
      ${pkgs.lib.getExe pkgs.rclone} rc --unix-socket "$socket" vfs/refresh \
        dir="''${library#"$HOME/pcloud/"}" recursive=true > /dev/null \
        || echo "warming $library failed; the library loads folder by folder"
    '';
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
