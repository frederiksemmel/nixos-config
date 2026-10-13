{
  flake,
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.dev;
  inherit (pkgs.stdenv.hostPlatform) system;
  llm-agents = flake.inputs.llm-agents.packages.${system};

  hostLabels = lib.mapAttrs (host: icon: "${icon} ${host}") {
    fstux = "💻";
    beast = "🐉";
  };
  claudeStatusLine = pkgs.writeShellApplication {
    name = "claude-status-line";
    runtimeInputs = with pkgs; [
      coreutils
      git
      jq
    ];
    text = ''
      host=$(uname -n)
      case $host in
      ${
        lib.concatStrings (lib.mapAttrsToList (host: label: "  ${host}) host='${label}' ;;\n") hostLabels)
      }esac

      IFS=$'\x1f' read -r dir model effort ctx title transcript < <(jq -r '[
        .workspace.current_dir // .cwd,
        .model.display_name // "",
        .effort.level // "",
        (.context_window.used_percentage // "" | tostring | split(".")[0]),
        .session_name // "",
        .transcript_path // ""
      ] | join("\u001f")')
      if [[ -z $title && -f $transcript ]]; then
        title=$(grep -E '"type":"(custom|ai)-title"' "$transcript" | jq -rs '
          (map(select(.type == "custom-title")) | last | .customTitle)
          // (map(select(.type == "ai-title")) | last | .aiTitle)
          // ""
        ') || true
      fi

      dim=$'\e[2m' bold=$'\e[1m' italic=$'\e[3m' blue=$'\e[34m' magenta=$'\e[35m' cyan=$'\e[36m' reset=$'\e[0m'
      sep=" ''${dim}·''${reset} "

      line="''${bold}$host''${reset}$sep''${blue}$(basename "$dir")''${reset}"
      if branch=$(git -C "$dir" --no-optional-locks branch --show-current 2>/dev/null) && [[ -n $branch ]]; then
        line+=" ''${magenta}⎇ $branch''${reset}"
      fi
      line+="''${title:+$sep''${italic}''${cyan}$title''${reset}}"
      line+="$sep$model''${effort:+ ''${dim}$effort''${reset}}"
      if [[ -n $ctx ]]; then
        if ((ctx >= 80)); then color=$'\e[31m'
        elif ((ctx >= 50)); then color=$'\e[33m'
        else color=$'\e[32m'
        fi
        line+="$sep''${color}$ctx% ctx''${reset}"
      fi
      printf '%s' "$line"
    '';
  };
in
{
  imports = [
    flake.inputs.mcp-servers-nix.homeManagerModules.default
  ];

  options.profiles.dev.enable = lib.mkEnableOption "development tools, shells, editors";

  config = lib.mkIf cfg.enable {
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
      mdcat

      # Containerization & Virtualization
      qemu

      # Version Control & Collaboration
      git-interactive-rebase-tool
      git-graph
      serie
      gitui
      (callPackage ../../packages/muxdiff.nix { })

      # Networking & Communication
      bluetui
      websocat
      # postman

      # File & Data Management
      trashy
      fd
      ripgrep
      sendme
      ffmpeg
      pipe-rename
      imagemagick
      ghostscript
      gdrive3

      # Miscellaneous Utilities
      rage
      ragenix
      zip
      unzip
      ncdu
      google-cloud-sdk
      tealdeer
      usbutils
      pciutils
      difftastic
      pwgen
      tokei
      nmap
      poppler-utils
      exfat
    ];

    home.sessionVariables = {
      EDITOR = "hx";
      TERM = "foot";
      NIXOS_OZONE_WL = "1";
      GLAMOUR_STYLE = "dark";
    };
    systemd.user.sessionVariables = {
      EDITOR = "hx";
      TERM = "foot";
      NIXOS_OZONE_WL = "1";
      # NOTE should not be set in the newer niri versions
      # WAYLAND_DISPLAY = "wayland-1";
      # DISPLAY = ":0";
    };

    # MCP servers, declared once and wired into every agent below.
    mcp-servers.programs = {
      playwright.enable = true;
    };
    mcp-servers.settings.servers.rerun = {
      command = lib.getExe pkgs.rerun;
      args = [ "viewer-mcp" ];
    };
    # Figma's hosted MCP; run `/mcp` once per machine to do the OAuth dance.
    mcp-servers.settings.servers.figma.url = "https://mcp.figma.com/mcp";
    # Linear's hosted MCP; same `/mcp` OAuth dance as Figma.
    mcp-servers.settings.servers.linear.url = "https://mcp.linear.app/mcp";
    # Google Docs/Sheets/Slides editing. The secret is the Desktop-app OAuth
    # client JSON from Google Cloud; the first tool call opens the consent page.
    age.secrets.workspace_mcp_client_secret = {
      file = ../../secrets/workspace_mcp_client_secret.age;
      path = "${config.xdg.configHome}/workspace-mcp/client_secret.json";
    };
    mcp-servers.settings.servers.google-workspace = {
      command = lib.getExe pkgs.workspace-mcp;
      args = [
        "--single-user"
        "--tool-tier"
        "complete"
        "--tools"
        "docs"
        "sheets"
        "slides"
        "drive"
      ];
      env = {
        GOOGLE_CLIENT_SECRET_PATH = config.age.secrets.workspace_mcp_client_secret.path;
        WORKSPACE_MCP_CREDENTIALS_DIR = "${config.xdg.stateHome}/workspace-mcp";
        WORKSPACE_MCP_PORT = "8917";
      };
    };
    # Disabled for now: devenv mcp hogs too much RAM.
    # mcp-servers.settings.servers.devenv = {
    #   command = lib.getExe pkgs.devenv;
    #   args = [ "mcp" ];
    # };

    # Infinity (accounting) MCP, kept out of the shared registry above so no
    # session loads it by default. Start one with the `claude-infinity` alias
    # to get it, then run `/mcp` once per machine to do the OAuth dance.
    xdg.configFile."mcp/infinity.json".source = (pkgs.formats.json { }).generate "infinity-mcp.json" {
      mcpServers.infinity = {
        type = "http";
        url = "https://api.infinity.swiss/mcp";
      };
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
          claude-infinity = "claude --mcp-config ${config.xdg.configHome}/mcp/infinity.json";
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
        extraConfig = ''
          mkdir ~/.cache/devenv | ignore
          devenv hook nu -- --no-tui | save --force ~/.cache/devenv/hook.nu
          source ~/.cache/devenv/hook.nu

          $env.GPG_TTY = (tty)

          # Same completer as carapace's nushell integration, but returning null
          # instead of an empty list so nushell falls back to file completion for
          # arguments carapace has no spec for (e.g. `nu some_script.nu <TAB>`).
          $env.config.completions.external.completer = {|spans|
            load-env {
              CARAPACE_SHELL_BUILTINS: (help commands | where category != "" | get name | each { split row " " | first } | uniq | str join "\n")
              CARAPACE_SHELL_FUNCTIONS: (help commands | where category == "" | get name | each { split row " " | first } | uniq | str join "\n")
            }

            let expanded_alias = scope aliases | where name == $spans.0 | $in.0?.expansion?
            let spans = if $expanded_alias != null {
              $spans | skip 1 | prepend ($expanded_alias | split row " " | take 1)
            } else {
              $spans
            }

            let result = carapace $spans.0 nushell ...$spans | from json
            if ($result | is-empty) { null } else { $result }
          }
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
          python.disabled = true;
          hostname.ssh_only = false;
          hostname.aliases = hostLabels;
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
            selection-target = "both";
          };
          mouse = {
            hide-when-typing = "yes";
          };
          scrollback = {
            lines = 100000;
          };
          # fyi rather than notify-send: it prints the XDG activation token
          # mako hands out on click, which foot needs to focus the window.
          desktop-notifications = {
            command = "${lib.getExe pkgs.fyi} --wait --app-name \${app-id} --icon \${app-id} --category \${category} --urgency \${urgency} --expire-time \${expire-time} --hint string:image-path:\${icon} --hint boolean:suppress-sound:\${muted} --hint string:sound-name:\${sound-name} --replaces \${replace-id} \${action-argument} --print-id --print-token -- \${title} \${body}";
            command-action-argument = "--action \${action-name}:\${action-label}";
            close = "${lib.getExe pkgs.fyi} --close \${id}";
          };
          colors-dark = {
            background = "1a1a1a";
            foreground = "d4d4d4";
            regular0 = "2a2a2a";
            regular1 = "e06c75";
            regular2 = "50a14f"; # green: a more "greener" but muted tone
            regular3 = "e5c07b";
            regular4 = "61afef";
            regular5 = "c678dd";
            regular6 = "56b6c2";
            regular7 = "c8c8c8";
            bright0 = "6e6e6e";
            bright1 = "ea858b";
            bright2 = "98c379"; # bright green: complements the normal green with a softer, bright appearance
            bright3 = "f0d197";
            bright4 = "82c0f5";
            bright5 = "d39ae6";
            bright6 = "7ccbd4";
            bright7 = "e8e8e8";
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
                  userDictPath = "~/.config/harper-ls/dictionary.txt";
                  fileDictPath = "~/.harper/";
                  # Per-project jargon goes in `.harper-dictionary.txt` at the
                  # workspace root, which harper-ls picks up on its own.
                  dialect = "american";
                  # Kept in sync with harper.toml in the blackjackeye repo.
                  linters = {
                    LongSentences = false;
                    SentenceCapitalization = false;
                    UseTitleCase = false;
                    CapitalizePersonalPronouns = false;
                    OrthographicConsistency = false;
                    Dashes = false;
                    UnclosedQuotes = false;
                    QuoteSpacing = false;
                    UseEllipsisCharacter = false;
                    EllipsisLength = false;
                    NumericRangeEnDash = false;
                    NoFrenchSpaces = false;
                    ExpandAlgorithm = false;
                    ExpandAlloc = false;
                    ExpandArgument = false;
                    ExpandBecause = false;
                    ExpandConfiguration = false;
                    ExpandControl = false;
                    ExpandCoordinate = false;
                    ExpandDecl = false;
                    ExpandDependencies = false;
                    ExpandDereference = false;
                    ExpandDirectory = false;
                    ExpandFavourite = false;
                    ExpandForward = false;
                    ExpandGovt = false;
                    ExpandLegitimate = false;
                    ExpandMemoryShorthands = false;
                    ExpandMinimum = false;
                    ExpandNotification = false;
                    ExpandParameter = false;
                    ExpandPeople = false;
                    ExpandPerformance = false;
                    ExpandPointer = false;
                    ExpandPreference = false;
                    ExpandPrevious = false;
                    ExpandStandardInputAndOutput = false;
                    ExpandThough = false;
                    ExpandThrough = false;
                    ExpandTimeShorthands = false;
                    ExpandVulnerability = false;
                    ExpandWith = false;
                    ExpandWithout = false;
                    SplitWords = false;
                    CompoundNouns = false;
                    MergeWords = false;
                    DisjointPrefixes = false;
                    PhrasalVerbAsCompoundNoun = false;
                    WrongNegative = false;
                    OxfordComma = false;
                    ToDoHyphen = false;
                    DiscourseMarkers = false;
                    CommaFixes = false;
                    MassNouns = false;
                    QuantifierNumeralConflict = false;
                    MissingTo = false;
                    MissingPreposition = false;
                  };
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

      git = {
        enable = true;
        settings = {
          merge.conflictstyle = "diff3";
          diff.colorMoved = "default";
          sequence.editor = "interactive-rebase-tool";
          init.defaultBranch = "main";
          fetch.prune = true;
        };
      };

      gh = {
        enable = true;
        settings.git_protocol = "ssh";
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
      };

      yazi = {
        enable = true;
        shellWrapperName = "yy";
        enableNushellIntegration = true;
        settings = {
          sort-by = "natural";
        };
        theme = {
          indicator.padding = {
            open = "█";
            close = "█";
          };
        };
        plugins = {
          inherit (pkgs.yaziPlugins) clipboard;
        };
        keymap.mgr.prepend_keymap = [
          {
            on = "y";
            run = [
              "yank"
              "plugin clipboard -- --action=copy"
            ];
            desc = "Yank selected files (copy)";
          }
          {
            on = "x";
            run = [
              "yank --cut"
              "plugin clipboard -- --action=copy"
            ];
            desc = "Yank selected files (cut)";
          }
          {
            on = "<C-p>";
            run = "plugin clipboard -- --action=paste";
            desc = "Paste files from system clipboard";
          }
        ];
      };

      bottom.enable = true;

      nh = {
        enable = true;
        clean.enable = false;
        clean.extraArgs = "--keep-since 14d --keep 10";
        flake = "${config.home.homeDirectory}/repos/nixos-config";
      };

      # MCP server registry, consumed by every agent with enableMcpIntegration.
      mcp.enable = true;

      claude-code = {
        enable = true;
        package = llm-agents.claude-code;
        enableMcpIntegration = true;

        # NOTE: setting `settings` makes ~/.claude/settings.json a read-only
        # store symlink, so everything that used to live there is declared here.
        # The CLI can no longer write it: `/config`, `/model` and `/plugin` only
        # hold for the session unless the change is made below.
        settings = {
          model = "opus";
          effortLevel = "high";
          modelSettings.claude-opus-5-5.effortLevel = "high";
          tui = "fullscreen";
          skipDangerousModePermissionPrompt = true;
          # The auto-mode twin of the line above: accept the opt-in dialog once,
          # declaratively, instead of once per machine.
          skipAutoPermissionPrompt = true;

          statusLine = {
            type = "command";
            command = lib.getExe claudeStatusLine;
          };

          permissions = {
            # Auto mode has a classifier vet every call, so this allow list is
            # about latency, not trust: these skip the round trip. Rules the CLI
            # learns from a prompt still land in the project's
            # .claude/settings.local.json, which stays writable.
            defaultMode = "auto";

            allow = [
              # Looking around.
              "Bash(ls:*)"
              "Bash(cat:*)"
              "Bash(head:*)"
              "Bash(tail:*)"
              "Bash(wc:*)"
              "Bash(which:*)"
              "Bash(grep:*)"
              "Bash(rg:*)"
              "Bash(fd:*)"
              "Bash(find:*)"
              "Bash(jq:*)"
              "Bash(sed -n:*)"
              # Reading history, never rewriting it.
              "Bash(git status:*)"
              "Bash(git diff:*)"
              "Bash(git log:*)"
              "Bash(git show:*)"
              "Bash(git blame:*)"
              "Bash(git branch:*)"
              "Bash(git rev-parse:*)"
              "Bash(git ls-files:*)"
              "Bash(git remote:*)"
              "Bash(git worktree list:*)"
              "Bash(git fetch:*)"
              # Staging and committing: local, and the reflog keeps a way back.
              "Bash(git add:*)"
              "Bash(git commit:*)"
              "Bash(git stash:*)"
              # The toolchains the repos build and check with.
              "Bash(uv run:*)"
              "Bash(ruff check:*)"
              "Bash(ruff format:*)"
              "Bash(ty check:*)"
              "Bash(cargo build:*)"
              "Bash(cargo check:*)"
              "Bash(cargo clippy:*)"
              "Bash(cargo test:*)"
              "Bash(cargo fmt:*)"
              "Bash(npm run:*)"
              "Bash(nix eval:*)"
              "Bash(nh os build:*)"
              # Reading GitHub, including whatever gh grew no subcommand for.
              "Bash(gh pr view:*)"
              "Bash(gh pr list:*)"
              "Bash(gh pr diff:*)"
              "Bash(gh pr checks:*)"
              "Bash(gh issue:*)"
              "Bash(gh api:*)"
              "Bash(gh run view:*)"
              "Bash(gh run list:*)"
              "Bash(gh repo view:*)"
              "Bash(gh search:*)"
              # Outward-facing, but the round trip costs more than it catches.
              "Bash(git push:*)"
              "Bash(gh pr create:*)"
              "Bash(gh pr merge:*)"
              # SSH because its super practical
              "Bash(ssh:*)"
              # Google Docs/Sheets/Slides/Drive on the work account.
              "mcp__plugin_hm_google-workspace"
            ];

            ask = [
              # Outward-facing or hard to walk back.
              "Bash(sudo:*)"
              "Bash(nixos-rebuild:*)"
              "Bash(nh:*)"
              # Change who a company file is shared with.
              "mcp__plugin_hm_google-workspace__manage_drive_access"
              "mcp__plugin_hm_google-workspace__set_drive_file_permissions"
            ];

            deny = [
              # Evaluates every host at once and fills RAM; check one named host
              # or parse the changed files instead.
              "Bash(nix flake check:*)"
              "Read(//home/frederik/.ssh/**)"
              "Read(//**/*.age)"
              "Read(//**/.env)"
            ];
          };

          enabledPlugins = {
            "astral@astral-sh" = true;
            "pyright-lsp@claude-plugins-official" = true;
            "rust-analyzer-lsp@claude-plugins-official" = true;
            "typescript-lsp@claude-plugins-official" = true;
            "frontend-design@claude-plugins-official" = true;
            "mattpocock-skills@claude-plugins-official" = true;
          };

          # Not the `marketplaces` option: that one only emits `source =
          # "directory"` entries, and this marketplace is a GitHub repo.
          extraKnownMarketplaces.astral-sh.source = {
            source = "github";
            repo = "astral-sh/claude-code-plugins";
          };

          hooks =
            let
              notify = {
                type = "command";
                command = lib.getExe pkgs.claude-notify;
              };
            in
            {
              # Fires when a session wants something from you: a permission
              # dialog, a 60s-idle prompt, an MCP form, or a waiting subagent.
              Notification = [
                {
                  matcher = "permission_prompt|idle_prompt|elicitation_dialog|elicitation_url_dialog|agent_needs_input";
                  hooks = [ notify ];
                }
              ];
              # Turn complete, i.e. the prompt is yours again. Drop this entry to
              # only get notified once the prompt has actually sat idle for 60s.
              Stop = [ { hooks = [ notify ]; } ];
              # The turn died on an API error: rate limit, overload, auth.
              StopFailure = [ { hooks = [ notify ]; } ];
              # Clears the session's notification once you are typing again.
              UserPromptSubmit = [ { hooks = [ notify ]; } ];
            };
        };
      };

      # Let Home Manager install and manage itself.
      home-manager.enable = true;
    };
  };
}
