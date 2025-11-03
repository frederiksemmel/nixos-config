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
    determinate.nixosModules.default
    # tuxedo-nixos.nixosModules.default
  ];

  nixpkgs.config.allowUnfree = true;
  nixpkgs.overlays = with flake.inputs; [ niri.overlays.niri ];

  home-manager.users.frederik = {
    imports = [ flake.self.homeModules.default ];
    home.stateVersion = "25.05";
  };
  home-manager.backupFileExtension = "backup";
  home-manager.useGlobalPkgs = true;

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
  nix.settings = {
    nix-path = lib.mkForce "nixpkgs=/etc/nix/inputs/nixpkgs";
    # substituters = [ "ssh://eu.nixbuild.net" ];
    # trusted-public-keys = [ "nixbuild.net/MCCPN0-1:T5gfNdFwM+2PutSR36OoYdHO6bL0tUGbjlbDl8KOGjk=" ];
  };

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
        speedFactor = 4;
        supportedFeatures = [
          "benchmark"
          "big-parallel"
        ];
      }
    ];
  };

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
  boot.binfmt.emulatedSystems = [ "aarch64-linux" ];
  # boot.kernelParams = [ "console=tty2" ];

  hardware.graphics = {
    enable = true;
  };
  hardware.enableRedistributableFirmware = true;
  hardware.tuxedo-rs = {
    enable = true;
    tailor-gui.enable = true;
  };
  # hardware.tuxedo-control-center.enable = true;
  hardware.bluetooth.enable = true; # enables support for Bluetooth
  services.blueman.enable = true;
  # services.blueman.enable = true;

  # hardware.logitech.wireless.enable = true;
  # hardware.logitech.wireless.enableGraphical = true;

  networking.hostName = "fstux";
  networking.useNetworkd = true;
  systemd.network.networks."25-wlan" = {
    matchConfig.Type = "wlan";
    networkConfig = {
      DHCP = "yes";
      MulticastDNS = "yes";
      # NOTE remove this in public networks
      DNS = "9.9.9.9 1.1.1.1";
    };
  };
  systemd.network.networks."10-usb-lan" = {
    matchConfig.Type = "ether";
    networkConfig = {
      DHCP = "yes";
      LinkLocalAddressing = "yes";
      # Resolved config and avahi needed for mdns for webrtc. Our app needs to resolve ....local addresses
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
      9876
      53317
    ];
    allowedUDPPorts = [
      5353
      5355
      41641
      3478
    ];
    extraCommands = ''
      iptables -A nixos-fw -p tcp --source 169.254.0.0/16 --dport 1024:65535 -j nixos-fw-accept
      iptables -A nixos-fw -p udp --source 169.254.0.0/16 --dport 1024:65535 -j nixos-fw-accept
    '';
  };

  users.users.frederik = {
    isNormalUser = true;
    extraGroups = [
      "wheel"
      "video"
      "input"
      "docker"
    ];
    hashedPassword = "$6$sBbqKGdIHSuqH2HL$voX0LJZaG/qu7y.0QUQYGSsadiHf99esXblI196WQNpvkeJf/P9L1BSoLJkCza/KQ/vCsibYteEHXXn0nMO7R0";
    shell = pkgs.nushell;
  };

  users.users.root.hashedPassword = "$6$TFVUB5p/gECWeZMb$GauIqbzHXTRitc6GHwlSGAzkfEpHqMoEnpu7dMK7.4gKNY4RIX3J2gBICORN4xeatkonTzKo0Q2wmyS4SQt7J.";

  programs.light.enable = true;
  programs.gnupg.agent = {
    enable = true;
    pinentryPackage = pkgs.pinentry-curses;
  };

  environment.systemPackages = [ ];

  xdg.portal = {
    config.common = {
      default = [
        "gnome"
        "gtk"
      ];
      # Electron apps and chromium fail to open the file picker for some reason.
      "org.freedesktop.impl.portal.FileChooser" = "gtk";
      "org.freedesktop.impl.portal.Access" = "gtk";
      "org.freedesktop.impl.portal.Notification" = "gtk";
    };
  };
  systemd.user.services.niri-flake-polkit.enable = false;
  systemd.user.services.polkit-gnome-authentication-agent-1 = {
    description = "polkit-gnome-authentication-agent-1";
    wantedBy = [ "graphical-session.target" ];
    wants = [ "graphical-session.target" ];
    after = [ "graphical-session.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "${pkgs.polkit_gnome}/libexec/polkit-gnome-authentication-agent-1";
      Restart = "on-failure";
      RestartSec = 1;
      TimeoutStopSec = 10;
    };
  };

  fonts.packages = with pkgs; [
    jetbrains-mono
    source-code-pro
    inconsolata
    iosevka
    intel-one-mono
    hack-font
    vista-fonts # includes consolas
    noto-fonts
    noto-fonts-color-emoji
    fira-code
    fira-code-symbols
    nerd-fonts.fira-code
  ];
  fonts.enableDefaultPackages = true;

  services.devmon.enable = true;
  services.tailscale.enable = true;
  services.mullvad-vpn.enable = true;
  services.udev.extraRules = ''
    #Enable user access to all basler cameras
    SUBSYSTEM=="usb", ATTRS{idVendor}=="2676", MODE:="0666", TAG+="uaccess", TAG+="udev-acl"
  '';
  services.printing.enable = true;
  services.avahi = {
    enable = true;
    nssmdns4 = true;
    openFirewall = true;
  };
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    alsa.support32Bit = true;
    pulse.enable = true;
  };

  services.greetd = {
    enable = true;
    settings = {
      default_session = {
        command = ''
          ${pkgs.tuigreet}/bin/tuigreet \
            --time \
            --remember \
            --cmd ${pkgs.niri-unstable}/bin/niri-session
        '';
        user = "greeter";
      };
    };
  };

  programs.sway.enable = true;

  programs.niri.enable = true;
  programs.niri.package = pkgs.niri-unstable;

  services.displayManager = {
    # sessionPackages = with pkgs; [ niri-unstable ];
    # sddm = {
    #   enable = true;
    #   wayland.enable = true;
    # };
    # ly = {
    #   enable = true;
    # };
  };

  # Enable SSH daemon
  services.openssh.enable = false;

  # Optional: configure SSH settings
  services.openssh.settings = {
    PasswordAuthentication = false; # or false for key-only
    PermitRootLogin = "no";
  };
  # In /etc/nixos/configuration.nix
  virtualisation.docker = {
    enable = true;
  };

  system.stateVersion = "25.05";
}
