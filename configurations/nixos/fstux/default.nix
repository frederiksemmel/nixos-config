{ flake, ... }:
{
  imports = [
    flake.inputs.disko.nixosModules.disko
    ./disko-btrfs.nix
    flake.inputs.nixos-hardware.nixosModules.tuxedo-infinitybook-pro14-gen7
  ];
  nixpkgs.hostPlatform = "x86_64-linux";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [
      "wheel"
    ];
  };

  system.stateVersion = "25.05";
}
