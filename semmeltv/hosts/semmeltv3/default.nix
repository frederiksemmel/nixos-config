{
  config,
  nixos-raspberrypi,
  ...
}:
{
  imports = [
    ../common.nix
    ../mount.nix
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
  ];

  networking.hostName = "semmeltv3";

  age.secrets.wifi_joincena.file = ../../../secrets/wifi_joincena.age;
  systemd.services.iwd-joincena = {
    wantedBy = [ "iwd.service" ];
    before = [ "iwd.service" ];
    serviceConfig = {
      Type = "oneshot";
      UMask = "0077";
    };
    script = ''
      mkdir -p /var/lib/iwd
      passphrase=$(sed -n 's/^psk_joincena=//p' ${config.age.secrets.wifi_joincena.path})
      printf '[Security]\nPassphrase=%s\n' "$passphrase" > /var/lib/iwd/joincena.psk
    '';
  };

  zramSwap = {
    enable = true;
    memoryPercent = 100;
  };

  services.semmeltv = {
    offlineCacheGiB = null;
    decoder = "v4l2m2m";
    tv = {
      renderer = "http://192.168.1.148:9197/dmr";
      decodes = [
        "hevc"
        "av1"
        "vp9"
      ];
    };
    headphones.enable = true;
    api.enable = true;
    web = {
      enable = true;
      tailscale = true;
      signIn = {
        tailnet = "cuscus-exponential.ts.net";
        serviceTokenFile = config.age.secrets.semmeltv_service_token.path;
      };
      lan = {
        domain = "semmeltv3.semmeltv.ch";
        public = true;
        relayedFrom = "192.168.1.178";
        desecTokenFile = config.age.secrets.desec_token.path;
      };
    };
    downloads = {
      enable = true;
      wireguardConfigFile = config.age.secrets.mullvad_wireguard.path;
    };
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard_semmeltv3.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
