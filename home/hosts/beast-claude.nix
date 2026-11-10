{
  config,
  lib,
  pkgs,
  ...
}:
let
  claude = lib.getExe config.programs.claude-code.finalPackage;

  # Loads the repo's devenv environment into every Bash call: sessions run in the
  # daemon or under a Remote Control server, never inside devenv shell.
  devenvEnv = pkgs.writeShellApplication {
    name = "claude-devenv-env";
    runtimeInputs = [ pkgs.devenv ];
    text = ''
      [ -z "''${DEVENV_ROOT:-}" ] || exit 0
      cd "$CLAUDE_PROJECT_DIR"
      [ -f devenv.nix ] || exit 0
      devenv shell -- bash -c 'export -p' \
        | grep '^declare -x ' \
        | grep -Ev '^declare -x (PWD|OLDPWD|SHLVL|_)=' >> "$CLAUDE_ENV_FILE"
    '';
  };

  session-group = pkgs.writeShellApplication {
    name = "session-group";
    runtimeInputs = [ pkgs.coreutils ];
    text = ''
      dir="$HOME/.local/state/beast-session-groups"
      session=''${2:-''${CLAUDE_CODE_SESSION_ID:-}}
      if [[ -z $session ]]; then
        echo "usage: session-group [group [session id]], the id is only optional inside Claude Code" >&2
        exit 1
      fi
      if (($# == 0)); then
        echo "this session: $(cat "$dir/$session" 2>/dev/null || echo none)"
        echo "groups in use:"
        cat "$dir"/* 2>/dev/null | sort -u
        exit
      fi
      if ! [[ $1 =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || ((''${#1} > 24)); then
        echo "a group is lowercase words joined by hyphens, at most 24 characters" >&2
        exit 1
      fi
      mkdir -p "$dir"
      printf '%s\n' "$1" >"$dir/$session"
    '';
  };

  session-end = pkgs.writeShellApplication {
    name = "session-end";
    runtimeInputs = [
      config.programs.claude-code.finalPackage
      pkgs.coreutils
      pkgs.git
      pkgs.gnused
      pkgs.jq
      pkgs.procps
      pkgs.shpool
      pkgs.util-linux
    ];
    text = builtins.readFile ./beast-session-end.sh;
  };

  # A server lets the app start sessions in a repo; worktree gives each its own.
  remoteControl = repo: spawn: {
    Unit = {
      Description = "Claude Code Remote Control server in ~/repos/${repo}";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      WorkingDirectory = "%h/repos/${repo}";
      ExecStartPre = "${pkgs.systemd}/lib/systemd/systemd-networkd-wait-online --timeout=0";
      ExecStart = "${claude} remote-control --remote-control-session-name-prefix ${repo} --spawn=${spawn} --no-create-session-in-dir";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };
in
{
  home.packages = [
    session-group
    session-end
    pkgs.pyright
  ];

  programs.claude-code.skills.end = ''
    ---
    name: end
    description: Ends this session for good. Its process stops, its worktree and branch are removed, attached terminals and its shpool session close, and it leaves beast's list and the app.
    disable-model-invocation: true
    ---

    Run `session-end`.

    If it refuses, show the user what it lists as unsaved and ask what to do. Push
    or commit when they say so, then run `session-end` again. Run
    `session-end --force` only when they say to discard the work.

    Once it succeeds, reply with one short line and end your turn: the session
    ends right after.
  '';

  # Hosts every `claude --bg` session. Its sessions get this unit's environment, not
  # the caller's. KillMode=process lets the sessions outlive a restart of the daemon.
  systemd.user.services.claude-daemon = {
    Unit = {
      Description = "Claude Code daemon for background sessions";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      ExecStart = "${claude} daemon run";
      KillMode = "process";
      OOMPolicy = "continue";
      Restart = "always";
      RestartSec = 5;
    };
    Install.WantedBy = [ "default.target" ];
  };

  systemd.user.services.claude-remote-control-nixos-config = remoteControl "nixos-config" "worktree";
  systemd.user.services.claude-remote-control-blackjackeye = remoteControl "blackjackeye" "worktree";
  systemd.user.services.claude-remote-control-spades-website = remoteControl "spades-website" "same-dir";
  systemd.user.services.claude-remote-control-adminwebsite = remoteControl "adminwebsite" "same-dir";
  systemd.user.services.claude-remote-control-holomorphic-visualizer = remoteControl "holomorphic-visualizer" "same-dir";
  systemd.user.services.claude-remote-control-steimannli = remoteControl "steimannli" "same-dir";

  systemd.user.services.beast-session-groups = {
    Unit.Description = "Gives every Claude session on beast without a group one";
    Service = {
      Type = "oneshot";
      ExecStart = "${lib.getExe pkgs.nushell} --no-config-file --commands 'source ${../modules/beast/sessions.nu}; label-sessions | ignore'";
    };
  };
  systemd.user.timers.beast-session-groups = {
    Unit.Description = "Groups new Claude sessions on beast every two minutes";
    Timer = {
      OnBootSec = "2min";
      OnUnitInactiveSec = "2min";
    };
    Install.WantedBy = [ "timers.target" ];
  };

  systemd.user.services.zurich-display-study = {
    Unit.Description = "Daily zurich-23 display study run";
    Service = {
      Type = "oneshot";
      WorkingDirectory = "%h/repos/blackjackeye";
      ExecStart = "${claude} --bg --name zurich-display-study --permission-mode auto 'Daily zurich-23 display study run: follow evaluation/display_study/SCHEDULED.md from origin/zurich-display-study (origin/main once #1175 is merged). Report briefly.'";
    };
  };
  systemd.user.timers.zurich-display-study = {
    Unit.Description = "Starts the display study run after the zurich-23 upload";
    Timer = {
      OnCalendar = "*-*-* 11:17";
      Persistent = true;
    };
    Install.WantedBy = [ "timers.target" ];
  };

  programs.claude-code.settings.hooks.SessionStart = [
    {
      matcher = "startup|resume|clear";
      hooks = [
        {
          type = "command";
          command = lib.getExe devenvEnv;
          # The first shell in a fresh worktree builds the venv and the sv-rs extensions.
          timeout = 1800;
        }
      ];
    }
  ];

  programs.claude-code.context = lib.mkAfter ''

    # Session groups on beast

    The session picker on beast groups Claude sessions by topic. `session-group`
    shows this session's group and the groups in use. When the work here has
    moved to another topic, run `session-group <name>`, reusing a name in use if
    one fits.
  '';

  programs.nushell.extraConfig = ''
    use ${../modules/beast}/beast.nu

    if $nu.is-interactive and ($env.SSH_CONNECTION? != null) {
      beast
    }
  '';
}
