{ flake, lib, ... }:
let
  inherit (flake.inputs) nixos-raspberrypi;
in
{
  imports = with nixos-raspberrypi.nixosModules; [
    raspberry-pi-5.base
    sd-image
  ];

  networking.hostName = "semmeltv";

  boot.supportedFilesystems.zfs = lib.mkForce false;

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com"
    ];
  };
  security.sudo.wheelNeedsPassword = false;

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "no";
    };
  };

  system.stateVersion = "26.05";
}
