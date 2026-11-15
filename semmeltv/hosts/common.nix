{
  config,
  flake,
  nixos-raspberrypi,
  pkgs,
  ...
}:
let
  ffmpeg = pkgs.callPackage "${nixos-raspberrypi}/pkgs/ffmpeg_8-rpi.nix" {
    ffmpeg = pkgs.ffmpeg_8;
  };
in
{
  imports = [
    ../modules/pi-host.nix
    flake.self.nixosModules.semmeltv
  ];

  time.timeZone = "Europe/Zurich";

  system.configurationRevision = flake.self.rev or flake.self.dirtyRev or null;

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

  services.semmeltv = {
    enable = true;
    adminUser = "frederik";
    mpv = pkgs.mpv.override {
      mpv-unwrapped = (pkgs.mpv-unwrapped.override { inherit ffmpeg; }).overrideAttrs (old: {
        patches = (old.patches or [ ]) ++ [ ./mpv-gpu-next-gl-fence-leak.patch ];
      });
    };
  };
}
