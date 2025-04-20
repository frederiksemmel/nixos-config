{
  pkgs,
  ...
}:
{
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
    # mullvad
    cosmic-files
    # betterbird  # issues on archlinux
    nixd
    nil
    nix-tree
    cachix
    devenv
    gitui
    websocat
    # overskride  # not used
    # bluetui  # not yet available
    git-graph
    zotero
    # zathura
    # rio
    colmena
    tailscale
    nix-eval-jobs
    nix-fast-build
    # rerun # doesnt work on archlinux by default
    #
    # nixgl.auto.nixGLDefault
    # nix-gl-host
    nixos-rebuild
    typstyle
    statix
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

  # Home Manager can also manage your environment variables through
  # 'home.sessionVariables'. If you don't want to manage your shell through Home
  # Manager then you have to manually source 'hm-session-vars.sh' located at
  # either
  #
  #  ~/.nix-profile/etc/profile.d/hm-session-vars.sh
  #
  # or
  #
  #  /etc/profiles/per-user/frederik/etc/profile.d/hm-session-vars.sh
  #
  home.sessionVariables = {
    EDITOR = "hx";
  };

  programs = {
    direnv = {
      enable = true;
      enableNushellIntegration = true;
      nix-direnv.enable = true;
    };

    nushell = {
      enable = true;
      extraConfig = ''
        $env.config = {
          show_banner: false
          rm: {
            always_trash: true
          }

          completions: {
            case_sensitive: false # set to true to enable case-sensitive completions
            quick: true  # set this to false to prevent auto-selecting completions when only one remains
            partial: true  # set this to false to prevent partial filling of the prompt
            algorithm: "prefix"  # prefix or fuzzy
            external: {
              enable: true # set to false to prevent nushell looking into $env.PATH to find more suggestions, `false` recommended for WSL users as this look up my be very slow
              max_results: 100 # setting it lower can improve completion performance at the cost of omitting some options
              completer: null # check 'carapace_completer' above as an example
            }
          }
          buffer_editor: "hx" # command that will be used to edit the current line buffer with ctrl+o, if unset fallback to $env.EDITOR and $env.VISUAL
          edit_mode: emacs # emacs, vi
          cursor_shape: {
            emacs: line # block, underscore, line (line is the default)
            vi_insert: block # block, underscore, line (block is the default)
            vi_normal: underscore # block, underscore, line  (underscore is the default)
          }
          shell_integration: {
            # osc2 abbreviates the path if in the home_dir, sets the tab/window title, shows the running command in the tab/window title
            osc2: true
            # osc7 is a way to communicate the path to the terminal, this is helpful for spawning new tabs in the same directory
            osc7: true
            # osc8 is also implemented as the deprecated setting ls.show_clickable_links, it shows clickable links in ls output if your terminal supports it. show_clickable_links is deprecated in favor of osc8
            osc8: true
            # osc9_9 is from ConEmu and is starting to get wider support. It's similar to osc7 in that it communicates the path to the terminal
            osc9_9: false
            # osc133 is several escapes invented by Final Term which include the supported ones below.
            # 133;A - Mark prompt start
            # 133;B - Mark prompt end
            # 133;C - Mark pre-execution
            # 133;D;exit - Mark execution finished with exit code
            # This is used to enable terminals to know where the prompt is, the command is, where the command finishes, and where the output of the command is
            osc133: true
            # osc633 is closely related to osc133 but only exists in visual studio code (vscode) and supports their shell integration features
            # 633;A - Mark prompt start
            # 633;B - Mark prompt end
            # 633;C - Mark pre-execution
            # 633;D;exit - Mark execution finished with exit code
            # 633;E - NOT IMPLEMENTED - Explicitly set the command line with an optional nonce
            # 633;P;Cwd=<path> - Mark the current working directory and communicate it to the terminal
            # and also helps with the run recent menu in vscode
            osc633: true
            # reset_application_mode is escape \x1b[?1l and was added to help ssh work better
            reset_application_mode: true
          }
        }
      '';
      extraEnv = ''
        $env.ENV_CONVERSIONS = {
          "PATH": {
            from_string: { |s| $s | split row (char esep) | path expand -n }
            to_string: { |v| $v | path expand -n | str join (char esep) }
          }
          "Path": {
            from_string: { |s| $s | split row (char esep) | path expand -n }
            to_string: { |v| $v | path expand -n | str join (char esep) }
          }
        }
        # Custom environment variables
        $env.BORG_REPO = "/run/media/frederik/LinuxBackup"
        $env.BUP_DIR = "/run/media/frederik/LinuxBackup/.bup"
        $env.FZF_DEFAULT_COMMAND = "fd --type f"
        $env.IPYTHONDIR = "/home/frederik/.config/ipython"
        $env.DEEPL_API_KEY = "***REMOVED***"
        $env.RAISENOW_CLIENT_ID = "***REMOVED***"
        $env.RAISENOW_CLIENT_SECRET = "***REMOVED***"
        $env.ILOVEPDF_PUBLIC_KEY = "***REMOVED***"
        $env.MAUDE_LIB = "/usr/share/maude"
        $env.HELIX_RUNTIME = "/home/frederik/.config/helix/runtime/"
        $env._JAVA_AWT_WM_NONREPARENTING = 1
        $env.TERM = "foot"
        $env.FLAKE = "/home/frederik/.config/home-manager"
        # $env.LOCALE_ARCHIVE = "${pkgs.glibcLocales}/lib/locale/locale-archive";
        # Added this because foot showed locale issues. Found https://gist.github.com/peti/2c818d6cb49b0b0f2fd7c300f8386bc3
        $env.LOCALE_ARCHIVE_2_27 = "${pkgs.glibcLocales}/lib/locale/locale-archive";
        $env.LOCALE_ARCHIVE_2_11 = "/usr/lib/locale/locale-archive";
        $env.COPILOT_API_KEY = "***REMOVED***"
        $env.HANDLER = "copilot"
        $env.SSH_ASKPASS = "/usr/bin/pinentry-bemenu"

        $env.PATH = ($env.PATH | prepend '/home/frederik/.local/bin' | append '/home/frederik/.cargo/bin')

        # use ~/.bin/wolframalpha.nu [ wolfram, wolframimg ]
        use ~/.bin/deeplen.nu
        use ~/.bin/joshuto.nu joshuto_cd

        use ~/.bin/yazi.nu yy

        alias tt = taskwarrior-tui
        alias jo = yy
        alias js = yy

        source /home/frederik/tools/nu_scripts/custom-completions/git/git-completions.nu
        source /home/frederik/tools/nu_scripts/custom-completions/pass/pass-completions.nu
        source /home/frederik/tools/nu_scripts/custom-completions/tealdeer/tldr-completions.nu
        source /home/frederik/tools/nu_scripts/custom-completions/npm/npm-completions.nu
        source /home/frederik/tools/nu_scripts/custom-completions/cargo/cargo-completions.nu
      '';
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
      enable = false;
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
      enable = false;
      settings = {
        main = {
          term = "xterm-256color";
          font = "Consolas ligaturized v3:size=10";
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
