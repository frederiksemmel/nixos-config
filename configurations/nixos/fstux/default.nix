{
  flake,
  pkgs,
  lib,
  config,
  ...
}:
{
  imports = with flake.inputs; [
    niri.nixosModules.niri
    agenix.nixosModules.default
    disko.nixosModules.disko
    ./disko-btrfs.nix
    nixos-hardware.nixosModules.tuxedo-infinitybook-pro14-gen7
  ];

  home-manager.users.frederik = {
    imports = [ flake.self.homeModules.default ];
    home.stateVersion = "25.05";
  };
  home-manager.backupFileExtension = "backup";
  home-manager.useGlobalPkgs = true;

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

  hardware.graphics = {
    enable = true;
  };
  hardware.enableRedistributableFirmware = true;

  networking.useNetworkd = true;
  networking.useDHCP = false;
  networking.wireless.iwd.enable = true;

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    hashedPassword = "$6$sBbqKGdIHSuqH2HL$voX0LJZaG/qu7y.0QUQYGSsadiHf99esXblI196WQNpvkeJf/P9L1BSoLJkCza/KQ/vCsibYteEHXXn0nMO7R0";
    shell = pkgs.nushell;
  };

  users.users.root.hashedPassword = "$6$TFVUB5p/gECWeZMb$GauIqbzHXTRitc6GHwlSGAzkfEpHqMoEnpu7dMK7.4gKNY4RIX3J2gBICORN4xeatkonTzKo0Q2wmyS4SQt7J.";

  programs.sway.enable = true;
  programs.niri.enable = true;
  programs.ssh.startAgent = true;

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

  system.stateVersion = "25.05";
}
