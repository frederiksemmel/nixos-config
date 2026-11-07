{
  flake,
  lib,
  nixos-raspberrypi,
  pkgs,
  ...
}:
let
  ffmpeg = pkgs.callPackage "${nixos-raspberrypi}/pkgs/ffmpeg_8-rpi.nix" {
    ffmpeg = pkgs.ffmpeg_8;
  };
  mpv = pkgs.mpv.override {
    mpv-unwrapped = pkgs.mpv-unwrapped.override { inherit ffmpeg; };
  };
  semmeltv = pkgs.callPackage ../../../packages/semmeltv {
    inherit mpv;
    inherit (flake.inputs) crane;
  };
  blankCursor = pkgs.callPackage ../../../packages/semmeltv/blank-cursor.nix { };
  stateDir = "/var/lib/semmeltv";
in
{
  imports = [ nixos-raspberrypi.nixosModules.sd-image ];

  boot.supportedFilesystems.zfs = lib.mkForce false;

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];
  nix.settings.trusted-users = [ "@wheel" ];

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFYY+tQCwMzcg13wu+aJbcK0uA2EWS89akoAgxnW2yRL frederiksemmel@gmail.com"
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOv3Vv3vTKjcqvmmsuTiYLA8pSb8WmaF6Ohn/LtgbOUw frederik@semmeltv"
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
      ]
      (_: {
        d = {
          user = "semmeltv";
          group = "users";
          mode = "2775";
        };
      });

  security.polkit.extraConfig = ''
    polkit.addRule(function(action, subject) {
      if (action.id == "org.freedesktop.systemd1.manage-units"
          && action.lookup("unit") == "cage-tty1.service"
          && subject.user == "frederik") {
        return polkit.Result.YES;
      }
    });
  '';

  services.cage = {
    enable = true;
    user = "semmeltv";
    program = lib.getExe semmeltv;
    environment = {
      SEMMELTV_LIBRARY = "${stateDir}/library";
      SEMMELTV_CACHE = "${stateDir}/cache";
      XCURSOR_THEME = "blank";
      XCURSOR_PATH = "${blankCursor}/share/icons";
    };
  };

  systemd.services.cage-tty1 = {
    after = [ "systemd-tmpfiles-setup.service" ];
    restartIfChanged = lib.mkForce true;
    serviceConfig = {
      Restart = "always";
      RestartSec = 2;
    };
  };

  systemd.services."getty@tty1".enable = false;
  systemd.services."autovt@tty1".enable = false;

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
