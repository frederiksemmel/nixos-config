{
  config,
  flake,
  lib,
  nixos-raspberrypi,
  pkgs,
  ...
}:
{
  imports = [ nixos-raspberrypi.nixosModules.sd-image ];

  boot.supportedFilesystems.zfs = lib.mkForce false;

  time.timeZone = "Europe/Zurich";

  system.configurationRevision = flake.self.rev or flake.self.dirtyRev or null;

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];
  nix.settings.trusted-users = [ "@wheel" ];

  nix.channel.enable = false;
  nix.nixPath = [ "nixpkgs=${pkgs.path}" ];
  nix.registry.nixpkgs.to = {
    type = "path";
    path = pkgs.path;
  };

  boot.loader.raspberry-pi.configurationLimit = 5;
  boot.loader.generic-extlinux-compatible.configurationLimit = 5;

  nix.gc = {
    automatic = true;
    dates = "04:30";
    persistent = true;
  };
  systemd.services.nix-gc.preStart = "${config.nix.package}/bin/nix-env --profile /nix/var/nix/profiles/system --delete-generations +5";

  environment.systemPackages = with pkgs; [
    bottom
    htop
    ncdu
    lsof
    curl
    jq
    dnsutils
    tmux
    iw
  ];

  networking.wireless.iwd.enable = true;

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com"
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOv3Vv3vTKjcqvmmsuTiYLA8pSb8WmaF6Ohn/LtgbOUw frederik@semmeltv"
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

  services.tailscale.enable = true;

  system.stateVersion = "26.05";
}
