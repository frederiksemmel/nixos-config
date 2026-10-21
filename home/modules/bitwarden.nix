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

  # Speaks just enough of the Assuan protocol for rbw: it sends SETTITLE,
  # SETPROMPT, SETDESC, maybe SETERROR, then GETPIN, and closes stdin.
  rbwPinentry = pkgs.writeShellApplication {
    name = "rbw-pinentry";
    runtimeInputs = [ pkgs.yofi ];
    text = ''
      echo "OK"
      label=""
      desc=""
      error=""
      while IFS= read -r line; do
        case "$line" in
          "SETPROMPT "*)
            label=''${line#SETPROMPT }
            label=''${label//__/_}
            echo "OK"
            ;;
          "SETDESC "*)
            desc=''${line#SETDESC }
            echo "OK"
            ;;
          "SETERROR "*)
            error=''${line#SETERROR }
            echo "OK"
            ;;
          GETPIN)
            prompt="rbw · $desc · $label"
            if [ -n "$error" ]; then
              prompt="$error · $prompt"
            fi
            pin=$(yofi --password --config-file "${config.theme.dir}/current/yofi-prompt.toml" \
              -p "$prompt" dialog </dev/null || true)
            if [ -n "$pin" ]; then
              printf 'D %s\nOK\n' "''${pin//%/%25}"
            else
              echo "ERR 83886179 Operation cancelled"
            fi
            ;;
          BYE)
            echo "OK"
            exit 0
            ;;
          *) echo "OK" ;;
        esac
      done
    '';
  };

  rbwMenu = pkgs.writeShellApplication {
    name = "rbw-menu";
    runtimeInputs = [
      pkgs.rbw
      pkgs.yofi
      pkgs.wl-clipboard
      pkgs.wtype
      pkgs.libnotify
    ];
    text = ''
      rbw unlocked 2>/dev/null || rbw unlock
      rbw sync 2>/dev/null &

      entries=$(rbw list --fields id,name,user,type | awk -F '\t' '$4 == "Login"')
      choice=$(awk -F '\t' '{ print ($3 == "" ? $2 : $2 "  ·  " $3) }' <<<"$entries" \
        | yofi -p Bitwarden dialog || true)
      [ -n "$choice" ] || exit 0
      id=$(awk -F '\t' -v c="$choice" '($3 == "" ? $2 : $2 "  ·  " $3) == c { print $1; exit }' <<<"$entries")
      [ -n "$id" ] || exit 0

      action=$(printf '%s\n' \
        "Copy password" \
        "Copy username" \
        "Copy TOTP" \
        "Type username, tab, password" \
        "Type password" \
        | yofi -p "$choice" dialog || true)

      copy() {
        local secret
        secret=$(rbw get --field "$1" "$id") || {
          notify-send -a rbw "Bitwarden" "No $1 for $choice"
          exit 1
        }
        printf '%s' "$secret" | wl-copy --sensitive
        notify-send -a rbw -t 2000 "Bitwarden" "Copied $1 of $choice, cleared in 30s"
        (
          sleep 30
          [ "$(wl-paste --no-newline 2>/dev/null)" = "$secret" ] && wl-copy --clear
        ) >/dev/null 2>&1 &
        disown
      }

      case "$action" in
        "Copy password") copy password ;;
        "Copy username") copy username ;;
        "Copy TOTP") copy totp ;;
        "Type username, tab, password")
          sleep 0.2
          wtype -- "$(rbw get --field username "$id")"
          wtype -k Tab
          wtype -- "$(rbw get "$id")"
          ;;
        "Type password")
          sleep 0.2
          wtype -- "$(rbw get "$id")"
          ;;
      esac
    '';
  };

  # ssh runs this through `Match exec` on every connection. With the agent
  # empty it unlocks rbw and loads every SSH key in the vault; it never
  # matches, so the block itself sets nothing.
  rbwSshAdd = pkgs.writeShellApplication {
    name = "rbw-ssh-add";
    runtimeInputs = [
      pkgs.rbw
      pkgs.openssh
    ];
    text = ''
      ssh-add -l >/dev/null 2>&1 && exit 1
      rbw unlocked 2>/dev/null || rbw unlock || exit 1
      rbw list --fields id,type | awk -F '\t' '$2 == "SSH Key" { print $1 }' \
        | while IFS= read -r id; do
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
