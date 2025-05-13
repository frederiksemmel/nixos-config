{
  flake,
  pkgs,
  lib,
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
  nix.settings.trusted-users = [
    "root"
    "frederik"
  ];

  # but NIX_PATH is still used by many useful tools, so we set it to the same value as the one used by this flake.
  # Make `nix repl '<nixpkgs>'` use the same nixpkgs as the one used by this flake.
  environment.etc."nix/inputs/nixpkgs".source = "${flake.inputs.nixpkgs}";
  # https://github.com/NixOS/nix/issues/9574
  nix.settings.nix-path = lib.mkForce "nixpkgs=/etc/nix/inputs/nixpkgs";

  time.timeZone = "Europe/Zurich";
  security.sudo-rs.enable = true;

  programs.ssh.extraConfig = ''
    Host eu.nixbuild.net
      PubkeyAcceptedKeyTypes ssh-ed25519
      ServerAliveInterval 60
      IPQoS throughput
      IdentityFile /home/frederik/.ssh/my-nixbuild-key
  '';

  programs.ssh.knownHosts = {
    nixbuild = {
      hostNames = [ "eu.nixbuild.net" ];
      publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPIQCZc54poJ8vqawd8TraNryQeJnvH1eLpIDgbiqymM";
    };
  };

  nix = {
    distributedBuilds = true;
    buildMachines = [
      {
        hostName = "eu.nixbuild.net";
        system = "aarch64-linux";
        maxJobs = 100;
        supportedFeatures = [
          "benchmark"
          "big-parallel"
        ];
      }
    ];
  };

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;

  hardware.graphics = {
    enable = true;
  };
  hardware.enableRedistributableFirmware = true;
  hardware.tuxedo-rs = {
    enable = true;
    tailor-gui.enable = true;
  };

  networking.hostName = "fstux";
  networking.useNetworkd = true;
  systemd.network.networks."25-wlan" = {
    matchConfig.Type = "wlan";
    networkConfig = {
      DHCP = "yes";
      MulticastDNS = "yes";
      DNS = "9.9.9.9 1.1.1.1";
    };
  };
  networking.useDHCP = false;
  networking.wireless.iwd.enable = true;
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [
      80
      443
      5173
      8765
    ];
    allowedUDPPorts = [
      5353
      5355
    ];
    allowedUDPPortRanges = [
      {
        from = 4000;
        to = 4007;
      }
      {
        from = 8000;
        to = 8010;
      }
    ];
  };

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [
      "wheel"
      "video"
      "input"
    ];
    hashedPassword = "$6$sBbqKGdIHSuqH2HL$voX0LJZaG/qu7y.0QUQYGSsadiHf99esXblI196WQNpvkeJf/P9L1BSoLJkCza/KQ/vCsibYteEHXXn0nMO7R0";
    shell = pkgs.nushell;
  };

  users.users.root.hashedPassword = "$6$TFVUB5p/gECWeZMb$GauIqbzHXTRitc6GHwlSGAzkfEpHqMoEnpu7dMK7.4gKNY4RIX3J2gBICORN4xeatkonTzKo0Q2wmyS4SQt7J.";

  programs.sway.enable = true;
  programs.niri.enable = true;
  nixpkgs.overlays = [ flake.inputs.niri.overlays.niri ];
  programs.niri.package = pkgs.niri-unstable;

  programs.ssh.startAgent = true;
  programs.light.enable = true;
  programs.gnupg.agent = {
    enable = true;
    pinentryPackage = pkgs.pinentry-curses;
  };

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

  services.devmon.enable = true;
  services.tailscale.enable = true;
  services.udev.extraRules = ''
    #Enable user access to all basler cameras
    SUBSYSTEM=="usb", ATTRS{idVendor}=="2676", MODE:="0666", TAG+="uaccess", TAG+="udev-acl"
  '';
  services.printing.enable = true;
  # services.avahi = {
  #   enable = true;
  #   nssmdns4 = true;
  #   openFirewall = true;
  # };

  services.greetd = {
    enable = true;
    settings = {
      default_session = {
        command = ''
          ${pkgs.greetd.tuigreet}/bin/tuigreet \
            --time \
            --remember \
            --cmd ${pkgs.niri-unstable}/bin/niri-session
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
