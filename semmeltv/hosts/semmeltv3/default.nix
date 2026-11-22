{
  config,
  flake,
  nixos-raspberrypi,
  ...
}:
{
  imports = [
    ../common.nix
    ../mount.nix
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
    flake.self.nixosModules.epaper
    flake.self.nixosModules.beachparty
  ];

  services.epaper = {
    enable = true;
    tokenFile = config.age.secrets.epaper_telegram_token.path;
    rotation = 270;
    whatsapp.enable = true;
  };
  users.users.frederik.extraGroups = [ "epaper" ];

  services.beachparty = {
    enable = true;
    domain = "beachparty.semmeltv.ch";
    target = "semmeltv3.semmeltv.ch";
    relayedFrom = "192.168.1.178";
    desecTokenFile = config.age.secrets.desec_token.path;
  };
  age.secrets.epaper_telegram_token.file = ../../../secrets/epaper_telegram_token.age;

  networking.hostName = "semmeltv3";

  systemd.services.power-led-off = {
    description = "Turn off the red power LED";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = ''
      echo none > /sys/class/leds/PWR/trigger
      echo 0 > /sys/class/leds/PWR/brightness
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
