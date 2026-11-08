{ flake, nixos-raspberrypi, ... }:
{
  imports = with nixos-raspberrypi.nixosModules; [
    ../common.nix
    ./mount.nix
    ./wake.nix
    ./speech.nix
    flake.self.nixosModules.hailo
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
  ];

  networking.hostName = "semmeltv5";

  services.semmeltv.offlineCacheGiB = 80;

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
