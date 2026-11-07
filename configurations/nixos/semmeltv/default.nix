{
  flake,
  lib,
  pkgs,
  ...
}:
let
  inherit (flake.inputs) nixos-raspberrypi;
  semmeltv = pkgs.callPackage ../../../packages/semmeltv { };
  stateDir = "/var/lib/semmeltv";
in
{
  imports = with nixos-raspberrypi.nixosModules; [
    raspberry-pi-5.base
    raspberry-pi-5.display-vc4
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

  users.users.semmeltv = {
    isNormalUser = true;
    home = stateDir;
    createHome = false;
    extraGroups = [
      "video"
      "render"
      "input"
    ];
  };

  systemd.tmpfiles.settings.semmeltv =
    lib.genAttrs
      [
        stateDir
        "${stateDir}/cache"
        "${stateDir}/library"
        "${stateDir}/library/Movies"
        "${stateDir}/library/Series"
      ]
      (_: {
        d = {
          user = "semmeltv";
          group = "users";
          mode = "0755";
        };
      });

  services.cage = {
    enable = true;
    user = "semmeltv";
    program = lib.getExe semmeltv;
    environment = {
      SEMMELTV_LIBRARY = "${stateDir}/library";
      SEMMELTV_CACHE = "${stateDir}/cache";
    };
  };

  systemd.services.cage-tty1 = {
    after = [ "systemd-tmpfiles-setup.service" ];
    serviceConfig = {
      Restart = "always";
      RestartSec = 2;
    };
  };

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
