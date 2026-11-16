{
  config,
  lib,
  pkgs,
  ...
}:
let
  # Opens beast's files in this desktop session. Beast's open-on-fstux runs it over
  # `ssh fstux` with the paths NUL-separated on stdin. Videos stream over sftp,
  # images are copied here for as long as imv shows them.
  openFromBeast = pkgs.writeShellApplication {
    name = "open-from-beast";
    runtimeInputs = [
      pkgs.openssh
      pkgs.coreutils
      pkgs.gnugrep
      pkgs.gnused
      pkgs.systemd
    ];
    text = ''
      mode=''${1:-}
      mapfile -d "" paths
      eval "$(systemctl --user show-environment | grep -E '^(SSH_AUTH_SOCK|WAYLAND_DISPLAY|DISPLAY)=' | sed 's/^/export /')"
      case $mode in
        video)
          urls=()
          for p in "''${paths[@]}"; do urls+=("sftp://beast$p"); done
          systemd-run --user --collect --quiet -- ${lib.getExe config.programs.mpv.finalPackage} -- "''${urls[@]}"
          ;;
        image)
          dir=$(mktemp -d "''${XDG_RUNTIME_DIR:?}/from-beast.XXXXXX")
          scp -q "''${paths[@]/#/beast:}" "$dir/"
          systemd-run --user --collect --quiet --property="ExecStopPost=${pkgs.coreutils}/bin/rm -rf $dir" \
            -- ${lib.getExe config.programs.imv.package} "$dir"
          ;;
        *)
          echo "usage: open-from-beast image|video < NUL-separated paths on beast" >&2
          exit 2
          ;;
      esac
    '';
  };
in
{
  options.profiles.beastSessions.enable = lib.mkEnableOption "the laptop's side of beast: the beast nushell command for Claude sessions on beast in terminal windows that reattach after the connection drops, and open-from-beast showing beast's images and videos here";

  config = lib.mkIf config.profiles.beastSessions.enable {
    programs.ssh.settings.beast = {
      ServerAliveInterval = 10;
      ServerAliveCountMax = 2;
    };

    programs.nushell.extraConfig = ''
      use ${./beast}/beast.nu
    '';

    home.packages = [ openFromBeast ];
  };
}
