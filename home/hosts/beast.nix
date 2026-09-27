{
  config,
  lib,
  pkgs,
  ...
}:
let
  remoteControl = repo: {
    Unit = {
      Description = "Claude Code Remote Control in ~/repos/${repo}";
      X-SwitchMethod = "keep-old";
    };
    Service = {
      WorkingDirectory = "%h/repos/${repo}";
      ExecStart = "${lib.getExe config.programs.claude-code.finalPackage} remote-control --spawn=worktree --no-create-session-in-dir";
      StandardOutput = "null";
      Restart = "always";
      RestartSec = 10;
    };
    Install.WantedBy = [ "default.target" ];
  };

  # Each session's own worktree's devenv shell, snapshotted once; Claude sources
  # CLAUDE_ENV_FILE before every Bash call.
  devenvEnv = pkgs.writeShellApplication {
    name = "claude-devenv-env";
    runtimeInputs = [ pkgs.devenv ];
    text = ''
      cd "$CLAUDE_PROJECT_DIR"
      [ -f devenv.nix ] || exit 0
      devenv shell -- bash -c 'export -p' \
        | grep '^declare -x ' \
        | grep -Ev '^declare -x (PWD|OLDPWD|SHLVL|_)=' >> "$CLAUDE_ENV_FILE"
    '';
  };
  repos = [
    "blackjackeye"
    "nixos-config"
  ];
in
{
  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  systemd.user.services = lib.listToAttrs (
    map (repo: lib.nameValuePair "claude-remote-control-${repo}" (remoteControl repo)) repos
  );

  # remote-control refuses an untrusted workspace, and the trust flag lives in the
  # CLI-written ~/.claude.json, so it is set in place rather than declared.
  home.activation.claudeTrustRepos = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    f="$HOME/.claude.json"
    if [ -f "$f" ]; then
      tmp=$(mktemp)
      ${lib.getExe pkgs.jq} '${
        lib.concatMapStrings (
          repo: ''.projects["${config.home.homeDirectory}/repos/${repo}"].hasTrustDialogAccepted = true | ''
        ) repos
      }.' "$f" > "$tmp"
      run cp "$tmp" "$f"
      rm "$tmp"
    fi
  '';

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
}
