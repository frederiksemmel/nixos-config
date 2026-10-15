{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.theme;
  dir = "${config.xdg.stateHome}/theme";

  # The greys the desktop is built on, with One Dark / One Light accents.
  # `terminal` is foot's 16-colour palette, written without the leading #.
  palettes = {
    dark = {
      name = "dark";
      bg = "#000000";
      bgAlt = "#1a1a1a";
      surface = "#0d0d0d";
      border = "#2e2e2e";
      focus = "#505050";
      fg = "#ffffff";
      muted = "#8a8a8a";
      faint = "#5c5c5c";
      red = "#e06c75";
      green = "#98c379";
      yellow = "#e5c07b";
      blue = "#61afef";
      magenta = "#c678dd";
      cyan = "#56b6c2";
      orange = "#d97757";
      terminal = {
        background = "1a1a1a";
        foreground = "d4d4d4";
        regular0 = "2a2a2a";
        regular1 = "e06c75";
        regular2 = "50a14f";
        regular3 = "e5c07b";
        regular4 = "61afef";
        regular5 = "c678dd";
        regular6 = "56b6c2";
        regular7 = "c8c8c8";
        bright0 = "6e6e6e";
        bright1 = "ea858b";
        bright2 = "98c379";
        bright3 = "f0d197";
        bright4 = "82c0f5";
        bright5 = "d39ae6";
        bright6 = "7ccbd4";
        bright7 = "e8e8e8";
      };
    };
    light = {
      name = "light";
      bg = "#fafafa";
      bgAlt = "#ebebeb";
      surface = "#f2f2f2";
      border = "#d4d4d6";
      focus = "#c8c9ce";
      fg = "#383a42";
      muted = "#696c77";
      faint = "#a0a1a7";
      red = "#e45649";
      green = "#50a14f";
      yellow = "#c18401";
      blue = "#4078f2";
      magenta = "#a626a4";
      cyan = "#0184bc";
      orange = "#c4623f";
      terminal = {
        background = "fafafa";
        foreground = "383a42";
        regular0 = "383a42";
        regular1 = "e45649";
        regular2 = "50a14f";
        regular3 = "c18401";
        regular4 = "4078f2";
        regular5 = "a626a4";
        regular6 = "0184bc";
        regular7 = "a0a1a7";
        bright0 = "696c77";
        bright1 = "ca1243";
        bright2 = "3e953a";
        bright3 = "986801";
        bright4 = "2f5af3";
        bright5 = "8b1e8a";
        bright6 = "0e7da8";
        bright7 = "d4d4d6";
      };
    };
  };

  # Every registered file, rendered once per mode into one directory.
  filesFor =
    mode:
    pkgs.linkFarm "theme-${mode}" (
      lib.mapAttrsToList (name: render: {
        inherit name;
        path = pkgs.writeText "theme-${mode}-${name}" (render cfg.palettes.${mode});
      }) cfg.files
    );

  apply = pkgs.writeShellApplication {
    name = "theme-apply";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.procps
    ];
    text = ''
      mode=''${1:-}
      case "$mode" in
        dark) files=${filesFor "dark"} ;;
        light) files=${filesFor "light"} ;;
        *) echo "usage: theme-apply dark|light" >&2; exit 2 ;;
      esac
      mkdir -p ${dir}
      ln -sfn "$files" ${dir}/current
      echo "$mode" > ${dir}/mode
      # Live apps are told best-effort: during activation there may be no
      # session to tell.
      live() { timeout 5 "$@" >/dev/null 2>&1 || true; }
      ${cfg.onSwitch}
    '';
  };
in
{
  options.theme = {
    enable = lib.mkEnableOption "the dark/light theme switch (darkman)";

    palettes = lib.mkOption {
      type = lib.types.attrs;
      default = palettes;
      readOnly = true;
      description = "The dark and light palettes every themed program reads.";
    };

    dir = lib.mkOption {
      type = lib.types.str;
      default = dir;
      readOnly = true;
      description = ''
        Where the switch keeps its state: `mode`, and `current`, a link to the
        active mode's rendering of {option}`theme.files`.
      '';
    };

    files = lib.mkOption {
      type = lib.types.attrsOf (lib.types.functionTo lib.types.lines);
      default = { };
      description = ''
        Files rendered for each mode and reachable as `''${theme.dir}/current/<name>`.
        Each is a function of the palette.
      '';
    };

    onSwitch = lib.mkOption {
      type = lib.types.lines;
      default = "";
      description = ''
        Shell run after every switch, with `$mode` and `$files` set. Wrap
        commands that talk to the session in `live`.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ apply ];

    services.darkman = {
      enable = true;
      # No location and no geoclue: the mode only changes when asked to.
      # The Settings portal stays with xdg-desktop-portal-gnome, which reads
      # the dconf key theme-apply writes.
      settings = {
        usegeoclue = false;
        portal = false;
      };
      scripts.theme = ''${apply}/bin/theme-apply "$1"'';
    };

    # darkman starts dark unless it remembers otherwise, and a rebuild must
    # repoint `current` at the new generation's files.
    home.activation.theme = lib.hm.dag.entryAfter [ "writeBoundary" "dconfSettings" ] ''
      cache=${config.xdg.cacheHome}/darkman/mode.txt
      if [ ! -e "$cache" ]; then
        run install -D -m600 ${pkgs.writeText "darkman-mode" "dark"} "$cache"
      fi
      run ${apply}/bin/theme-apply "$(cat ${dir}/mode 2>/dev/null || echo dark)"
    '';
  };
}
