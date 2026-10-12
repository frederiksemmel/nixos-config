{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.bitwarden;
  socket = "${config.home.homeDirectory}/.bitwarden-ssh-agent.sock";
  bw = lib.getExe' pkgs.bitwarden-cli "bw";
  server = "https://vault.bitwarden.eu";
in
{
  options.profiles.bitwarden = {
    enable = lib.mkEnableOption "Bitwarden desktop and CLI";
    sshAgent.enable = lib.mkEnableOption "the Bitwarden desktop app as the SSH agent";
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = [
          pkgs.bitwarden-desktop
          pkgs.bitwarden-cli
        ];
        home.activation.bitwardenServer = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
          status=$(${bw} status 2>/dev/null || true)
          case "$status" in
            *'"serverUrl":"${server}"'*) ;;
            *'"status":"unauthenticated"'*) run ${bw} config server ${server} >/dev/null ;;
            *) warnEcho "bw is logged in to another server than ${server}; run bw logout" ;;
          esac
        '';
        programs.nushell.extraConfig = ''
          def --env bw-unlock [] {
            $env.BW_SESSION = (bw unlock --raw)
          }
        '';
      }
      (lib.mkIf cfg.sshAgent.enable {
        home.sessionVariables.SSH_AUTH_SOCK = socket;
        systemd.user.sessionVariables.SSH_AUTH_SOCK = socket;
        systemd.user.services.bitwarden = {
          Unit = {
            Description = "Bitwarden desktop (SSH agent)";
            PartOf = [ "graphical-session.target" ];
            After = [ "graphical-session.target" ];
          };
          Service = {
            ExecStart = lib.getExe' pkgs.bitwarden-desktop "bitwarden";
            Restart = "on-failure";
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
      })
    ]
  );
}
