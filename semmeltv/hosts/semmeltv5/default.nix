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

  networking.hostName = "semmeltv5";

  services.semmeltv = {
    offlineCacheGiB = 60;
    tv = {
      kind = "samsung";
      renderer = "http://192.168.1.148:9197/dmr";
      edid = ./samsung-edid.bin;
    };
    web = {
      tailscale = true;
      signIn = {
        tailnet = "cuscus-exponential.ts.net";
        serviceTokenFile = config.age.secrets.semmeltv_service_token.path;
      };
      lan = {
        domain = "semmeltv5.semmeltv.ch";
        public = true;
        forward = {
          "beachparty.semmeltv.ch" = "192.168.1.230:8445";
        };
        desecTokenFile = config.age.secrets.desec_token.path;
      };
    };
    downloads = {
      enable = true;
      wireguardConfigFile = config.age.secrets.mullvad_wireguard.path;
    };
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
