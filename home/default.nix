{
  flake,
  config,
  pkgs,
  ...
}:
{
  imports = [ ]; # exposes programs.niri.settings
  # Home Manager needs a bit of information about you and the paths it should
  # manage.
  home.username = "frederik";
  home.homeDirectory = "/home/frederik";
  home.language.base = "en_US.UTF-8";

  # This value determines the Home Manager release that your configuration is
  # compatible with. This helps avoid breakage when a new Home Manager release
  # introduces backwards incompatible changes.
  #
  # You should not change this value, even if you update Home Manager. If you do
  # want to update the value, then make sure to first check the Home Manager
  # release notes.
  home.stateVersion = "25.05"; # Please read the comment before changing.

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

    # JSON‐like attrset → Waybar config
    settings.mainBar = {
      layer = "top";
      position = "top";

      # choose only the sections you care about:
      "modules-left" = [ "sway/workspaces" ];
      "modules-center" = [ "sway/mode" ];
      "modules-right" = [
        "cpu"
        "memory"
        "network"
        "battery"
        "clock"
      ];

      # per‑module configuration:
      cpu = {
        format = "{usage}%";
      };
      memory = {
        format = "{usedMiB} MiB";
      };
      network = {
        interface = "wlan0"; # adjust to your Wi‑Fi / ethernet device
        format = "{essid} {ipaddr}";
      };
      battery = {
        format = "{capacity}%";
        # icon‑only when low:
        low-at = 15;
      };
      clock = {
        format = "%Y‑%m‑%d %H:%M";
      };
    };

    # (optional) very simple CSS to tighten spacing:
    style = ''
      * {
        padding: 0 6px;
        margin: 0;
        background: #1e1e1e;
        color: #eee;
        font-family: Iosevka, monospace;
        font-size: 10pt;
      }
    '';
  };

  # The home.packages option allows you to install Nix packages into your
  # environment.
  home.packages = with pkgs; [
    nixos-generators
    trashy
    qrscan
    qemu
    deploy-rs
    nufmt
    nixfmt-rfc-style
    alejandra
    dprint
    nh
    nix-output-monitor
    rage
    nvd
    vulnix
    yazi
    mullvad
    cosmic-files
    # betterbird  # issues on archlinux
    nixd
    nil
    nix-tree
    cachix
    devenv
    gitui
    websocat
    overskride # not used
    bluetui # not yet available
    git-graph
    zotero
    zathura
    rio
    colmena
    tailscale
    nix-eval-jobs
    nix-fast-build
    rerun # doesnt work on archlinux by default
    typstyle
    statix
    rclone
  ];

  # Home Manager is pretty good at managing dotfiles. The primary way to manage
  # plain files is through 'home.file'.
  home.file = {
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
    # centerpiece.enable = true;

    # Let Home Manager install and manage itself.
    home-manager.enable = true;
  };

  # This doesnt support flakes yet. Doing a custom service
  # services.home-manager.autoUpgrade = {
  #   enable = true;
  #   frequency = "weekly";
  # };
  # # Fix for https://github.com/nix-community/home-manager/issues/3127
  # systemd.user.services.home-manager-auto-upgrade.Service.Environment = [
  #   "PATH=${lib.makeSearchPath "bin" [ pkgs.nix ]}"
  # ];
}
