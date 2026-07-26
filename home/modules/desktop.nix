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
    #!${pkgs.bash}/bin/bash
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
  swaylockWithWallpaper = pkgs.writeShellScriptBin "swaylock-with-awww" ''
    #!${pkgs.bash}/bin/bash
    exec ${lib.getExe pkgs.swaylock-effects} --image ${cfg.wallpaper}
  '';
in
{
  # imports = with flake.inputs; [
  #   centerpiece.hmModules."x86_64-linux".default
  # ];

  options.profiles.desktop = {
    enable = lib.mkEnableOption "graphical desktop (niri, waybar, GUI apps)";
    wallpaper = lib.mkOption {
      type = lib.types.str;
      default = "${config.home.homeDirectory}/wallpapers/longboard.jpg";
      description = "Wallpaper image used by swaylock and other desktop services.";
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

      # File & Data Management
      cosmic-files
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
      # inlyne  # TODO enable when working again

      # Desktop / Wayland utilities
      overskride
      rerun
      pass-wayland
      wl-clipboard
      xdg-utils
      xwayland-satellite
      wlr-which-key
      brightnessctl

      # Remote desktop
      rustdesk-flutter
      anydesk
    ];

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
            desc: Off
            cmd: ${lib.getExe swaylockWithWallpaper}
          "i":
            desc: Inhibit Sleep
            cmd: systemd-inhibit --what=handle-lid-switch sleep infinity
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
    services.swayidle = {
      enable = true;
      package = pkgs.swayidle;
      extraArgs = [ "-w" ];
      events = {
        "before-sleep" = lib.getExe swaylockWithWallpaper;
      };
    };
    services.mako = {
      enable = true;
      extraConfig = ''
        [app-name=Spotify]
        default-timeout=3000
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

    programs.swaylock = {
      enable = true;
      package = pkgs.swaylock-effects;
      settings = {
        indicator = true;
        indicator-radius = 60;
        indicator-thickness = 12;

        ring-color = "ffffff55";
        ring-clear-color = "ffffff55";
        ring-ver-color = "ffffff55";
        ring-wrong-color = "ffffff55";

        key-hl-color = "ffffffff";

        line-color = "00000000";
        line-clear-color = "00000000";
        line-ver-color = "00000000";
        line-wrong-color = "00000000";

        inside-color = "00000000";
        inside-clear-color = "00000000";
        inside-ver-color = "00000000";
        inside-wrong-color = "00000000";

        separator-color = "00000000";

        text-color = "ffffffff";
        text-clear-color = "ffffffff";
        text-ver-color = "ffffffff";
        text-wrong-color = "ffffffff";

        font = "monospace";
        font-size = 30;

        effect-blur = "20x3";
        grace = 0;
        hide-keyboard-layout = true;
      };
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
          "${mod}+B".action =
            let
              toggleWaybar = pkgs.writeShellScriptBin "toggle-waybar" ''
                #!/usr/bin/env bash
                ${lib.getExe pkgs.killall} -SIGUSR1 -r waybar
              '';
            in
            spawn (lib.getExe toggleWaybar);

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
          "network"
          "backlight"
          "pulseaudio"
          "battery"
          "cpu"
          "memory"
        ];

        backlight = {
          format = "BRI {percent}%";
        };

        pulseaudio = {
          format-muted = "VOL MUTED";
          format = "VOL {volume}% {icon}";
          format-icons = {
            default = "";
            headphone = "headphone";
            headset = "headset";
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

        # Memory usage
        memory = {
          format = "MEM {used}%";
          interval = 30;
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
  };
}
