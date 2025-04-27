{
  flake,
  config,
  pkgs,
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
    spadesvision_gdrive_client_secret.file = ../secrets/spadesvision_gdrive_token.age;
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
    nh
    nil

    # Development & Formatting Tools
    alejandra
    dprint
    nufmt
    nixfmt-rfc-style
    statix
    typstyle
    cargo

    # Containerization & Virtualization
    qemu

    # Version Control & Collaboration
    git-interactive-rebase-tool
    git-graph
    gitui

    # Networking & Communication
    bluetui
    mullvad
    tailscale
    websocat

    # File & Data Management
    cosmic-files
    qrscan
    trashy
    yazi

    # Productivity & Document Tools
    zathura
    zotero

    # Miscellaneous Utilities
    overskride
    rage
    rerun
    pass-wayland
    ragenix
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

  home.sessionVariables = {
    EDITOR = "hx";
    TERM = "foot";
  };

  fonts.fontconfig = {
    enable = true;
    defaultFonts.monospace = [ "JetBrains Mono" ];
  };

  programs.firefox = {
    enable = true;
    languagePacks = [
      "en-US"
      "de-CH"
      "es"
    ];
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
          enable = true;
        };
        brave_history = {
          enable = true;
        };
        brave_progressive_web_apps = {
          enable = true;
        };
        clock = {
          enable = true;
        };
        firefox_bookmarks = {
          enable = true;
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
          enable = true;
        };
        system = {
          enable = true;
        };
        wifi = {
          enable = true;
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
          client_secret = config.age.secrets.spadesvision_gdrive_client_secret.path;
          token = config.age.secrets.spadesvision_gdrive_token.path;
        };
      };
    };
  };

  programs.bottom.enable = true;

  programs.niri.settings = {
    "prefer-no-csd" = true;
    ### Inputs
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
        mod = "Alt";
      in
      {
        # Hotkey overlay
        "${mod}+Shift+Slash".action = show-hotkey-overlay;

        # Launchers
        "${mod}+Return".action = spawn "foot";
        "${mod}+D".action = spawn "${pkgs.fuzzel}/bin/fuzzel";
        "${mod}+Shift+D".action = spawn "${pkgs.centerpiece}/bin/centerpiece";
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

        # Window management
        "${mod}+Q".action = close-window;

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
        "${mod}+Ctrl+1".action = move-column-to-workspace 1;
        "${mod}+Ctrl+2".action = move-column-to-workspace 2;
        "${mod}+Ctrl+3".action = move-column-to-workspace 3;
        "${mod}+Ctrl+4".action = move-column-to-workspace 4;
        "${mod}+Ctrl+5".action = move-column-to-workspace 5;
        "${mod}+Ctrl+6".action = move-column-to-workspace 6;
        "${mod}+Ctrl+7".action = move-column-to-workspace 7;
        "${mod}+Ctrl+8".action = move-column-to-workspace 8;
        "${mod}+Ctrl+9".action = move-column-to-workspace 9;

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

  programs.waybar = {
    enable = true;
    systemd.enable = true;

    settings.mainBar = {
      layer = "top";
      position = "top";
      height = 30;
      output = [
        "eDP-1"
        "HDMI-A-1"
      ];
      modules-left = [
        "wlr/taskbar"
      ];
      modules-center = [
        "custom/hello-from-waybar"
      ];
      modules-right = [
        "mpd"
        "custom/mymodule#with-css-id"
        "temperature"
      ];

      "sway/workspaces" = {
        disable-scroll = true;
        all-outputs = true;
      };
      "custom/hello-from-waybar" = {
        format = "hello {}";
        max-length = 40;
        interval = "once";
        exec = pkgs.writeShellScript "hello-from-waybar" ''
          echo "from within waybar"
        '';
      };
    };
    # (optional) very simple CSS to tighten spacing:
    style = ''
      * {
        border: none;
        border-radius: 0;
        font-family: Source Code Pro;
      }
      window#waybar {
        background: #16191C;
        color: #AAB2BF;
      }
      #workspaces button {
        padding: 0 5px;
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
          font = "monospace:size=12";
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
              checkOnSave = {
                command = "clippy";
              };
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
            command = "taplo";
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
