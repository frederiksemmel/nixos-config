{
  flake,
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.desktop;
  handlr-xdg-open = pkgs.writeShellScriptBin "xdg-open" ''
    ${lib.getExe pkgs.handlr-regex} open "$@"
  '';
  wlr-common-config = ''
    # Theming
    font: monospace
    background: "#000000ff"
    color: "#ffffff"
    # border: "#8ec07c"
    separator: " ➜ "
    border_width: 0
    corner_r: 0
    padding: 15 # Defaults to corner_r
    # rows_per_column: 5 # No limit by default
    # column_padding: 25 # Defaults to padding

    # Anchor and margin
    anchor: center # One of center, left, right, top, bottom, bottom-left, top-left, etc.
    # Only relevant when anchor is not center
    margin_right: 0
    margin_bottom: 0
    margin_left: 0
    margin_top: 0

    # Permits key bindings that conflict with compositor key bindings.
    # Default is `false`.
    # inhibit_compositor_keyboard_shortcuts: true
  '';
  wlsunsetToggle = pkgs.writeShellApplication {
    name = "wlsunset-toggle";
    runtimeInputs = with pkgs; [
      libnotify
      systemd
    ];
    text = ''
      if systemctl --user is-active --quiet wlsunset; then
        systemctl --user stop wlsunset
        notify-send -a wlsunset "Night light off"
      else
        systemctl --user start wlsunset
        notify-send -a wlsunset "Night light on"
      fi
    '';
  };
  usbcShareToggle = pkgs.writeShellApplication {
    name = "usbc-share-toggle";
    runtimeInputs = with pkgs; [
      libnotify
      systemd
    ];
    text = ''
      if systemctl is-active --quiet usbc-share.service; then
        verb=stop
      else
        verb=start
      fi
      if out=$(/run/wrappers/bin/sudo -n /run/current-system/sw/bin/systemctl "$verb" usbc-share.service 2>&1); then
        if [ "$verb" = start ]; then
          notify-send -a usbc-share "USB-C sharing on"
        else
          notify-send -a usbc-share "USB-C sharing off"
        fi
      else
        notify-send -a usbc-share -u critical "USB-C sharing" "$verb failed: $out"
      fi
    '';
  };
  usbEjectAll = pkgs.writeShellApplication {
    name = "usb-eject-all";
    runtimeInputs = with pkgs; [
      coreutils
      gawk
      libnotify
      udisks
      util-linux
    ];
    text = ''
      eject() {
        local name fstype mountpoint
        while read -r name fstype mountpoint; do
          if [ -n "$mountpoint" ]; then
            udisksctl unmount --no-user-interaction -b "$name" || return 1
          fi
          if [ "$fstype" = crypto_LUKS ] && [ "$(lsblk -nro NAME "$name" | wc -l)" -gt 1 ]; then
            udisksctl lock --no-user-interaction -b "$name" || return 1
          fi
        done < <(lsblk -rnpo NAME,FSTYPE,MOUNTPOINT "$1" | tac)
        udisksctl power-off --no-user-interaction -b "$1"
      }

      mapfile -t disks < <(lsblk -dnbpo NAME,TRAN,SIZE | awk '$2 == "usb" && $3 > 0 { print $1 }')
      if [ ''${#disks[@]} -eq 0 ]; then
        notify-send -a usb-eject "No USB drives"
        exit 0
      fi
      for disk in "''${disks[@]}"; do
        label=$(lsblk -dno MODEL "$disk" | xargs)
        label="''${label:-$disk}"
        if out=$(eject "$disk" 2>&1); then
          notify-send -a usb-eject "Ejected $label"
        else
          notify-send -a usb-eject -u critical "Eject $label failed" "$out"
        fi
      done
    '';
  };
  # TLP 1.8+ carries three named profiles (performance/balanced/power-saver,
  # configured through the _ON_AC/_ON_BAT/_ON_SAV suffixes of the host's
  # services.tlp.settings). The named verbs deliberately *clear* TLP's manual
  # mode -- only `tlp ac`/`tlp bat` latch it -- so a pick survives plugging and
  # unplugging by way of smart switching instead: TLP_AUTO_SWITCH=2 keeps the
  # current profile whenever it is not the power source's default. `tlp start`
  # and boot both hand the machine back to the assigned profile. The host grants
  # passwordless sudo for exactly these four invocations.
  tlpProfile = pkgs.writeShellApplication {
    name = "tlp-profile";
    runtimeInputs = with pkgs; [
      libnotify
      procps
    ];
    text = ''
      profile="$1"
      if out=$(/run/wrappers/bin/sudo -n /run/current-system/sw/bin/tlp "$profile" 2>&1); then
        if [ "$profile" = start ]; then
          notify-send -a tlp "Power profile" "automatic"
        else
          notify-send -a tlp "Power profile" "$profile"
        fi
        pkill -RTMIN+14 waybar || true
      else
        notify-send -a tlp -u critical "Power profile" "tlp $profile failed: $out"
      fi
    '';
  };
  # `tlp-stat -m` needs no root: it reads /run/tlp/last_pwr, which tlp leaves
  # world-readable. It prints "<profile>/<CODE>", optionally followed by
  # " (manual)" or " (default)"; the bar shows the short code.
  tlpStatus = pkgs.writeShellApplication {
    name = "tlp-status";
    text = ''
      mode=$(/run/current-system/sw/bin/tlp-stat -m) || mode="unknown/???"
      code=''${mode#*/}
      code=''${code%% *}
      if [ "$(cat /sys/devices/system/cpu/intel_pstate/no_turbo)" = 1 ]; then
        turbo=off
      else
        turbo=on
      fi
      maxpct=$(cat /sys/devices/system/cpu/intel_pstate/max_perf_pct)
      printf '{"text":"PWR %s","tooltip":"%s\\nturbo %s, max %s%%","class":"%s"}\n' \
        "$code" "$mode" "$turbo" "$maxpct" "''${code,,}"
    '';
  };
  rerun-icon = pkgs.callPackage ../../packages/rerun-icon.nix { };
  toggleWaybar = pkgs.writeShellScriptBin "toggle-waybar" ''
    ${lib.getExe pkgs.killall} -SIGUSR1 -r waybar
  '';
  # Route audio over HDMI when the tv kanshi profile is applied. The card's
  # analog profile has a higher priority than the HDMI one, so wireplumber
  # never picks HDMI on its own.
  tvAudio = pkgs.writeShellApplication {
    name = "tv-audio";
    runtimeInputs = with pkgs; [
      jq
      pipewire
      wireplumber
    ];
    text = ''
      card="${cfg.hdmiAudio.card}"
      profile="output:hdmi-stereo+input:analog-stereo"
      sink="alsa_output.${cfg.hdmiAudio.device}.hdmi-stereo"

      card_id=$(pw-dump | jq -r --arg n "$card" \
        'first(.[] | select(.info.props."device.name" == $n) | .id)')
      profile_index=$(pw-dump "$card_id" | jq -r --arg p "$profile" \
        'first(.[0].info.params.EnumProfile[] | select(.name == $p) | .index)')
      wpctl set-profile "$card_id" "$profile_index"

      # The sink node only shows up once the profile switch has settled.
      sink_id=""
      for _ in $(seq 20); do
        sink_id=$(pw-dump | jq -r --arg n "$sink" \
          'first(.[] | select(.info.props."node.name" == $n) | .id) // empty')
        if [ -n "$sink_id" ]; then
          break
        fi
        sleep 0.25
      done
      wpctl set-default "$sink_id"
    '';
  };
in
{
  imports = with flake.inputs; [
    veila.homeModules.default
    # centerpiece.hmModules."x86_64-linux".default
  ];

  options.profiles.desktop = {
    enable = lib.mkEnableOption "graphical desktop (niri, waybar, GUI apps)";
    wallpaper = lib.mkOption {
      type = lib.types.str;
      default = "${config.home.homeDirectory}/wallpapers/longboard.jpg";
      description = "Wallpaper image used by the lock screen and other desktop services.";
    };
    hdmiAudio = {
      device = lib.mkOption {
        type = lib.types.str;
        default = "pci-0000_00_1f.3";
        description = "ALSA device id of the codec carrying HDMI output.";
      };
      card = lib.mkOption {
        type = lib.types.str;
        readOnly = true;
        default = "alsa_card.${cfg.hdmiAudio.device}";
        description = "Pipewire device.name of that codec, read by the host to target its profile-set.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = with pkgs; [
      (lib.hiPrio handlr-xdg-open)

      # Networking & Communication
      transmission_4-gtk
      iwmenu
      pwmenu
      bzmenu
      mqttx

      # File & Data Management
      # nautilus (systemPackages in fstux) is the one GUI file manager: it owns
      # org.freedesktop.FileManager1, which chromium's "show in files" calls.
      pcloud
      handlr-regex

      # Audio & Video
      # pavucontrol
      spotify
      shotcut
      wf-recorder
      pwvucontrol

      # Productivity & Document Tools
      zotero
      xournalpp
      # libreoffice
      # ungoogled-chromium
      evince
      # papers issues with saving and icons
      signal-desktop
      sqlitebrowser
      inlyne

      # Desktop / Wayland utilities
      overskride
      rerun
      pass-wayland
      wl-clipboard
      xdg-utils
      xwayland-satellite
      wlr-which-key
      brightnessctl
      claudebar

      # Remote desktop
      rustdesk-flutter
      anydesk
    ];

    # One look for both file-picker code paths: xdg-desktop-portal-gtk and apps
    # that call gtk_file_chooser_dialog_new themselves (onlyoffice). Leaving
    # gtk3.bookmarks unset keeps ~/.config/gtk-3.0/bookmarks writable and shared.
    gtk = {
      enable = true;
      # Not Cantarell or Adwaita Sans: fontconfig 2.18 tags both with the
      # system-ui generic, chromium's own lookup then rejects them and its UI
      # falls back to an unrelated font (NixOS/nixpkgs#541553). fc-match is
      # unaffected. Inter is what Adwaita Sans derives from and survives it.
      font = {
        name = "Inter";
        size = 11;
      };
      theme = {
        name = "Adwaita";
        package = pkgs.gnome-themes-extra;
      };
      iconTheme = {
        name = "Adwaita";
        package = pkgs.adwaita-icon-theme;
      };
      colorScheme = "dark";
      # libadwaita (nautilus) only honours colorScheme; post-26.05 default.
      gtk4.theme = null;
    };

    home.file = {
      ".rclone_filter_spadesvision.txt".text = ''
        - Marketing/Material für Swisscasino Video/**
        - Development/Jetson Server Dev/*Releases*/**
      '';
      ".config/handlr/handlr.toml".text = ''
        enable_selector = true
        selector = "${lib.getExe pkgs.fuzzel} --dmenu"
        term_exec_args = '-e'
      '';
      ".config/inlyne/inlyne.toml".text = ''
        theme = "Dark"
        page-width = 1150
        lines-to-scroll = 3.0

        [font-options]
        regular-font = "Inter"
        monospace-font = "JetBrains Mono"

        [dark-theme]
        code-highlighter = "monokai-extended"
      '';
      ".config/wlr-which-key/config_power.yaml".text = ''
        ${wlr-common-config}
        menu:
          "w":
            desc: Wifi Menu
            cmd: iwmenu -l fuzzel
          "a":
            desc: Audio Menu
            cmd: pwmenu -l fuzzel -m output-devices
          "b":
            desc: Bluetooth Menu
            cmd: bzmenu -l fuzzel
          "n":
            desc: Toggle Night Light
            cmd: ${lib.getExe wlsunsetToggle}
          "t":
            desc: Toggle USB-C Internet Sharing
            cmd: ${lib.getExe usbcShareToggle}
          "e":
            desc: Eject USB Drives
            cmd: ${lib.getExe usbEjectAll}
          "c":
            desc: CPU Profile
            submenu:
              "p":
                desc: Performance
                cmd: ${lib.getExe tlpProfile} performance
              "b":
                desc: Balanced
                cmd: ${lib.getExe tlpProfile} balanced
              "s":
                desc: Power Saver (no turbo, half clock)
                cmd: ${lib.getExe tlpProfile} power-saver
              "a":
                desc: Automatic (follow power source)
                cmd: ${lib.getExe tlpProfile} start
          "s":
            desc: Sleep
            cmd: systemctl suspend
          "r":
            desc: Reboot
            cmd: reboot
          "o":
            desc: Off
            cmd: poweroff
          "l":
            desc: Lock
            cmd: ${config.programs.veila.package}/bin/veila lock
          "i":
            desc: Inhibit Sleep (5h)
            cmd: systemd-inhibit --what=handle-lid-switch sleep 5h
          "u":
            desc: Uninhibit Sleep
            cmd: pkill -x systemd-inhibit
      '';
      ".config/wlr-which-key/config_opener.yaml".text = ''
        ${wlr-common-config}
        menu:
          "w":
            desc: Browser
            cmd: handlr launch text/html
          "f":
            desc: Files
            cmd: handlr launch x-scheme-handler/file
      '';
    };

    # Icon= is a theme name, and nixpkgs' rerun installs no icon at all, so the
    # lookup failed and launchers drew their fallback glyph. recursive, because
    # ~/.local/share/icons/hicolor also holds unmanaged icons (pcloud's
    # AppImage) that a whole-directory symlink would displace.
    xdg.dataFile."icons/hicolor" = {
      source = "${rerun-icon}/share/icons/hicolor";
      recursive = true;
    };

    # Define an XDG desktop entry for Rerun
    xdg.desktopEntries = {
      rerun = {
        name = "Rerun";
        genericName = "Multimodal Data Viewer";
        comment = "Visualize streams of multimodal data";
        exec = "rerun %F";
        terminal = false;
        icon = "rerun";
        categories = [
          "Development"
          "Science"
          "Utility"
        ];
      };
    };

    # NOTE this was an attempt to make ly work. But somehow graphical-session.target starts way to early
    # systemd.user.services.waybar.Unit.Requisite = [ "graphical-session.target" ];
    # systemd.user.services.swayidle.Unit.Requisite = [ "graphical-session.target" ];

    services.walker = {
      enable = true;
    };
    services.kanshi = {
      enable = true;
      settings = [
        {
          output.criteria = "eDP-1";
          output.mode = "2880x1800@90Hz";
          output.scale = 1.75;
        }
        {
          profile.name = "laptop";
          profile.outputs = [
            {
              criteria = "eDP-1";
            }
          ];
        }
        {
          profile.name = "mobile";
          profile.outputs = [
            {
              criteria = "eDP-1";
              position = "1494,0";
            }
            {
              criteria = "Lenovo Group Limited M14d *";
              position = "0,0";
            }
          ];
        }
        {
          # TV above the laptop, horizontally centered on it:
          # (3840 - 2880/1.75) / 2 = 1097
          profile.name = "tv";
          profile.outputs = [
            {
              criteria = "Samsung Electric Company SAMSUNG *";
              mode = "3840x2160@24Hz";
              scale = 1.0;
              position = "0,0";
            }
            {
              criteria = "eDP-1";
              position = "1097,2160";
            }
          ];
          profile.exec = [ (lib.getExe tvAudio) ];
        }
        {
          profile.name = "office";
          profile.outputs = [
            {
              criteria = "eDP-1";
              position = "456,1441";
            }
            {
              criteria = "DP-3";
              position = "0,0";
            }
          ];
        }
      ];
    };
    # Locking is handled by veila's own idle monitor, which takes a logind delay
    # inhibitor so the lock surface is up before the system actually suspends.
    #
    # Blanking, however, cannot be left to veila: it drives DPMS through
    # zwlr_output_power_management_v1, which niri does not implement (see the
    # note on lock.screen_off_seconds below). So we blank compositor-side
    # instead, reproducing what veila's screen_off_seconds=60 was meant to do:
    # the display goes dark 60s after veila locks.
    # No resumeCommand is needed -- niri calls activate_monitors() itself on the
    # next input event -- but it is harmless, idempotent, and makes the intent
    # explicit rather than relying on that behaviour staying true.
    services.swayidle = {
      enable = true;
      timeouts = [
        {
          timeout = config.programs.veila.idle.lockAfter + 60;
          command = "${config.programs.niri.package}/bin/niri msg action power-off-monitors";
          resumeCommand = "${config.programs.niri.package}/bin/niri msg action power-on-monitors";
        }
      ];
    };
    services.mako = {
      enable = true;
      extraConfig = ''
        default-timeout=10000

        [app-name=Spotify]
        default-timeout=3000

        [app-name=claude-code]
        border-color=#d97757
        border-size=2
      '';
    };
    services.awww.enable = true;
    services.wlsunset = {
      enable = true;
      latitude = 47.37;
      longitude = 8.5417;
      temperature.night = 3000;
    };

    programs.zed-editor.enable = true;

    # Lock screen. NOTE: PAM cannot be set up from home-manager, so
    # `security.pam.services.veila` lives in the host config.
    programs.veila = {
      enable = true;
      service.enable = true;
      idle = {
        enable = true;
        lockAfter = 600;
        lockBeforeSleep = true;
      };
      settings = {
        theme = "normandy";

        # Everything below overrides the normandy theme.
        background = {
          mode = "file";
          path = cfg.wallpaper;
          scaling = "fill";
        };

        lock = {
          hide_cursor = true;
          # MUST stay 0 on niri. Veila blanks via zwlr_output_power_management_v1,
          # which niri does not implement, so this never worked -- but worse, the
          # failure pegs a CPU core: once the delay expires, ScreenOffState::due_in
          # returns Duration::ZERO forever (the power-off request silently no-ops,
          # so outputs_powered_off is never set), and that zero becomes the
          # curtain's calloop timeout -- a non-blocking dispatch in a tight loop.
          # Measured: 105% CPU and 95 C package temp for an entire lock.
          # To actually blank, drive niri's `power-off-monitors` from an idle
          # timeout (swayidle) rather than asking veila to do it.
          screen_off_seconds = 0;
        };

        # Widgets. Positions/colors come from normandy.
        battery.enabled = true;
        weather = {
          enabled = true;
          location = "Zurich";
          latitude = 47.37;
          longitude = 8.5417;
          unit = "celsius";
        };
        visuals = {
          # Which of us/us(intl)/us(colemak) is active — typing the password in
          # the wrong layout is otherwise silent.
          keyboard.enabled = true;
          # Caps is remapped to Ctrl and its LED shows the xkb group, so the
          # caps indicator would only ever be noise.
          caps_lock.enabled = false;
          avatar.enabled = false;
          username.enabled = false;
          now_playing.enabled = true;
          eye.enabled = false;
        };
      };
    };

    # veilad spawns veila-curtain as a plain child, so the curtain inherits the
    # daemon's environment rather than the session's. The veila module orders
    # veilad after graphical-session.target only, which is not a barrier for
    # WAYLAND_DISPLAY: niri publishes it with `systemctl --user import-environment`
    # once its socket is up, and the manager hands each service the environment
    # block as it was at spawn time. Start veilad before niri -- which happens
    # whenever graphical-session.target is reached outside niri's own transaction,
    # e.g. after niri is OOM-killed and the session restarts -- and it holds an
    # environment with no WAYLAND_DISPLAY for its whole life, so every curtain
    # dies with "Could not find wayland compositor" and locking silently fails.
    # BindsTo pulls niri.service into veilad's start transaction so After can
    # actually order against it.
    systemd.user.services.veilad.Unit = {
      After = [ "niri.service" ];
      BindsTo = [ "niri.service" ];
    };

    programs.sherlock = {
      enable = false;
      settings = {
        aliases = { };
        config = {
        };
        launchers = [
          {
            name = "App Launcher";
            type = "app_launcher";
            args = { };
            priority = 1;
            home = true;
          }
        ];
        style = null;
      };
    };

    programs.zathura = {
      enable = true;
      extraConfig = ''
        set selection-clipboard clipboard
      '';
    };

    programs.onlyoffice = {
      enable = true;
    };

    programs.fuzzel = {
      enable = true;
      settings = {
        # COLORS (all in RRGGBBAA)
        colors.background = "000000ff"; # black background :contentReference[oaicite:0]{index=0}
        colors.text = "ffffffff"; # white text for unselected entries :contentReference[oaicite:1]{index=1}
        # colors.border = "ffffffff";

        # keep the selection colors consistent (black bg + white fg)
        colors.selection = "000000ff";
        colors.selection-text = "ffffffff";

        # matching substrings in both normal and selected entries
        colors.match = "ffffffff";
        colors.selection-match = "ffffffff";

        # BORDER
        border.radius = 0; # disable rounded corners (default is 10) :contentReference[oaicite:2]{index=2}
        border.width = 0; # 1px border
      };
    };

    programs.chromium = {
      enable = true;
      commandLineArgs = [ "--enable-features=TouchpadOverscrollHistoryNavigation" ];
      package = (pkgs.chromium.override { enableWideVine = true; });
      # NOTE pass extension needs native python app, not easy on nixos
      # extensions = [
      #   {
      #     id = "oblajhnjmknenodebpekmkliopipoolo";
      #   }
      # ];
    };
    programs.firefox = {
      enable = true;
      configPath = "${config.xdg.configHome}/mozilla/firefox";
    };

    # programs.centerpiece = {
    #   enable = true;
    #   config = {
    #     color = {
    #       text = "#ffffff";
    #       background = "#000000";
    #     };
    #     plugin = {
    #       applications = {
    #         enable = true;
    #       };
    #       brave_bookmarks = {
    #         enable = false;
    #       };
    #       brave_history = {
    #         enable = false;
    #       };
    #       brave_progressive_web_apps = {
    #         enable = false;
    #       };
    #       clock = {
    #         enable = true;
    #       };
    #       firefox_bookmarks = {
    #         enable = false;
    #       };
    #       firefox_history = {
    #         enable = true;
    #       };
    #       git_repositories = {
    #         enable = false;
    #       };
    #       gitmoji = {
    #         enable = true;
    #       };
    #       resource_monitor_battery = {
    #         enable = true;
    #       };
    #       resource_monitor_cpu = {
    #         enable = true;
    #       };
    #       resource_monitor_disks = {
    #         enable = true;
    #       };
    #       resource_monitor_memory = {
    #         enable = true;
    #       };
    #       niri_windows = {
    #         enable = true;
    #       };
    #       system = {
    #         enable = true;
    #       };
    #       wifi = {
    #         enable = true;
    #       };
    #     };
    #   };

    # enables a systemd service to index git-repositories
    #   services.index-git-repositories = {
    #     enable = true;
    #     interval = "5min";
    #   };
    # };

    programs.niri.package = lib.mkDefault pkgs.niri;

    programs.niri.settings = {
      "prefer-no-csd" = true;
      ### Inputs
      input.focus-follows-mouse.enable = true;
      input.touchpad = {
        accel-profile = "adaptive";
        accel-speed = 0.3;
        scroll-factor = 0.5;
        dwt = true;
      };
      input.keyboard = {
        repeat-delay = 250;
        repeat-rate = 30;
        xkb.layout = "us, us(intl), us(colemak)";
        xkb.options = "ctrl:nocaps, compose:ralt, grp:win_space_toggle, grp_led:caps, altwin:swap_alt_win";
      };

      ## Outputs
      outputs."eDP-1" = {
        # scale = 2.0;
        position = {
          x = 0;
          y = 0;
        };
      };
      outputs."Dell Inc. DELL U2721DE B0GC623" = {
        position = {
          x = 0;
          y = -1440 - 1;
        };
      };
      outputs."Samsung Electric Company SAMSUNG 0x01000E00" = {

      };
      # outputs."HDMI-A-1" = {
      #   mode = {
      #     width = 3840;
      #     height = 2160;
      #     refresh = 23.976;
      #   };
      # };

      ### Startup
      spawn-at-startup = [
        {
          command = [
            (lib.getExe pkgs.xwayland-satellite)
          ];
        }
      ];
      environment = {
        QT_QPA_PLATFORM = "wayland";
      };

      ### Visuals
      layout.gaps = 0; # disables the default 16px gap :contentReference[oaicite:0]{index=0}
      # layout.border = {
      #   enable = true;
      #   width = 2;
      #   inactive.color = "#000000";
      #   active.color = "#505050";
      # };
      layout.focus-ring = {
        enable = true;
        width = 2;
        inactive.color = "#000000";
        active.color = "#505050";
      };
      # Turn off only the window-close animation
      animations = {
        window-close.enable = false;
        window-open.enable = false;
      };

      ### Keybindings
      binds =
        with config.lib.niri.actions;
        let
          mod = "Super";
        in
        {
          # Hotkey overlay
          "${mod}+Shift+Slash".action = show-hotkey-overlay;

          # Launchers
          "${mod}+Return".action = spawn "foot";
          "${mod}+D".action = spawn "vicinae" "toggle";
          "${mod}+Shift+D".action = spawn "fuzzel";
          "${mod}+P".action =
            spawn "wlr-which-key" "${config.home.homeDirectory}/.config/wlr-which-key/config_power.yaml";
          "${mod}+O".action =
            spawn "wlr-which-key" "${config.home.homeDirectory}/.config/wlr-which-key/config_opener.yaml";

          # Volume controls
          "XF86AudioRaiseVolume".allow-when-locked = true;
          "XF86AudioRaiseVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1+";
          "XF86AudioLowerVolume".allow-when-locked = true;
          "XF86AudioLowerVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1-";
          "XF86AudioMute".allow-when-locked = true;
          "XF86AudioMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SINK@" "toggle";

          "XF86AudioMicMute".allow-when-locked = true;
          "XF86AudioMicMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SOURCE@" "toggle";
          "XF86MonBrightnessUp".action = spawn "brightnessctl" "s" "10%+";
          "XF86MonBrightnessDown".action = spawn "brightnessctl" "s" "10%-";
          "Shift+XF86MonBrightnessUp".action = spawn "brightnessctl" "s" "1%+";
          "Shift+XF86MonBrightnessDown".action = spawn "brightnessctl" "s" "1%-";

          # Window management
          "${mod}+Shift+Q".action = close-window;

          # Focus
          "${mod}+H".action = focus-column-left;
          "${mod}+J".action = focus-window-down;
          "${mod}+K".action = focus-window-up;
          "${mod}+L".action = focus-column-right;
          "${mod}+Left".action = focus-column-left;
          "${mod}+Down".action = focus-window-down;
          "${mod}+Up".action = focus-window-up;
          "${mod}+Right".action = focus-column-right;

          # Move
          "${mod}+Ctrl+H".action = move-column-left;
          "${mod}+Ctrl+J".action = move-window-down-or-to-workspace-down;
          "${mod}+Ctrl+K".action = move-window-up-or-to-workspace-up;
          "${mod}+Ctrl+L".action = move-column-right;
          "${mod}+Ctrl+Left".action = move-column-left;
          "${mod}+Ctrl+Down".action = move-window-down-or-to-workspace-down;
          "${mod}+Ctrl+Up".action = move-window-up-or-to-workspace-up;
          "${mod}+Ctrl+Right".action = move-column-right;

          # Cycle columns/workspaces
          "${mod}+Home".action = focus-column-first;
          "${mod}+End".action = focus-column-last;
          "${mod}+Ctrl+Home".action = move-column-to-first;
          "${mod}+Ctrl+End".action = move-column-to-last;

          # Monitors
          "${mod}+Shift+H".action = focus-monitor-left;
          "${mod}+Shift+J".action = focus-monitor-down;
          "${mod}+Shift+K".action = focus-monitor-up;
          "${mod}+Shift+L".action = focus-monitor-right;
          "${mod}+Shift+Ctrl+H".action = move-column-to-monitor-left;
          "${mod}+Shift+Ctrl+J".action = move-column-to-monitor-down;
          "${mod}+Shift+Ctrl+K".action = move-column-to-monitor-up;
          "${mod}+Shift+Ctrl+L".action = move-column-to-monitor-right;

          # Workspaces
          "${mod}+U".action = focus-workspace-down;
          "${mod}+I".action = focus-workspace-up;
          # "${mod}+Ctrl+U".action = move-window-down-or-workspace-down;
          # "${mod}+Ctrl+I".action = move-window-up-or-workspace-up;
          "${mod}+Shift+U".action = move-workspace-down;
          "${mod}+Shift+I".action = move-workspace-up;

          # Scroll through workspaces
          "${mod}+WheelScrollDown" = {
            cooldown-ms = 150;
            action = focus-workspace-down;
          };
          "${mod}+WheelScrollUp" = {
            cooldown-ms = 150;
            action = focus-workspace-up;
          };
          "${mod}+Ctrl+WheelScrollDown" = {
            cooldown-ms = 150;
            action = move-column-to-workspace-down;
          };
          "${mod}+Ctrl+WheelScrollUp" = {
            cooldown-ms = 150;
            action = move-column-to-workspace-up;
          };

          # Scroll columns
          "${mod}+WheelScrollRight".action = focus-column-right;
          "${mod}+WheelScrollLeft".action = focus-column-left;
          "${mod}+Ctrl+WheelScrollRight".action = move-column-right;
          "${mod}+Ctrl+WheelScrollLeft".action = move-column-left;
          "${mod}+Shift+WheelScrollDown".action = focus-column-right;
          "${mod}+Shift+WheelScrollUp".action = focus-column-left;
          "${mod}+Ctrl+Shift+WheelScrollDown".action = move-column-right;
          "${mod}+Ctrl+Shift+WheelScrollUp".action = move-column-left;

          # Numeric workspaces
          "${mod}+1".action = focus-workspace 1;
          "${mod}+2".action = focus-workspace 2;
          "${mod}+3".action = focus-workspace 3;
          "${mod}+4".action = focus-workspace 4;
          "${mod}+5".action = focus-workspace 5;
          "${mod}+6".action = focus-workspace 6;
          "${mod}+7".action = focus-workspace 7;
          "${mod}+8".action = focus-workspace 8;
          "${mod}+9".action = focus-workspace 9;
          "${mod}+Ctrl+1".action = move-column-to-index 1;
          "${mod}+Ctrl+2".action = move-column-to-index 2;
          "${mod}+Ctrl+3".action = move-column-to-index 3;
          "${mod}+Ctrl+4".action = move-column-to-index 4;
          "${mod}+Ctrl+5".action = move-column-to-index 5;
          "${mod}+Ctrl+6".action = move-column-to-index 6;
          "${mod}+Ctrl+7".action = move-column-to-index 7;
          "${mod}+Ctrl+8".action = move-column-to-index 8;
          "${mod}+Ctrl+9".action = move-column-to-index 9;

          # Window grouping
          "${mod}+BracketLeft".action = consume-or-expel-window-left;
          "${mod}+BracketRight".action = consume-or-expel-window-right;
          "${mod}+Comma".action = consume-window-into-column;
          "${mod}+Period".action = expel-window-from-column;

          # Layout actions
          "${mod}+R".action = switch-preset-column-width;
          "${mod}+Shift+R".action = switch-preset-window-height;
          "${mod}+Ctrl+R".action = reset-window-height;
          "${mod}+F".action = maximize-column;
          "${mod}+Shift+F".action = fullscreen-window;
          "${mod}+Ctrl+F".action = expand-column-to-available-width;
          "${mod}+C".action = center-column;
          "${mod}+B".action = spawn (lib.getExe toggleWaybar);

          # Adjust dimensions
          "${mod}+Minus".action = set-column-width "-10%";
          "${mod}+Equal".action = set-column-width "+10%";
          "${mod}+Shift+Minus".action = set-window-height "-10%";
          "${mod}+Shift+Equal".action = set-window-height "+10%";

          # Floating/Tiling
          "${mod}+V".action = toggle-window-floating;
          "${mod}+Shift+V".action = switch-focus-between-floating-and-tiling;

          # Tabs
          "${mod}+W".action = toggle-column-tabbed-display;

          # Screenshots
          "Print".action.screenshot = [ ];
          # "Ctrl+Print".action = screenshot-screen;  # Doesnt work
          "Alt+Print".action.screenshot-window = [ ];

          # Inhibitor toggle
          "${mod}+Escape" = {
            allow-inhibiting = false;
            action = toggle-keyboard-shortcuts-inhibit;
          };

          # Quit & power
          "${mod}+Shift+E".action = quit;
          "Ctrl+Alt+Delete".action = quit;
          "${mod}+Shift+P".action = power-off-monitors;
        };

      ### Window‑specific rules
      "window-rules" = [
        {
          matches = [ { "app-id" = "foot"; } ];
          "default-column-width".proportion = 0.5;
        }
        {
          matches = [ { "app-id" = "chromium-browser"; } ];
          "default-column-width".proportion = 1.0;
        }
        {
          matches = [ { "app-id" = ".qemu-system-x86_64-wrapped"; } ];
          "default-column-width".fixed = 1280;
        }
        {
          matches = [ { "app-id" = "mpv"; } ];
          "default-column-width".proportion = 0.5;
        }
        {
          matches = [ { "app-id" = "rerun"; } ];
          "default-column-width".proportion = 1.0;
        }
        # {
        #   matches = [ { "title" = "centerpiece"; } ];
        #   "open-floating" = true;
        # }
      ];
    };

    # Enable Waybar service
    programs.waybar = {
      enable = true;
      systemd.enable = true;
      package = pkgs.waybar;

      # Broad stroke customization
      settings.mainBar = {
        # Positioning and size
        position = "bottom";
        layer = "overlay";
        height = 26;

        spacing = 20;

        # Choose only the modules you care about
        modules-left = [
          "niri/workspaces"
        ];
        "modules-center" = [ "clock" ];
        "modules-right" = [
          "custom/claudebar"
          "network"
          "pulseaudio"
          "custom/tlp"
          "battery"
          "cpu"
          "memory"
          "disk"
        ];

        # Claude plan usage: session % and time until the 5h window resets.
        # The usage endpoint is rate limited, so 300s is the lowest sane interval;
        # `pkill -RTMIN+13 waybar` forces a refresh.
        "custom/claudebar" = {
          exec = "${lib.getExe pkgs.claudebar} --format 'CLD {session_pct}% {session_reset}'";
          "return-type" = "json";
          interval = 300;
          signal = 13;
          tooltip = true;
          "on-click" = "xdg-open https://claude.ai/settings/usage";
        };

        # Active TLP profile. Nothing polls for it, so the interval is only a
        # backstop against a change made outside the power menu; tlp-profile
        # signals the bar directly.
        "custom/tlp" = {
          exec = lib.getExe tlpStatus;
          "return-type" = "json";
          interval = 60;
          signal = 14;
          tooltip = true;
          "on-click" =
            "wlr-which-key --initial-keys c ${config.home.homeDirectory}/.config/wlr-which-key/config_power.yaml";
        };

        pulseaudio = {
          format-muted = "VOL MUTED";
          format = "VOL {volume}% {icon}";
          format-icons = {
            default = "";
            headphone = "HP";
            headset = "HS";
          };
        };

        # Example: override only the clock format
        clock = {
          format = "{:%Y-%m-%d %H:%M}";
          tooltip = true;
          interval = 60;
        };

        # Example: simple battery display
        battery = {
          format = "BAT {capacity}%";
          "critical-threshold" = 15;
          interval = 30;
        };

        # Network: show SSID and strength on wlan0
        network = {
          interface = "wlan0";
          "format-wifi" = "{essid} ({signalStrength}%)";
        };
        # CPU load
        cpu = {
          format = "CPU {usage}%";
          interval = 10;
        };

        # Memory usage: {used} is already in GiB
        memory = {
          format = "MEM {used:0.1f}G";
          interval = 30;
        };

        # Free space on the root filesystem
        disk = {
          path = "/";
          format = "DSK {percentage_free}%";
          interval = 60;
        };
      };

      # Inject minimal custom CSS; other styles use Waybar defaults
      style = ''
        * {
          border: none;
          border-radius: 4px;
          /* `ttf-font-awesome` is required to be installed for icons */
          font-family: "monospace", JetBrains Mono;

          /* adjust font-size value to your liking: */
          font-size: 12px;

          min-height: 0;
        }
        window#waybar {
          background: rgba(30, 30, 30, 1.0);
          color: #ffffff;
        }

        #workspaces button {
          color: #ffffff;
          box-shadow: inset 0 -3px transparent;
          min-width: 0;
        }

        #workspaces button:hover {
          background: rgba(0, 0, 0, 0.9);
          box-shadow: inset 0 -3px #ffffff;
        }

        #workspaces button.focused {
          background-color: #64727d;
        }

        #workspaces button.urgent {
          background-color: #eb4d4b;
        }

        #workspaces {
          margin-left: 0;
          min-width: 0;
        }

        /* Being throttled is easy to forget about; make it visible. */
        #custom-tlp.sav {
          color: #f0c674;
        }
      '';
    };

    programs.imv = {
      enable = true;
      settings = {
        options = {
          overlay = "1";
          overlay_font = "Monospace:10";
        };
      };
    };

    programs.mpv = {
      enable = true;
      config = {
        hwdec = "auto-safe";
        vo = "gpu";
        profile = "gpu-hq";
        gpu-context = "wayland";
        sub-scale = 0.7;
        keep-open = true;
        # autoload the other files of the same type in the directory as a playlist
        autocreate-playlist = "same";
        osd-status-msg = "\${playback-time/full} / \${duration} (\${percent-pos}%)\\nframe: \${estimated-frame-number} / \${estimated-frame-count}";
        sub-file-paths = "Subs";
      };
      bindings = {
        # increase subtitle font size
        "ALT+k" = "add sub-scale +0.1";
        # decrease subtitle font size
        "ALT+j" = "add sub-scale -0.1";
        # reduce video pan step size
        "ALT+LEFT" = "add video-pan-x +0.01";
        "ALT+RIGHT" = "add video-pan-x -0.01";
        "ALT+UP" = "add video-pan-y +0.01";
        "ALT+DOWN" = "add video-pan-y -0.01";
        # Disable scrolling, because it sucks
        MOUSE_BTN3 = "ignore";
        MOUSE_BTN4 = "ignore";
      };
    };
    programs.vicinae = {
      enable = true;
      systemd.enable = true;
    };
    # Quiet the file indexer. Release builds still `add_compile_definitions(DEBUG=0)`,
    # but the server gates on `#if defined(DEBUG)` — true either way — so it forces
    # VICINAE_FILE_INDEXER_LOG=debug onto the indexer unless the var is already set.
    systemd.user.services.vicinae.Service.Environment = [
      "VICINAE_FILE_INDEXER_LOG=warn"
    ];
  };
}
