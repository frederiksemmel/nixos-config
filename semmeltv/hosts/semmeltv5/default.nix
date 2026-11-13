{
  config,
  flake,
  nixos-raspberrypi,
  ...
}:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../common.nix
    ./mount.nix
    ./wake.nix
    ./phone-mic.nix
    ./speech.nix
    flake.self.nixosModules.hailo
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
    headphones.enable = true;
    api.enable = true;
    web = {
      enable = true;
      tailscale = true;
      lan = {
        domain = "semmeltv5.semmeltv.ch";
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

  hardware.hailo = {
    enable = true;
    generation = "hailo8";
  };
  nixpkgs.config.allowUnfreePackages = [ "hailo8-firmware" ];
  hardware.raspberry-pi.config.all.base-dt-params.pciex1_gen = {
    enable = true;
    value = "3";
  };
}
