{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.profiles.bitwarden;
  bw = lib.getExe' pkgs.bitwarden-cli "bw";
  server = "https://vault.bitwarden.eu";
  sshKeyLifetime = 24 * 3600;

  # rbw sends SETTITLE, SETPROMPT, SETDESC, maybe SETERROR and GETPIN, then
  # waits for an OK to each and a D line with the password.
  rbwPinentry = pkgs.writeShellApplication {
    name = "rbw-pinentry";
    runtimeInputs = [ pkgs.fuzzel ];
    text = ''
      echo OK
      label="" desc="" error=""
      while read -r cmd arg; do
        case "$cmd" in
          SETPROMPT) label=''${arg//__/_} ;;
          SETDESC) desc=$arg ;;
          SETERROR) error="$arg · " ;;
          GETPIN)
            pin=$(fuzzel --dmenu --password --prompt-only "$label: " \
              --mesg "''${error}rbw · $desc" </dev/null || true)
            if [ -z "$pin" ]; then
              echo "ERR 83886179 Operation cancelled"
              exit 0
            fi
            echo "D ''${pin//%/%25}"
            ;;
        esac
        echo OK
      done
    '';
  };

  rbwMenu = pkgs.writeShellApplication {
    name = "rbw-menu";
    runtimeInputs = [
      pkgs.rbw
      pkgs.fuzzel
      pkgs.wl-clipboard
      pkgs.wtype
      pkgs.libnotify
    ];
    text = ''
      # "<name>  ·  <user>\t<id>" for every login; fuzzel shows the first column
      picked=$(rbw list --fields id,name,user,type \
        | awk -F '\t' '$4 == "Login" { print ($3 == "" ? $2 : $2 "  ·  " $3) "\t" $1 }' \
        | fuzzel --dmenu --with-nth 1 --placeholder Bitwarden) || exit 0
      IFS=$'\t' read -r choice id <<<"$picked"

      action=$(printf '%s\n' "Copy password" "Copy username" "Copy TOTP" \
        "Type username, tab, password" "Type password" \
        | fuzzel --dmenu --mesg "$choice") || exit 0

      copy() {
        local secret
        secret=$(rbw get --field "$1" "$id")
        printf '%s' "$secret" | wl-copy --sensitive
        notify-send -a rbw -t 2000 Bitwarden "Copied $1 of $choice, cleared in 30s"
        (
          sleep 30
          [ "$(wl-paste --no-newline)" = "$secret" ] && wl-copy --clear
        ) >/dev/null 2>&1 &
      }
      typeout() {
        sleep 0.2
        wtype -- "$(rbw get --field "$1" "$id")"
      }

      case "$action" in
        "Copy password") copy password ;;
        "Copy username") copy username ;;
        "Copy TOTP") copy totp ;;
        "Type username, tab, password") typeout username && wtype -k Tab && typeout password ;;
        "Type password") typeout password ;;
      esac
    '';
  };

  # Run by ssh's `Match exec` on every connection: loads the vault's SSH keys
  # when the agent is empty, and never matches.
  rbwSshAdd = pkgs.writeShellApplication {
    name = "rbw-ssh-add";
    runtimeInputs = [
      pkgs.rbw
      pkgs.openssh
    ];
    text = ''
      ssh-add -l >/dev/null 2>&1 && exit 1
      rbw list --fields id,type | awk -F '\t' '$2 == "SSH Key" { print $1 }' \
        | while read -r id; do
          rbw get --field private_key "$id" | ssh-add -q -t ${toString sshKeyLifetime} - || true
        done
      exit 1
    '';
  };
in
{
  options.profiles.bitwarden = {
    enable = lib.mkEnableOption "Bitwarden desktop and CLI";
    sshAgent.enable = lib.mkEnableOption "ssh-agent loaded with the vault's SSH keys through rbw";
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = [
          pkgs.bitwarden-desktop
          pkgs.bitwarden-cli
          rbwMenu
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
        programs.rbw = {
          enable = true;
          settings = {
            email = "frederiksemmel@gmail.com";
            base_url = server;
            lock_timeout = 3600;
            pinentry = rbwPinentry;
          };
        };
      }
      (lib.mkIf cfg.sshAgent.enable {
        home.packages = [ rbwSshAdd ];
        services.ssh-agent = {
          enable = true;
          defaultMaximumIdentityLifetime = sshKeyLifetime;
        };
        programs.ssh.settings.rbw-ssh-add.header = "Match exec \"${lib.getExe rbwSshAdd}\"";
      })
    ]
  );
}
