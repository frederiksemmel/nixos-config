{
  lib,
  pkgs,
  ...
}:
{
  imports = [
    ./beast-disk-cleanup
    ./beast-claude.nix
  ];

  profiles.bitwarden.headlessSshKeys.enable = true;

  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  programs.ssh.settings.fstux = {
    IdentityFile = "~/.ssh/id_fstux";
    IdentitiesOnly = true;
  };
  programs.ssh.settings."office-windows".IdentityFile = lib.mkForce "~/.ssh/id_fstux";

  nix.package = pkgs.nix;
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    accept-flake-config = true;
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

  programs.nh.clean = {
    enable = lib.mkForce true;
    extraArgs = lib.mkForce "--keep 5";
  };

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
  # The default (stop) SIGKILLs every session when the kernel OOM-kills one process in them.
  systemd.user.services.shpool.Service.OOMPolicy = "continue";
}
