{
  flake,
  config,
  pkgs,
  lib,
  ...
}:
{
  imports = [
    flake.inputs.zen-browser.homeModules.beta
    flake.inputs.centerpiece.hmModules."x86_64-linux".default
    flake.inputs.agenix.homeManagerModules.default
  ];

  home.username = "frederik";
  home.homeDirectory = "/home/frederik";
  home.language.base = "en_US.UTF-8";

  home.stateVersion = "25.05";

  age.secrets = {
    spadesvision_gdrive_token.file = ../secrets/spadesvision_gdrive_token.age;
    spadesvision_gdrive_client_secret.file = ../secrets/spadesvision_gdrive_client_secret.age;
  };
  age.identityPaths = [ "/home/frederik/.ssh/id_github" ];

  home.packages = with pkgs; [
    # Nix stuff
    cachix
    colmena
    deploy-rs
    devenv
    nix-eval-jobs
    nix-fast-build
    nix-output-monitor
    nixd
    nix-tree
    nixos-generators
    nvd
    vulnix
    nil
    nix-index
    nix-search-tv

    # Development & Formatting Tools
    alejandra
    dprint
    nufmt
    nixfmt-rfc-style
    statix
    typstyle
    cargo
    rustc
    rust-analyzer
    clippy
    rerun
    bacon

    # Containerization & Virtualization
    qemu

    # Version Control & Collaboration
    git-interactive-rebase-tool
    git-graph
    gitui

    # Networking & Communication
    bluetui
    mullvad
    websocat
    transmission_4-gtk

    # File & Data Management
    cosmic-files
    qrscan
    trashy
    yazi
    fd
    ripgrep
    sendme
    ffmpeg
    pcloud

    # Productivity & Document Tools
    zathura
    zotero
    xournalpp

    # Miscellaneous Utilities
    overskride
    rage
    rerun
    pass-wayland
    ragenix
    wl-clipboard
    zip
    unzip
    xdg-utils
    ncdu
    google-cloud-sdk
    xwayland-satellite
    tldr
  ];

  # Home Manager is pretty good at managing dotfiles. The primary way to manage
  # plain files is through 'home.file'.
  home.file = {
    ".rclone_filter_spadesvision.txt".text = ''
      - Marketing/Material für Swisscasino Video/**
      - Development/TCSJOHNHUXLEY Shared/Jetson reComputer j4011 Installation/**
      - Development/TCSJOHNHUXLEY Shared/Mock Spades Vision Server/**
    '';
    # # Building this configuration will create a copy of 'dotfiles/screenrc' in
    # # the Nix store. Activating the configuration will then make '~/.screenrc' a
    # # symlink to the Nix store copy.
    # ".screenrc".source = dotfiles/screenrc;

    # # You can also set the file content immediately.
    # ".gradle/gradle.properties".text = ''
    #   org.gradle.console=verbose
    #   org.gradle.daemon.idletimeout=3600000
    # '';
  };

  # Define an XDG desktop entry for Rerun
  xdg.desktopEntries.rerun = {
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

  home.sessionVariables = {
    DISPLAY = ":0"; # For xwayland
    EDITOR = "hx";
    TERM = "foot";
  };
  systemd.user.sessionVariables = {
    DISPLAY = ":0"; # For xwayland
    EDITOR = "hx";
    TERM = "foot";
  };

  fonts.fontconfig = {
    enable = true;
    defaultFonts.monospace = [ "JetBrains Mono" ];
  };

  services.swww.enable = true;

  programs.firefox = {
    enable = true;
    languagePacks = [
      "en-US"
      "de-CH"
      "es"
    ];
  };

  programs.ssh = {
    enable = true;
    addKeysToAgent = "yes";
    matchBlocks = {
      "github.com" = {
        user = "git";
        identityFile = "~/.ssh/id_github";
      };

      "git.sr.ht" = {
        user = "git";
        identityFile = "~/.ssh/id_sourcehut";
      };

      "gitlab.inf.ethz.ch" = {
        user = "git";
        identityFile = "~/.ssh/id_gitlab_inf";
      };

      "gitlab.ethz.ch" = {
        user = "git";
        identityFile = "~/.ssh/id_gitlab_ethz";
      };

      tcsJetson = {
        host = "*tcs-jetson*";
        identityFile = "~/.ssh/spadesvision/tcs-jetson/id_tcs-jetson_spadesvision";
      };

      tcsJetsonRoot = {
        match = ''
          host *tcs-jetson*
          user root
        '';
        identityFile = "~/.ssh/spadesvision/tcs-jetson/id_tcs-jetson_root";
      };
      bern = {
        host = "*bern*";
        identityFile = "~/.ssh/spadesvision/spadesvision/id_spadesvision_spadesvision";
      };

      bernRoot = {
        match = ''
          host *bern*
          user root
        '';
        identityFile = "~/.ssh/spadesvision/spadesvision/id_spadesvision_root";
      };
    };
  };

  programs.zen-browser = {
    enable = true;
    policies = {
      DisableAppUpdate = true;
      DisableTelemetry = true;
      # find more options here: https://mozilla.github.io/policy-templates/
    };
  };

  programs.centerpiece = {
    enable = true;
    config = {
      color = {
        text = "#ffffff";
        background = "#000000";
      };
      plugin = {
        applications = {
          enable = true;
        };
        brave_bookmarks = {
          enable = false;
        };
        brave_history = {
          enable = false;
        };
        brave_progressive_web_apps = {
          enable = false;
        };
        clock = {
          enable = true;
        };
        firefox_bookmarks = {
          enable = false;
        };
        firefox_history = {
          enable = true;
        };
        git_repositories = {
          enable = true;
          commands = [
            [
              "alacritty"
              "--command"
              "nvim"
              "$GIT_DIRECTORY"
            ]
            [
              "alacritty"
              "--working-directory"
              "$GIT_DIRECTORY"
            ]
          ];
        };
        gitmoji = {
          enable = false;
        };
        resource_monitor_battery = {
          enable = true;
        };
        resource_monitor_cpu = {
          enable = true;
        };
        resource_monitor_disks = {
          enable = true;
        };
        resource_monitor_memory = {
          enable = true;
        };
        sway_windows = {
          enable = false;
        };
        system = {
          enable = true;
        };
        wifi = {
          enable = false;
        };
      };
    };

    # enables a systemd service to index git-repositories
    services.index-git-repositories = {
      enable = true;
      interval = "5min";
    };
  };

  programs.git = {
    enable = true;

    # identity
    userName = "Frederik Semmel";
    userEmail = "frederiksemmel@gmail.com";

    # Delta syntax-highlighting
    delta = {
      enable = true;
      options = {
        navigate = true; # use n/N to move between diff hunks
        "side-by-side" = true; # two-column view
        dark = true; # dark theme
        # light = false;              # disable auto-detect if you prefer a light theme
      };
    };

    # other git settings
    extraConfig = {
      merge.conflictstyle = "diff3";
      diff.colorMoved = "default";
      sequence.editor = "interactive-rebase-tool";
      init.defaultBranch = "main";
    };
  };

  programs.jq.enable = true;

  programs.rclone = {
    enable = true;
    remotes = {
      spadesvision = {
        # non-secret config options
        config = {
          type = "drive";
          client_id = "113233231770-v4b6rfq5ajc3imv0vs0g3fnbl2q8rd84.apps.googleusercontent.com";
          scope = "drive";
          team_drive = "0AH2EQdMAdZriUk9PVA";
          root_folder_id = ""; # leave blank if you don’t use it
        };
        secrets = {
          client_secret = config.age.secrets.spadesvision_gdrive_client_secret.path;
          token = config.age.secrets.spadesvision_gdrive_token.path;
        };
      };
    };
  };

  programs.bottom.enable = true;

  programs.nh = {
    enable = true;
    clean.enable = true;
    clean.extraArgs = "--keep-since 14d --keep 10";
    flake = "/home/frederik/repos/nixos-config";
  };

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
      xkb.layout = "us, us(intl)";
      xkb.options = "ctrl:nocaps, compose:ralt, grp:win_space_toggle, grp_led:caps, altwin:swap_alt_win";
    };

    ### Outputs
    outputs."eDP-1" = {
      position = {
        x = 0;
        y = 1440;
      };
    };
    outputs."DP-3" = {
      position = {
        x = 0;
        y = 0;
      };
    };

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
      DISPLAY = ":0";
    };

    ### Visuals
    layout.gaps = 0; # disables the default 16px gap :contentReference[oaicite:0]{index=0}
    # Turn off only the window-close animation
    animations = {
      window-close = null;
      window-open = null;
    };
    layout.focus-ring.enable = false;
    layout.border = {
      enable = true;
      width = 1;
      inactive.color = "#000000";
      active.color = "#505050";
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
        "${mod}+D".action = spawn "centerpiece";
        # "${mod}+Shift+D".action = spawn "centerpiece";
        # "${mod}+Ctrl+Shift+L".action = spawn "swaylock";

        # Volume controls
        "XF86AudioRaiseVolume".allow-when-locked = true;
        "XF86AudioRaiseVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1+";
        "XF86AudioLowerVolume".allow-when-locked = true;
        "XF86AudioLowerVolume".action = spawn "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "0.1-";
        "XF86AudioMute".allow-when-locked = true;
        "XF86AudioMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SINK@" "toggle";

        "XF86AudioMicMute".allow-when-locked = true;
        "XF86AudioMicMute".action = spawn "wpctl" "set-mute" "@DEFAULT_AUDIO_SOURCE@" "toggle";
        "XF86MonBrightnessUp".action = spawn "light" "-A" "10";
        "XF86MonBrightnessDown".action = spawn "light" "-U" "10";
        "Shift+XF86MonBrightnessUp".action = spawn "light" "-A" "1";
        "Shift+XF86MonBrightnessDown".action = spawn "light" "-U" "1";

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
        "Print".action = screenshot;
        # "Ctrl+Print".action = screenshot-screen;  # Doesnt work
        "Alt+Print".action = screenshot-window;

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
        "pulseaudio"
        "battery"
        "cpu"
        "memory"
      ];

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

  programs = {
    direnv = {
      enable = true;
      enableNushellIntegration = true;
      nix-direnv.enable = true;
    };

    nushell = {
      enable = true;
      environmentVariables = config.home.sessionVariables // {
        ENV_CONVERSIONS.PATH = {
          from_string = lib.hm.nushell.mkNushellInline "{|s| $s | split row (char esep) }";
          to_string = lib.hm.nushell.mkNushellInline "{|v| $v | str join (char esep) }";
        };
      };
      settings = {
        show_banner = false;
        rm.always_trash = true;
        buffer_editor = "hx";
      };
    };
    starship = {
      enable = true;
      enableNushellIntegration = true;
      settings = {
        add_newline = false;
        direnv.disabled = false;
        battery.disabled = true;
        package.disabled = true;
        nodejs.disabled = true;
        gcloud.format = "on [$symbol($project)]($style) ";
        gcloud.project_aliases.decent-decker-379914 = "bj";
      };
    };
    zoxide = {
      enable = true;
      enableNushellIntegration = true;
    };
    mpv = {
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
    foot = {
      enable = true;
      settings = {
        main = {
          font = "monospace:size=10";
        };
        mouse = {
          hide-when-typing = "yes";
        };
        scrollback = {
          lines = 100000;
        };
        colors = {
          background = "1e1e1e";
          regular2 = "50a14f"; # green: a more "greener" but muted tone
          bright2 = "98c379"; # bright green: complements the normal green with a softer, bright appearance
        };
      };
    };
    helix = {
      enable = true;
      defaultEditor = true;
      settings = {
        theme = "monokai_pro_ristretto";

        editor = {
          end-of-line-diagnostics = "disable";
          inline-diagnostics = {
            cursor-line = "disable";
          };
          "line-number" = "relative";
          shell = [
            "nu"
            "--stdin"
            "-c"
          ];
          "idle-timeout" = 200;
          "auto-save" = true;

          "cursor-shape" = {
            insert = "bar";
          };

          lsp = {
            "display-inlay-hints" = false;
          };
        };

        keys = {
          normal = {
            "C-s" = ":w"; # Maps the Control-s to the typable command :w which is an alias for :write (save file)
            pageup = "move_char_left";
            pagedown = "move_char_right";
            "C-g" = [
              ":new"
              ":insert-output gitui"
              ":buffer-close!"
              ":redraw"
            ];
          };

          insert = {
            j = {
              k = "normal_mode";
            }; # Maps `jk` to exit insert mode
            "C-s" = ":w"; # Maps the Control-s to the typable command :w which is an alias for :write (save file)
            pageup = "move_char_left";
            pagedown = "move_char_right";
          };
        };
      };

      languages = {
        "language-server" = {
          pyright = {
            command = "pyright-langserver";
            args = [ "--stdio" ];
          };

          ruff = {
            command = "ruff";
            args = [ "server" ];
            config = {
              settings = {
                run = "onSave";
              };
            };
          };

          "rust-analyzer" = {
            command = "rust-analyzer";
            config = {
              procMacro = {
                ignored = {
                  leptos_macro = [ "server" ];
                };
              };
              checkOnSave = true;
              check.command = "clippy";
            };
          };

          nulsp = {
            command = "nu";
            args = [ "--lsp" ];
          };

          "tailwindcss-ls" = {
            config = {
              userLanguages = {
                rust = "html";
                "*.rs" = "html";
              };
            };
          };
          "svelteserver" = {
            command = "svelteserver";
            args = [ "--stdio" ];
          };

          tinymist = {
            command = "tinymist";
          };

          "typst-languagetool-lsp" = {
            command = "typst-languagetool-lsp";
            config = {
              bundled = false;
            };
          };

          harper = {
            command = "harper-ls";
            args = [ "--stdio" ];
            config = {
              "harper-ls" = {
                userDictPath = "~/dict.txt";
                fileDictPath = "~/.harper/";
              };
            };
          };

          "typst-lsp" = {
            command = "typst-lsp";
          };

          nil = {
            command = "nil";
            config = {
              formatting = {
                command = [ "nixfmt" ];
              };
            };
          };

          nixd = {
            command = "nixd";
          };

          taplo = {
            command = lib.getExe pkgs.taplo;
            args = [
              "lsp"
              "stdio"
            ];
          };
        };

        language = [
          {
            name = "rust";
            formatter = {
              command = "rustfmt";
            };
            language-servers = [ "rust-analyzer" ];
            auto-format = false;
          }
          {
            name = "svelte";
            scope = "source.svelte";
            injection-regex = "svelte";
            comment-token = "//";
            file-types = [
              "svelte"
            ];
            indent = {
              tab-width = 2;
              unit = "  ";
            };
            language-servers = [
              "svelteserver"
              "tailwindcss-ls"
            ];
          }
          {
            name = "markdown";
            formatter = {
              command = "dprint";
              args = [
                "fmt"
                "--stdin"
                "md"
              ];
            };
            language-servers = [ "harper" ];
          }
          {
            name = "nu";
            language-servers = [ "nulsp" ];
          }
          {
            name = "python";
            roots = [ "pyproject.toml" ];
            language-servers = [
              "pyright"
              "ruff"
            ];
            auto-format = true;
          }
          {
            name = "typst";
            indent = {
              tab-width = 2;
              unit = "  ";
            };
            language-servers = [
              "tinymist"
              "harper"
            ];
            formatter = {
              command = "typstyle";
            };
          }
          {
            name = "toml";
            scope = "source.toml";
            language-servers = [ "taplo" ];
          }
          {
            name = "nix";
            language-servers = [
              "nil"
              "nixd"
            ];
            auto-format = true;
          }
        ];
      };
    };

    # Let Home Manager install and manage itself.
    home-manager.enable = true;
  };
}
