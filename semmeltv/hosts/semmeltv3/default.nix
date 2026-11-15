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

  zramSwap = {
    enable = true;
    memoryPercent = 100;
  };

  services.semmeltv = {
    offlineCacheGiB = null;
    decoder = "v4l2m2m";
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
