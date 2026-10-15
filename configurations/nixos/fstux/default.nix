{
  flake,
  config,
  pkgs,
  lib,
  ...
}:
let
  hm = config.home-manager.users.frederik;

  # The Intel HDMI codec exposes four HDMI PCMs, but the laptop only has one
  # HDMI port. ACP therefore builds a dozen "Digital Stereo/Surround (HDMI
  # 2/3/4)" card profiles that can never become available and just clutter
  # pavucontrol. Drop those mappings from the stock profile set.
  singleHdmiProfileSet = pkgs.runCommand "alsa-single-hdmi-profile-set" { } ''
    mkdir -p $out
    awk '
      /^\[/ { skip = /^\[Mapping hdmi-[a-z0-9-]+-extra[0-9]+\]/ }
      !skip
    ' ${pkgs.pipewire}/share/alsa-card-profile/mixer/profile-sets/default.conf \
      > $out/single-hdmi.conf
  '';

  greeter = pkgs.callPackage ../../../packages/greeter {
    niri = config.programs.niri.package;
    veila = hm.programs.veila.package;
    wallpaper =
      let
        w = builtins.head (import ../../../home/wallpapers.nix);
      in
      pkgs.fetchurl {
        inherit (w) url hash;
        name = "surf-${w.id}.jpg";
      };
    user = "frederik";
    session = {
      command = [
        "${pkgs.systemd}/bin/systemd-cat"
        "--identifier=niri-session"
        (lib.getExe' config.programs.niri.package "niri-session")
      ];
      desktopNames = [ "niri" ];
    };
    input = with hm.programs.niri.settings.input; ''
      keyboard {
          xkb {
              layout "${keyboard.xkb.layout}"
              options "${keyboard.xkb.options}"
          }
          repeat-delay ${toString keyboard.repeat-delay}
          repeat-rate ${toString keyboard.repeat-rate}
      }
      touchpad {
          ${lib.optionalString touchpad.tap "tap"}
          ${lib.optionalString touchpad.natural-scroll "natural-scroll"}
      }
    '';
  };
in
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

  nixpkgs.hostPlatform = "x86_64-linux";
  nixpkgs.config.allowUnfree = true;
  nixpkgs.overlays = [ flake.self.overlays.default ];

  home-manager.users.frederik = {
    imports = [ flake.self.homeModules.frederik ];
    home.stateVersion = "25.05";
    profiles.bitwarden.sshAgent.enable = true;
  };
  home-manager.backupFileExtension = "backup";
  home-manager.useGlobalPkgs = true;
  home-manager.useUserPackages = true;

  # make `nix run nixpkgs#nixpkgs` use the same nixpkgs as the one used by this flake.
  nix.registry.nixpkgs.flake = flake.inputs.nixpkgs;
  nix.channel.enable = false; # remove nix-channel related tools & configs, we use flakes instead.

  # but NIX_PATH is still used by many useful tools, so we set it to the same value as the one used by this flake.
  # Make `nix repl '<nixpkgs>'` use the same nixpkgs as the one used by this flake.
  environment.etc."nix/inputs/nixpkgs".source = "${flake.inputs.nixpkgs}";
  # https://github.com/NixOS/nix/issues/9574
  nix.settings = {
    trusted-users = [
      "frederik"
    ];
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    nix-path = lib.mkForce "nixpkgs=/etc/nix/inputs/nixpkgs";
    # build inputs are removed otherwise. It causes rebuilds of aarch64 opencv for example
    # if this is disabled. Other things I tried:
    # - Adding a gc root to the attributes inputDerivation, -> only direct build deps
    # - Listing all build outputs -qR --include-outputs ... works, but I couldn't create a
    # single root that keeps all these things
    keep-outputs = true;
    extra-substituters = [ "https://cache.nix-ci.com" ];
    extra-trusted-public-keys = [ "nix-ci:g3xV5BDTLtIBZr/A00IU1x0EtKKlb7YLgBN2SgYgM6A=" ];
    builders-use-substitutes = true;
  };

  age.secrets.nixci_netrc.file = ../../../secrets/nixci_netrc.age;
  age.identityPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];

  environment.etc."determinate/config.json".text = builtins.toJSON {
    authentication.additionalNetrcSources = [ config.age.secrets.nixci_netrc.path ];
  };

  # time.timeZone = "Europe/Zurich";
  security.sudo-rs.enable = true;
  # Veila is configured via home-manager, but PAM can only be set up system-wide.
  security.pam.services.veila = lib.mkIf hm.programs.veila.enable { };
  # Veila's battery widget reads the UPower display device over D-Bus; it has no
  # sysfs fallback, so without this it just logs "name is not activatable".
  services.upower.enable = lib.mkIf (hm.programs.veila.settings.battery.enabled or false) true;
  services.gnome.gcr-ssh-agent.enable = lib.mkIf hm.profiles.bitwarden.sshAgent.enable false;

  programs.ssh.extraConfig = ''
    Host eu.nixbuild.net
      PubkeyAcceptedKeyTypes ssh-ed25519
      ServerAliveInterval 60
      IPQoS throughput
      IdentityFile /home/frederik/.ssh/my-nixbuild-key
      Port 2223
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
        system = "x86_64-linux";
        maxJobs = 100;
        speedFactor = 4;
        supportedFeatures = [
          "benchmark"
          "big-parallel"
          "kvm"
          "nixos-test"
        ];
      }
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
  boot.loader.systemd-boot.configurationLimit = 5;
  # Menu stays hidden; hold Space during boot (or `systemctl reboot --boot-loader-menu=30`) to pick a generation.
  boot.loader.timeout = 0;
  boot.binfmt.emulatedSystems = [ "aarch64-linux" ];
  boot.plymouth = {
    enable = true;
    themePackages = [ pkgs.plymouth-spades-vision ];
    theme = "spades-vision";
    # The HiDPI auto scale would upscale the frames, which are rendered at native size.
    extraConfig = "DeviceScale=1";
  };
  # Leave the last splash frame on screen until the greeter's niri takes over
  # the display, instead of clearing back to the text console in between.
  systemd.services.plymouth-quit.serviceConfig.ExecStart = [
    ""
    "-${lib.getExe' config.boot.plymouth.package "plymouth"} quit --retain-splash"
  ];
  systemd.services.greetd.serviceConfig.ExecStartPre =
    "-${pkgs.writeShellScript "blank-fbdev" ''
      fb=/sys/class/graphics/fb0
      [ -e $fb ] || exit 0
      height=$(${pkgs.coreutils}/bin/cut -d, -f2 $fb/virtual_size)
      ${pkgs.coreutils}/bin/head -c $(( $(${pkgs.coreutils}/bin/cat $fb/stride) * height )) /dev/zero > /dev/fb0
    ''}";
  boot.consoleLogLevel = 3;
  boot.initrd.verbose = false;
  boot.kernelParams = [
    "quiet"
    "udev.log_level=3"
    "rd.udev.log_level=3"
    "systemd.show_status=auto"
    "rd.systemd.show_status=auto"
  ];
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
      # DNS = "9.9.9.9 1.1.1.1";
    };
  };

  # Started from the power menu (Super+P, t): while active, a laptop plugged in
  # over a Thunderbolt/USB4 USB-C cable or a USB Ethernet adapter gets a DHCP
  # lease and is NATed out through whatever uplink this machine has. IPv4LL stays
  # on so static-IP devices without DHCP stay reachable at their 169.254 mDNS
  # address. The .network file is dropped into /run so it sorts ahead of
  # 10-usb-lan only while sharing is on.
  systemd.services.usbc-share =
    let
      shareNetwork = pkgs.writeText "05-usbc-share.network" ''
        [Match]
        Name=thunderbolt* enp*u*

        [Network]
        Address=10.42.0.1/24
        LinkLocalAddressing=yes
        DHCPServer=yes
        IPMasquerade=ipv4
        IPv4Forwarding=yes

        [DHCPServer]
        PoolOffset=100
        PoolSize=50
        EmitDNS=yes
        DNS=9.9.9.9 1.1.1.1
      '';
      target = "/run/systemd/network/05-usbc-share.network";
    in
    {
      description = "Share internet over Thunderbolt or USB Ethernet";
      path = [
        pkgs.coreutils
        config.systemd.package
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
      };
      script = ''
        mkdir -p /run/systemd/network
        ln -sf ${shareNetwork} ${target}
        networkctl reload
      '';
      preStop = ''
        rm -f ${target}
        networkctl reload
      '';
    };
  networking.firewall.interfaces."thunderbolt+".allowedUDPPorts = [ 67 ];
  networking.firewall.interfaces."enp+".allowedUDPPorts = [ 67 ];

  networking.useDHCP = false;
  networking.wireless.iwd.enable = true;
  networking.wireless.iwd.settings.DriverQuirks.OweDisable = "iwlwifi";
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [
      80
      443
      # 5173
      # 8765
      9876
      # 53317
      1883 # for testing rabbitmq
    ];
    allowedUDPPorts = [
      5353
      # 5355
      # for tailscale. TODO does it work?
      # 41641
      # 3478
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
      "seat"
    ];
    hashedPassword = "$6$sBbqKGdIHSuqH2HL$voX0LJZaG/qu7y.0QUQYGSsadiHf99esXblI196WQNpvkeJf/P9L1BSoLJkCza/KQ/vCsibYteEHXXn0nMO7R0";
    shell = pkgs.nushell;
  };

  users.users.root.hashedPassword = "$6$TFVUB5p/gECWeZMb$GauIqbzHXTRitc6GHwlSGAzkfEpHqMoEnpu7dMK7.4gKNY4RIX3J2gBICORN4xeatkonTzKo0Q2wmyS4SQt7J.";

  programs.sniffnet.enable = true;
  programs.gnupg.agent = {
    enable = true;
    pinentryPackage = pkgs.pinentry-curses;
  };

  xdg.portal = {
    enable = true;
    extraPortals = [
      pkgs.xdg-desktop-portal-gnome
      pkgs.xdg-desktop-portal-gtk
    ];
    config.common = {
      default = [
        "gnome"
        "gtk"
      ];
      # Electron/chromium fail to open the file picker otherwise.
      "org.freedesktop.impl.portal.FileChooser" = "gtk";
      "org.freedesktop.impl.portal.Access" = "gtk";
      "org.freedesktop.impl.portal.Notification" = "gtk";
    };
  };
  # Without a dconf backend GSettings uses the memory backend and libadwaita
  # apps ignore the theme home manager writes to org/gnome/desktop/interface.
  programs.dconf.enable = lib.mkIf hm.gtk.enable true;

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
    corefonts
    noto-fonts
    noto-fonts-color-emoji
    fira-code
    fira-code-symbols
    nerd-fonts.fira-code
    inter # GTK UI font
  ];
  fonts.enableDefaultPackages = true;

  services.tlp = {
    enable = true;

    # Global TLP settings
    settings = {
      # Disable USB autosuspend on both AC and battery
      USB_AUTOSUSPEND = "0";

      # The three profiles the power menu's CPU submenu selects between. This
      # i7-12700H runs intel_pstate in active mode, where "powersave" is the
      # normal governor and the energy/performance preference does the steering;
      # "performance" would peg every core. Every knob is spelled out for all
      # three profiles because TLP only writes what it is told: leaving
      # CPU_BOOST_ON_AC unset would strand no_turbo=1 after a visit to SAV.
      CPU_SCALING_GOVERNOR_ON_AC = "powersave";
      CPU_SCALING_GOVERNOR_ON_BAT = "powersave";
      CPU_SCALING_GOVERNOR_ON_SAV = "powersave";

      CPU_ENERGY_PERF_POLICY_ON_AC = "balance_performance";
      CPU_ENERGY_PERF_POLICY_ON_BAT = "balance_power";
      CPU_ENERGY_PERF_POLICY_ON_SAV = "power";

      CPU_MIN_PERF_ON_AC = 0;
      CPU_MIN_PERF_ON_BAT = 0;
      CPU_MIN_PERF_ON_SAV = 0;
      CPU_MAX_PERF_ON_AC = 100;
      CPU_MAX_PERF_ON_BAT = 100;
      CPU_MAX_PERF_ON_SAV = 50;

      # Turbo is what spins the fans up; SAV caps the cores at their base clock.
      CPU_BOOST_ON_AC = 1;
      CPU_BOOST_ON_BAT = 1;
      CPU_BOOST_ON_SAV = 0;
      CPU_HWP_DYN_BOOST_ON_AC = 1;
      CPU_HWP_DYN_BOOST_ON_BAT = 1;
      CPU_HWP_DYN_BOOST_ON_SAV = 0;
    };
  };
  # The power menu (Super+P, c) picks a TLP profile as the user; tlp itself
  # needs root. Restricted to these four argument forms so the rule cannot be
  # turned into a general root shell via `tlp discharge` and friends.
  security.sudo-rs.extraRules = [
    {
      users = [ "frederik" ];
      commands =
        map
          (arg: {
            command = "/run/current-system/sw/bin/tlp ${arg}";
            options = [ "NOPASSWD" ];
          })
          [
            "performance"
            "balanced"
            "power-saver"
            "start"
          ];
    }
    {
      users = [ "frederik" ];
      commands =
        map
          (verb: {
            command = "/run/current-system/sw/bin/systemctl ${verb} usbc-share.service";
            options = [ "NOPASSWD" ];
          })
          [
            "start"
            "stop"
          ];
    }
  ];
  services.devmon.enable = true;
  services.tailscale = {
    enable = true;
  };
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
    pulse.enable = true;
    wireplumber.extraConfig."51-single-hdmi" = {
      "monitor.alsa.rules" = [
        {
          matches = [ { "device.name" = hm.profiles.desktop.hdmiAudio.card; } ];
          actions.update-props = {
            "device.profile-set" = "${singleHdmiProfileSet}/single-hdmi.conf";
          };
        }
      ];
    };
  };

  services.greetd = {
    enable = true;
    settings.default_session.command = lib.getExe greeter;
  };

  # programs.sway.enable = true;

  programs.niri.enable = true;
  programs.niri.package = pkgs.niri;
  niri-flake.cache.enable = false;
  # for pcloud
  programs.fuse.enable = true;
  programs.fuse.userAllowOther = true;

  services.usbmuxd.enable = true;

  zramSwap = {
    enable = true;
    algorithm = "zstd";
    memoryPercent = 50;
  };
  boot.kernel.sysctl = {
    "vm.swappiness" = 180;
    "vm.page-cluster" = 0;
    "vm.watermark_boost_factor" = 0;
    "vm.watermark_scale_factor" = 125;
  };

  systemd.oomd = {
    enableUserSlices = true;
    settings.OOM.DefaultMemoryPressureDurationSec = "20s";
  };
  systemd.slices."-".sliceConfig.ManagedOOMSwap = "kill";

  services.automatic-timezoned.enable = true;
  services.tzupdate.enable = true;
  # A switch restarts iwd, so tzupdate can start while wifi is still down.
  systemd.services.tzupdate = {
    serviceConfig = {
      Type = lib.mkForce "exec";
      Restart = "on-failure";
      RestartSec = "30s";
    };
    startLimitIntervalSec = 300;
    startLimitBurst = 5;
  };

  environment.systemPackages =
    with pkgs;
    [
      libimobiledevice
      ifuse # optional, to mount using 'ifuse'
      fuse3 # for pcloud

      nautilus
      zenity
    ]
    ++ lib.optional hm.profiles.bitwarden.enable bitwarden-desktop;

  # In /etc/nixos/configuration.nix
  virtualisation.docker = {
    enable = true;
  };

  virtualisation.virtualbox.host.enable = true;
  # virtualisation.virtualbox.host.enableExtensionPack = true;
  # virtualisation.virtualbox.guest.enable = true;
  users.extraGroups.vboxusers.members = [ "frederik" ];

  systemd.network.wait-online.ignoredInterfaces = [ "vboxnet0" ];

  system.stateVersion = "25.05";
}
