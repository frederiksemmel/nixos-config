{
  config,
  lib,
  pkgs,
  ...
}:
let
  systemdRun = "${pkgs.systemd}/bin/systemd-run --user --collect --quiet";

  # Opens beast's files in this desktop session. Beast's open-on-fstux runs it over
  # `ssh fstux`. Videos stream over sftp, images are copied here for as long as
  # imv shows them.
  openFromBeast = pkgs.writers.writeNuBin "open-from-beast" ''
    def main [mode: string, ...paths: string] {
      let session = ^${pkgs.systemd}/bin/systemctl --user show-environment
        | lines
        | parse "{name}={value}"
        | where name in [SSH_AUTH_SOCK WAYLAND_DISPLAY DISPLAY]
        | reduce --fold {} {|var, acc| $acc | insert $var.name $var.value }
      with-env $session {
        match $mode {
          video => {
            ^${systemdRun} -- ${lib.getExe config.programs.mpv.finalPackage} -- ...($paths | each {|p| $"sftp://beast($p)" })
          }
          image => {
            let dir = mktemp --directory --tmpdir-path $env.XDG_RUNTIME_DIR from-beast.XXXXXX
            ^${pkgs.openssh}/bin/scp -q ...($paths | each {|p| $"beast:($p)" }) $dir
            ^${systemdRun} $"--property=ExecStopPost=${pkgs.coreutils}/bin/rm -rf ($dir)" -- ${lib.getExe config.programs.imv.package} $dir
          }
          _ => { error make --unspanned { msg: "usage: open-from-beast image|video ...paths-on-beast" } }
        }
      }
    }
  '';
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
