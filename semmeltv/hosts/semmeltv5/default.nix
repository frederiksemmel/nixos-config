{
  config,
  flake,
  nixos-raspberrypi,
  ...
}:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../common.nix
    ../mount.nix
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
    flake.self.nixosModules.courts
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
          "semmeltvmami.semmeltv.ch" = "192.168.1.164:8445";
          ${config.services.courts.domain} = "127.0.0.1:${toString config.services.courts.port}";
        };
        desecTokenFile = config.age.secrets.desec_token.path;
      };
    };
    downloads = {
      enable = true;
      wireguardConfigFile = config.age.secrets.mullvad_wireguard.path;
    };
  };

  services.courts = {
    enable = true;
    domain = "courts.semmeltv.ch";
    target = "semmeltv5.semmeltv.ch";
    desecTokenFile = config.age.secrets.desec_token.path;
    # Missing from GotCourts' map itself; both members-only, summer only.
    clubs = [
      52805 # Vontobel Tennis Club
      53170 # Tennis-Sportclub Zürich
    ];
    exclude = [
      53610 # GCL 2024 Finalturnier, a tournament placeholder
      26 # Im Hau, an old copy of 45026 TC Im Hau Witikon
    ];
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
