{
  config,
  flake,
  nixos-raspberrypi,
  ...
}:
let
  wireguard = ../../../secrets/mullvad_wireguard.age;
  vpn = builtins.pathExists wireguard;
in
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
    };
    api.enable = true;
    web = {
      enable = true;
      tailscale = true;
      funnel = true;
    };
    downloads = {
      enable = true;
      wireguardConfigFile = if vpn then config.age.secrets.mullvad_wireguard.path else null;
      testWithoutVpn = !vpn;
    };
  };

  age.secrets = if vpn then { mullvad_wireguard.file = wireguard; } else { };

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
