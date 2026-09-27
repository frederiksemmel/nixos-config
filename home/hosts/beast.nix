{
  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

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

  programs.nushell.extraConfig = ''
    if $nu.is-interactive and ($env.SSH_CONNECTION? != null) and ($env.SHPOOL_SESSION_NAME? == null) {
      let sessions = shpool list --json | from json | get sessions | sort-by last_connected_at_unix_ms --reverse
      let new = "new session"
      let choice = if ($sessions | is-empty) { $new } else {
        $sessions
        | each {|s| $"($s.name) \(($s.status | str lowercase)\)" }
        | append $new
        | input list "shpool session (esc for a plain shell)"
      }
      if $choice == $new {
        let taken = $sessions.name
        let name = 1..(($taken | length) + 1) | each {|i| $"s($i)" } | where {|n| $n not-in $taken } | first
        exec shpool attach $name
      } else if $choice != null {
        exec shpool attach --force ($choice | split row " " | first)
      }
    }
  '';
}
