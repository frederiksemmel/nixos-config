{
  config,
  pkgs,
  lib,
  ...
}:
{
  home.packages = with pkgs; [
    # Nix stuff
    cachix
    colmena
    # deploy-rs  # is in devenv
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
    fzf
    nix-diff
    nix-bundle
    elfutils

    # Development & Formatting Tools
    alejandra
    dprint
    nufmt
    nixfmt
    nixfmt-tree
    statix
    typstyle
    cargo
    rustc
    rust-analyzer
    clippy
    bacon
    ty

    ## Find and replace tools
    amber
    ruplacer

    sad
    bat

    # Containerization & Virtualization
    qemu

    # Version Control & Collaboration
    git-interactive-rebase-tool
    git-graph
    gitui

    # Networking & Communication
    bluetui
    websocat
    transmission_4-gtk
    # postman
    iwmenu
    pwmenu
    bzmenu

    # File & Data Management
    cosmic-files
    trashy
    fd
    ripgrep
    sendme
    ffmpeg
    pcloud
    handlr-regex
    pipe-rename
    imagemagick
    ghostscript
    # inlyne  # TODO enable when working again
    gdrive3

    # Audio & Video
    pavucontrol
    spotify
    shotcut
    wf-recorder
    pwvucontrol

    # Productivity & Document Tools
    zotero
    xournalpp
    libreoffice
    # ungoogled-chromium
    evince
    #papers issues with saving and icons
    signal-desktop
    sqlitebrowser

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
    tealdeer
    wlr-which-key
    usbutils
    pciutils
    difftastic
    pwgen
    tokei
    nmap
    poppler-utils
    exfat

    # productivity
    rustdesk-flutter
    anydesk
    # rustdesk-flutter
  ];

  home.sessionVariables = {
    EDITOR = "hx";
    TERM = "foot";
    NIXOS_OZONE_WL = "1";
  };
  systemd.user.sessionVariables = {
    EDITOR = "hx";
    TERM = "foot";
    NIXOS_OZONE_WL = "1";
    # NOTE should not be set in the newer niri versions
    # WAYLAND_DISPLAY = "wayland-1";
    # DISPLAY = ":0";
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
      environmentVariables = config.home.sessionVariables // {
        ENV_CONVERSIONS.PATH = {
          from_string = lib.hm.nushell.mkNushellInline "{|s| $s | split row (char esep) }";
          to_string = lib.hm.nushell.mkNushellInline "{|v| $v | str join (char esep) }";
        };
      };
      shellAliases = {
        ns = "exec (nix-search-tv print | fzf --preview 'nix-search-tv preview {}' --scheme history)";
      };
      settings = {
        completions.external = {
          enable = true;
          max_results = 20;
        };
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
        python.disabled = true;
        gcloud.format = "on [$symbol($project)]($style) ";
        gcloud.project_aliases.decent-decker-379914 = "bj";
        nix_shell.format = "via [$symbol$name](bold blue) ";
      };
    };

    zoxide = {
      enable = true;
      enableNushellIntegration = true;
    };

    carapace = {
      enable = true;
      enableNushellIntegration = true;
    };

    ghostty = {
      enable = true;
    };

    eza = {
      enable = true;
      # enableNushellIntegration = true;
    };

    foot = {
      enable = true;
      server.enable = true;
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
        colors-dark = {
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
            "C-t" = [
              ":new"
              ":insert-output gitui"
              ":buffer-close!"
              ":redraw"
            ];
            # space.t submenu for themes
            space = {
              t = {
                "1" = ":theme monokai_pro_ristretto";
                "2" = ":theme zed_onelight";
              };
            };
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
        language-server = {
          pyright = {
            command = "pyright-langserver";
            args = [ "--stdio" ];
          };
          ty = {
            command = "ty";
            args = [ "server" ];
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

          rust-analyzer = {
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

          tailwindcss-language-server = {
            command = "tailwindcss-language-server";
            config = {
              userLanguages = {
                rust = "html";
                "*.rs" = "html";
              };
            };
          };
          svelteserver = {
            command = "svelteserver";
            args = [ "--stdio" ];
          };

          tinymist = {
            command = "tinymist";
          };

          harper = {
            command = lib.getExe pkgs.harper;
            args = [ "--stdio" ];
            config = {
              harper-ls = {
                userDictPath = "~/dict.txt";
                fileDictPath = "~/.harper/";
                linters.ToDoHyphen = false;
              };
            };
          };

          typst-lsp = {
            command = "typst-lsp";
          };

          nil = {
            command = "nil";
            # config = {
            #   formatting = {
            #     command = [ "nixfmt" ];
            #   };
            # };
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
              "tailwindcss-language-server"
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
              # "pyright"
              "ty"
              "ruff"
              "harper"
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

    zed-editor = {
      enable = true;
    };

    git = {
      enable = true;
      settings = {
        # identity
        user.name = "Frederik Semmel";
        user.email = "frederiksemmel@gmail.com";

        merge.conflictstyle = "diff3";
        diff.colorMoved = "default";
        sequence.editor = "interactive-rebase-tool";
        init.defaultBranch = "main";
      };
    };

    # Delta syntax-highlighting
    delta = {
      enable = true;
      enableGitIntegration = true;
      options = {
        navigate = true; # use n/N to move between diff hunks
        "side-by-side" = true; # two-column view
        dark = true; # dark theme
        # light = false;              # disable auto-detect if you prefer a light theme
      };
    };

    jq.enable = true;

    rclone = {
      enable = true;
      remotes = {
        spadesvision = {
          # non-secret config options
          config = {
            type = "drive";
            client_id = "113233231770-v4b6rfq5ajc3imv0vs0g3fnbl2q8rd84.apps.googleusercontent.com";
            scope = "drive";
            team_drive = "0AH2EQdMAdZriUk9PVA";
            root_folder_id = ""; # leave blank if you don't use it
          };
          secrets = {
            client_secret = config.age.secrets.spadesvision_gdrive_client_secret.path;
            token = config.age.secrets.spadesvision_gdrive_token.path;
          };
        };
      };
    };

    yazi = {
      enable = true;
      shellWrapperName = "yy";
      enableNushellIntegration = true;
      settings = {
        sort_by = "natural";
      };
      theme = {
        indicator.padding = {
          open = "█";
          close = "█";
        };
      };
    };

    bottom.enable = true;

    nh = {
      enable = true;
      clean.enable = false;
      clean.extraArgs = "--keep-since 14d --keep 10";
      flake = "${config.home.homeDirectory}/repos/nixos-config";
    };

    claude-code = {
      enable = true;
    };

    # Let Home Manager install and manage itself.
    home-manager.enable = true;
  };
}
