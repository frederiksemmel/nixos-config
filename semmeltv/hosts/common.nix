{
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

  nix.channel.enable = false;
  nix.nixPath = [ "nixpkgs=${pkgs.path}" ];
  nix.registry.nixpkgs.to = {
    type = "path";
    path = pkgs.path;
  };

  environment.systemPackages = with pkgs; [
    bottom
    htop
    ncdu
    lsof
    curl
    jq
    dnsutils
    tmux
  ];

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
