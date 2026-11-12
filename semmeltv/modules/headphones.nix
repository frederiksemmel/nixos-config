{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.semmeltv;
  bluealsa = pkgs.bluez-alsa;
in
{
  options.services.semmeltv.headphones.enable = lib.mkEnableOption "Bluetooth headphones the app can send its sound to instead of the TV, through BlueALSA. They are paired from the web app's Settings and kept in /var/lib/bluetooth";

  config = lib.mkIf (cfg.enable && cfg.headphones.enable) {
    hardware.bluetooth = {
      enable = true;
      powerOnBoot = true;
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

    services.cage.environment.SEMMELTV_HEADPHONES = "bluetooth";
  };
}
