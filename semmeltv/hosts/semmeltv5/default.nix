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
      # Tennis inside the city of Zurich (swisstopo boundary), bookable on GotCourts
      21 # ASVZ Zürich
      53253 # ATC Zürich
      52805 # Vontobel Tennis Club
      53170 # Tennis-Sportclub Zürich
      53223 # TC Fairplay Zürich
      254 # FC Red Star Tennis Sektion
      53138 # TC Zürich
      207 # TC Belvoir
      53785 # Belvoir Indoors
      6 # ASVZ Fluntern
      873 # TC Fluntern
      16 # TA Mythenquai
      57 # TC Hakoah Zürich
      193 # TC Waidberg
      18 # TA Frauental
      45422 # TC Oerlikon
      167 # TC Engematt
      17 # TA Hardhof
      204 # TC Swiss Re
      39 # TC Riesbach
      20 # Tennis Lengg
      19 # Tennis Lengg
      53195 # TC Bachtobel
      881 # TC Wasserversorgung Zürich
      221 # TC Hönggerberg
      53351 # TC Wollishofen
      774 # TA Buchlern
      45026 # TC Im Hau Witikon
      161 # TC Seeblick
      53725 # Hallen Tennis Fronwald
      5 # TA Fronwald
      53073 # Tennisclub Seebach
      15 # TA Eichrain
      157 # TC Leimbach
      # Squash and padel: none inside the city books on GotCourts; the nearest ones
      53314 # Sportzentrum Sanapark Squash
      53142 # Indoorsport Aesch ZH
      53001 # TSM Grindel
      213 # Tennis- und Padelclub Unterengstringen
      200 # TC Hausen a/A
    ];
  };

  age.secrets.mullvad_wireguard.file = ../../../secrets/mullvad_wireguard.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;
  age.secrets.semmeltv_service_token = {
    file = ../../../secrets/semmeltv_service_token.age;
    owner = "semmeltv";
  };
}
