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
      rbw list --fields id,name,type | awk -F '\t' '$3 == "SSH Key" { print $1 "\t" $2 }' \
        | while IFS=$'\t' read -r id name; do
          case "$name" in
            "ssh id_github") confirm=() ;;
            *) confirm=(-c) ;;
          esac
          rbw get --field private_key "$id" \
            | ssh-add -q "''${confirm[@]}" -t ${toString sshKeyLifetime} - || true
        done
      exit 1
    '';
  };

  # ssh-agent runs this before each use of a key added with `ssh-add -c` and
  # passes "Allow use of key <comment>?\nKey fingerprint <fp>."; exit 0
  # allows. The agent never learns the destination, so the clients connected
  # to its socket are looked up and shown with their parent process.
  sshAskpass = pkgs.writeShellApplication {
    name = "ssh-askpass-fuzzel";
    runtimeInputs = [
      pkgs.fuzzel
      pkgs.iproute2
      pkgs.procps
    ];
    text = ''
      pids=$(ss -xpH | awk -v sock="$XDG_RUNTIME_DIR/${config.services.ssh-agent.socket}" '
        $5 == sock { peer[$8] }
        { users[$6] = $9 }
        END {
          for (inode in peer)
            if (match(users[inode], /pid=[0-9]+/)) print substr(users[inode], RSTART + 4, RLENGTH - 4)
        }')
      callers=""
      for pid in $pids; do
        cmd=$(tr '\0' ' ' <"/proc/$pid/cmdline" | sed 's|^/[^ ]*/||') || continue
        parent=$(ps -o args= -p "$(ps -o ppid= -p "$pid" | tr -d ' ')" | sed 's|^/[^ ]*/||' || true)
        callers+=$'\n'"by: $cmd"$'\n'"from: $parent"
      done
      choice=$(printf 'Allow\nDeny\n' | fuzzel --dmenu --mesg "ssh-agent · $1$callers") || exit 1
      [ "$choice" = Allow ]
    '';
  };

  # veila sets logind's LockedHint whenever it locks, also right before
  # suspend; PrepareForSleep covers a suspend without a lock.
  sshAgentClear = pkgs.writeShellApplication {
    name = "ssh-agent-clear-on-lock";
    runtimeInputs = [
      pkgs.glib
      pkgs.openssh
    ];
    text = ''
      gdbus monitor --system --dest org.freedesktop.login1 | while read -r line; do
        case "$line" in
          *"'LockedHint': <true>"* | *"PrepareForSleep (true,)"*) ssh-add -D >/dev/null 2>&1 || true ;;
        esac
      done
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
        # Started with the graphical session so its confirm prompts get
        # WAYLAND_DISPLAY.
        systemd.user.services.ssh-agent = {
          Unit = {
            PartOf = [ "graphical-session.target" ];
            After = [ "graphical-session.target" ];
          };
          Service.Environment = [ "SSH_ASKPASS=${lib.getExe sshAskpass}" ];
          Install.WantedBy = lib.mkForce [ "graphical-session.target" ];
        };
        systemd.user.services.ssh-agent-clear = {
          Unit = {
            Description = "Empty ssh-agent when the session locks or suspends";
            PartOf = [ "graphical-session.target" ];
            After = [ "ssh-agent.service" ];
          };
          Service = {
            ExecStart = lib.getExe sshAgentClear;
            Environment = [ "SSH_AUTH_SOCK=%t/${config.services.ssh-agent.socket}" ];
            Restart = "on-failure";
          };
          Install.WantedBy = [ "graphical-session.target" ];
        };
        programs.ssh.settings.rbw-ssh-add.header = "Match exec \"${lib.getExe rbwSshAdd}\"";
      })
    ]
  );
}
