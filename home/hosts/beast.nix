{
  config,
  lib,
  pkgs,
  ...
}:
let
  # Keeps one untouched `claude --remote-control` session in a detached shpool
  # session named rc-<n>, so one is always waiting in the app. A session counts
  # as used once its transcript has a user message; then another is started.
  # Also drops shpool sessions whose process has exited, e.g. after /exit: shpool
  # keeps those listed, and `shpool kill` only removes one with a live process.
  claude-rc-spare = pkgs.writeShellApplication {
    name = "claude-rc-spare";
    runtimeInputs = with pkgs; [
      coreutils
      gnugrep
      jq
      shpool
    ];
    text = ''
      dir="$HOME/repos/blackjackeye"
      while true; do
        for name in $(shpool list --json | jq -r '.sessions[].name'); do
          if ! grep -qszxF "SHPOOL_SESSION_NAME=$name" /proc/[0-9]*/environ; then
            shpool attach --background --cmd "sleep 60" "$name"
            shpool kill "$name"
          fi
        done
        names=$(shpool list --json | jq -r '.sessions[].name | select(startswith("rc-"))')
        fresh=0
        for name in $names; do
          used=0
          for f in "$HOME"/.claude/sessions/*.json; do
            pid=$(jq -r .pid "$f" 2>/dev/null) || continue
            [[ -r /proc/$pid/environ ]] || continue
            grep -qxzF "SHPOOL_SESSION_NAME=$name" "/proc/$pid/environ" || continue
            cwd=$(jq -r .cwd "$f")
            session=$(jq -r .sessionId "$f")
            if grep -qs '"type":"user"' "$HOME/.claude/projects/''${cwd//[^A-Za-z0-9]/-}/$session.jsonl"; then
              used=1
            fi
          done
          if ((used == 0)); then
            fresh=$((fresh + 1))
          fi
        done
        if ((fresh == 0)); then
          n=1
          while grep -qx "rc-$n" <<< "$names"; do
            n=$((n + 1))
          done
          shpool attach --background --dir "$dir" --cmd "devenv shell --no-tui -- claude --remote-control" "rc-$n"
        fi
        sleep 10
      done
    '';
  };

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

  remoteControl = repo: {
    Unit = {
      Description = "Claude Code Remote Control server in ~/repos/${repo}";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      WorkingDirectory = "%h/repos/${repo}";
      ExecStartPre = "${pkgs.systemd}/lib/systemd/systemd-networkd-wait-online --timeout=0";
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --remote-control-session-name-prefix ${repo} --spawn=worktree --no-create-session-in-dir";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };
in
{
  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  programs.ssh.settings.fstux = {
    IdentityFile = "~/.ssh/id_fstux";
    IdentitiesOnly = true;
  };

  nix.package = pkgs.nix;
  nix.settings = {
    builders = "ssh://eu.nixbuild.net x86_64-linux,aarch64-linux - 100 4 benchmark,big-parallel";
    builders-use-substitutes = true;
    substituters = [
      "https://cache.nixos.org"
      "https://niri.cachix.org"
      "https://nix-community.cachix.org"
    ];
    trusted-public-keys = [
      "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
      "niri.cachix.org-1:Wv0OmO7PsuocRKzfDoJ3mulSl7Z6oezYhGhR+3W2964="
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
    ];
  };
  xdg.configFile."nix/nix.conf".force = true;

  programs.rclone.remotes.pcloud.mounts."".enable = lib.mkForce false;

  services.pueue.enable = true;

  services.shpool = {
    enable = true;
    settings = {
      prompt_prefix = "";
      motd = "never";
      forward_env = [
        "PATH"
        "NIX_PATH"
        "NIX_PROFILES"
        "NIX_USER_PROFILE_DIR"
        "NIXPKGS_CONFIG"
        "LOCALE_ARCHIVE"
        "LANG"
        "TZDIR"
        "TERMINFO_DIRS"
        "XDG_CONFIG_DIRS"
        "XDG_DATA_DIRS"
        "INFOPATH"
        "LIBEXEC_PATH"
        "EDITOR"
        "PAGER"
      ];
    };
  };
  systemd.user.services.shpool.Unit.X-SwitchMethod = "keep-old";

  systemd.user.services.claude-rc-spare = {
    Unit = {
      Description = "Reaps exited shpool sessions and keeps one fresh Claude Code Remote Control session";
      Requires = [ "shpool.socket" ];
      After = [ "shpool.socket" ];
    };
    Service = {
      ExecStart = lib.getExe claude-rc-spare;
      Environment = [ "TERM=foot" ];
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Remote Control servers, so sessions can be spawned from the app, each in its
  # own worktree. They run beside the blackjackeye spare above.
  systemd.user.services.claude-remote-control-nixos-config = remoteControl "nixos-config";
  systemd.user.services.claude-remote-control-blackjackeye = remoteControl "blackjackeye";

  # Server-spawned sessions don't start inside devenv shell, so their worktree's
  # devenv environment is snapshotted once; Claude sources CLAUDE_ENV_FILE before
  # every Bash call.
  programs.claude-code.settings.hooks.SessionStart = [
    {
      matcher = "startup";
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

  programs.nushell.extraConfig = ''
    def shpool-session-name [pid] {
      open --raw $"/proc/($pid)/environ" | split row (char nul) | where $it starts-with "SHPOOL_SESSION_NAME=" | first | str substring 20..
    }

    def shpool-sessions [] {
      let sessions = shpool list --json | from json | get sessions
      let daemon = ^pgrep -u $env.USER -x shpool | lines | first
      let shells = ^pgrep -P $daemon | lines | each {|pid|
        try {
          let fg = open --raw $"/proc/($pid)/stat" | split row ") " | last | split row " " | get 5
          { name: (shpool-session-name $pid), fg: $fg }
        }
      } | compact
      let claudes = glob $"($env.HOME)/.claude/sessions/*.json" | each {|f|
        try {
          let s = open $f
          let transcript = $"($env.HOME)/.claude/projects/($s.cwd | str replace --all --regex '[^A-Za-z0-9]' '-')/($s.sessionId).jsonl"
          let titles = try { ^grep -E '"type":"(custom|ai)-title"' $transcript | lines | each { from json } | collect } catch { [] }
          let prompt = try {
            ^grep -m1 '"type":"user"' $transcript | from json | get message.content
            | if ($in | describe | str starts-with "string") { $in } else { $in | where type == "text" | get text | str join " " }
            | lines | str trim | compact --empty
            | where {|l| not ($l starts-with "<" or $l starts-with "Another Claude session sent a message") }
            | first
          } catch { null }
          let title = $titles | where type == "custom-title" | get -o customTitle | last
            | default ($titles | where type == "ai-title" | get -o aiTitle | last)
            | default $prompt
            | default "spare"
          { name: (shpool-session-name $s.pid), cname: ($s.name? | default ""), title: $title, status: $s.status, cwd: $s.cwd }
        }
      } | compact
      $sessions | each {|s|
        let claude = $claudes | where name == $s.name | get -o 0
        let fg = $shells | where name == $s.name | get -o 0.fg
        let cwd = if $claude != null { $claude.cwd } else if $fg != null { try { $"/proc/($fg)/cwd" | path expand } } else { null }
        let cmd = if $fg != null { open --raw $"/proc/($fg)/cmdline" | split row (char nul) | compact --empty | each { path basename } | str join " " } else { "" }
        let title = if $claude != null {
          $"($claude.title) \(($claude.status)\)"
        } else if ($cmd | is-empty) or ($cmd starts-with "-") { $s.name } else { $"($s.name): ($cmd)" }
        let ids = if $claude == null { "" } else {
          [$s.name $claude.cname] | where {|n| ($n | is-not-empty) and $n != $claude.title } | uniq | str join " "
        }
        let last = [$s.started_at_unix_ms $s.last_connected_at_unix_ms $s.last_disconnected_at_unix_ms] | compact | math max
        {
          name: $s.name
          status: ($s.status | str lowercase)
          last: $last
          ago: ($last * 1_000_000 | into datetime | date humanize)
          dir: ($cwd | default "")
          title: $title
          ids: $ids
        }
      } | sort-by last --reverse
    }

    def shpool-pick [] {
      let sessions = shpool-sessions
      let new = "new session"
      let kill = "kill sessions…"
      let lines = if ($sessions | is-empty) { [] } else {
        let title_w = [($sessions.title | str length | math max) 40] | math min
        let ids_w = $sessions.ids | str length | math max
        let cols = (term size).columns - 4
        $sessions | each {|s|
          [
            ($s.title | str substring ..<$title_w | fill -w $title_w)
            ($s.ids | fill -w $ids_w)
            ($s.ago | str replace " ago" "" | fill -w 14)
            ($s.status | str replace "disconnected" "detached" | fill -w 8)
            ($s.dir | path basename)
          ] | str join "  " | str substring ..<$cols
        }
      }
      let n = $sessions | length
      let choice = if ($lines | is-empty) { $n } else {
        $lines | append [$new $kill] | input list --fuzzy --index "shpool session (type to filter, esc for a plain shell)"
      }
      if $choice == null {
      } else if $choice < $n {
        exec shpool attach --force ($sessions | get $choice | get name)
      } else if $choice > $n {
        $lines
        | input list --fuzzy --multi --index "kill which sessions (tab marks, enter kills, esc cancels)"
        | default []
        | each {|i| shpool kill ($sessions | get $i | get name) }
        | ignore
        shpool-pick
      } else {
        let taken = $sessions.name
        let default = 1..(($taken | length) + 1) | each {|i| $"s($i)" } | where {|n| $n not-in $taken } | first
        let name = input $"session name \(enter for ($default)\): " | str trim | str replace --all --regex '\s+' '-'
        exec shpool attach (if ($name | is-empty) { $default } else { $name })
      }
    }

    if $nu.is-interactive and ($env.SSH_CONNECTION? != null) and ($env.SHPOOL_SESSION_NAME? == null) {
      shpool-pick
    }
  '';
}
