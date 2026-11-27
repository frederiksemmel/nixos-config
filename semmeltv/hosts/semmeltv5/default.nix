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
    clubs = [
      5 # TA Fronwald
      6 # ASVZ Fluntern
      15 # TA Eichrain
      16 # TA Mythenquai
      17 # TA Hardhof
      18 # TA Frauental
      19 # Tennis Lengg, clay
      20 # Tennis Lengg, Rebound Ace hall
      21 # ASVZ Zürich
      39 # TC Riesbach
      56 # TC Wallisellen
      157 # TC Leimbach
      161 # TC Seeblick
      167 # TC Engematt
      180 # TC Uitikon
      193 # TC Waidberg
      204 # TC Swiss Re
      207 # TC Belvoir
      219 # TC Stallikon
      233 # TC Zollikon
      239 # TC Engstringen
      254 # FC Red Star
      268 # TC Mösli
      502 # TC Adliswil
      774 # TA Buchlern
      873 # TC Fluntern
      881 # TC Wasserversorgung
    ];
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
