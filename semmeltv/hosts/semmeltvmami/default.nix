{
  config,
  nixos-raspberrypi,
  ...
}:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../common.nix
    ../mount.nix
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
  ];

  networking.hostName = "semmeltvmami";

  services.semmeltv = {
    offlineCacheGiB = 15;
    web = {
      tailscale = true;
      signIn = {
        tailnet = "cuscus-exponential.ts.net";
        serviceTokenFile = config.age.secrets.semmeltv_service_token.path;
      };
      lan = {
        domain = "semmeltvmami.semmeltv.ch";
        public = true;
        relayedFrom = "192.168.1.178";
        desecTokenFile = config.age.secrets.desec_token.path;
      };
    };
    downloads = {
      enable = true;
      quotaGiB = 8;
      wireguardConfigFile = config.age.secrets.mullvad_wireguard.path;
    };
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard_semmeltvmami.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
