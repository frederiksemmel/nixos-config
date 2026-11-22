{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv;
  bluealsa = pkgs.callPackage ../app/bluez-alsa.nix { };
in
{
  config = lib.mkIf cfg.enable {
    hardware.bluetooth = {
      enable = true;
      powerOnBoot = true;
      settings.Policy.ReconnectAttempts = 0;
    };

    users.users.bluealsa = {
      isSystemUser = true;
      group = "audio";
    };

    services.dbus.packages = [ bluealsa ];
    systemd.packages = [ bluealsa ];
    systemd.services.bluealsa = {
      wantedBy = [ "bluetooth.target" ];
      serviceConfig.ExecStart = [
        ""
        "${bluealsa}/bin/bluealsa -S -p a2dp-source -c +aac --a2dp-volume"
      ];
    };

    environment.etc."asound.conf".text = ''
      pcm_type.bluealsa.lib "${bluealsa}/lib/alsa-lib/libasound_module_pcm_bluealsa.so"
      ctl_type.bluealsa.lib "${bluealsa}/lib/alsa-lib/libasound_module_ctl_bluealsa.so"
      <${bluealsa}/etc/alsa/conf.d/20-bluealsa.conf>
    '';

    environment.systemPackages = [ bluealsa ];

    services.semmeltv.coreEnvironment.SEMMELTV_HEADPHONES = "bluetooth";
  };
}
