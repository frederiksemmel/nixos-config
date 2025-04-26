{
  flake,
  pkgs,
  lib,
  ...
}:
{
  imports = [
    flake.inputs.disko.nixosModules.disko
    ./disko-btrfs.nix
    flake.inputs.nixos-hardware.nixosModules.tuxedo-infinitybook-pro14-gen7
  ];

  nixpkgs.config.allowUnfree = true;
  # make `nix run nixpkgs#nixpkgs` use the same nixpkgs as the one used by this flake.
  nix.registry.nixpkgs.flake = flake.inputs.nixpkgs;
  nix.channel.enable = false; # remove nix-channel related tools & configs, we use flakes instead.

  # but NIX_PATH is still used by many useful tools, so we set it to the same value as the one used by this flake.
  # Make `nix repl '<nixpkgs>'` use the same nixpkgs as the one used by this flake.
  environment.etc."nix/inputs/nixpkgs".source = "${flake.inputs.nixpkgs}";
  # https://github.com/NixOS/nix/issues/9574
  nix.settings.nix-path = lib.mkForce "nixpkgs=/etc/nix/inputs/nixpkgs";

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    hashedPassword = "$6$sBbqKGdIHSuqH2HL$voX0LJZaG/qu7y.0QUQYGSsadiHf99esXblI196WQNpvkeJf/P9L1BSoLJkCza/KQ/vCsibYteEHXXn0nMO7R0";
    shell = pkgs.nushell;
  };

  users.users.root.hashedPassword = "$6$TFVUB5p/gECWeZMb$GauIqbzHXTRitc6GHwlSGAzkfEpHqMoEnpu7dMK7.4gKNY4RIX3J2gBICORN4xeatkonTzKo0Q2wmyS4SQt7J.";

  programs.sway.enable = true;
  programs.niri.enable = true;

  environment.systemPackages = [ ];

  fonts.packages = with pkgs; [
    jetbrains-mono
    source-code-pro
    inconsolata
    iosevka
    intel-one-mono
    hack-font
    vistafonts # includes consolas
    noto-fonts
    noto-fonts-emoji
    fira-code
    fira-code-symbols
    nerd-fonts.fira-code
  ];
  fonts.enableDefaultPackages = true;

  # nixpkgs.overlays = [ flake.inputs.niri.overlays.niri ];
  # programs.niri.package = pkgs.niri-unstable;

  services.greetd = {
    enable = true;
    settings = {
      default_session = {
        command = ''
          ${pkgs.greetd.tuigreet}/bin/tuigreet \
            --time \
            --remember \
            --cmd ${pkgs.niri}/bin/niri-session
        '';
        user = "greeter";
      };
    };
  };
  # list the sessions that gtkgreet will offer
  environment.etc."greetd/environments".text = ''
    niri
    sway
  '';

  virtualisation = {
    memorySize = 1024 * 2;
    qemu = {
      options = [
        "-device virtio-gpu-gl-pci"
        "-display sdl,gl=on"
        "-vga none"
      ];
    };
  };

  system.stateVersion = "25.05";
}
