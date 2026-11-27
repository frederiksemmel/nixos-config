{
  config,
  lib,
  pkgs,
  ...
}:
let
  imv = lib.getExe config.programs.imv.package;
  mpv = lib.getExe config.programs.mpv.finalPackage;

  # fstux's login shell is nushell, so the paths reach its open-from-beast as nuon.
  openOnFstux = pkgs.writers.writeNuBin "open-on-fstux" ''
    def main [mode: string, ...files: path] {
      let files = $files | path expand
      if "WAYLAND_DISPLAY" in $env {
        match $mode {
          image => { ^${imv} -- ...$files }
          video => { ^${mpv} -- ...$files }
        }
        return
      }
      let fstux = try {
        ^${pkgs.getent}/bin/getent ahostsv4 fstux | lines | first | split row --regex '\s+' | first
      } catch { "" }
      let connected = $fstux != "" and (
        ^${pkgs.iproute2}/bin/ss -Htn state established $"sport = :22 and dst ($fstux)" | str trim | is-not-empty
      )
      if not $connected {
        error make --unspanned { msg: "beast has no display, and fstux is not connected over ssh" }
      }
      ^${pkgs.openssh}/bin/ssh -o ConnectTimeout=5 fstux $"open-from-beast ($mode | to nuon) ...($files | to nuon)"
    }
  '';
in
{
  imports = [
    ./beast-disk-cleanup
    ./beast-claude.nix
    ./beast-semmeltv
    ../../feedback/hub
    ../../courts/beast.nix
    ../../ramura-site/beast.nix
    ./beast-vm-tests.nix
  ];

  profiles.bitwarden.headlessSshKeys.enable = true;

  services.feedback-hub.enable = true;

  programs.nushell.extraEnv = ''
    $env.SSH_AUTH_SOCK = $env.SSH_AUTH_SOCK? | default $"($env.XDG_RUNTIME_DIR)/gcr/ssh"
  '';

  programs.ssh.settings.fstux = {
    IdentityFile = "~/.ssh/id_fstux";
    IdentitiesOnly = true;
  };
  programs.ssh.settings."office-windows".IdentityFile = lib.mkForce "~/.ssh/id_fstux";

  # yazi on beast shows images and videos on fstux's screen while beast has no
  # display of its own and fstux is connected over ssh.
  programs.yazi.settings = {
    opener = lib.genAttrs [ "image" "video" ] (mode: [
      {
        run = "${lib.getExe openOnFstux} ${mode} %s";
        desc = "${if mode == "image" then "imv" else "mpv"}, on fstux over ssh";
      }
    ]);
    open.prepend_rules = [
      {
        mime = "image/*";
        use = [
          "image"
          "reveal"
        ];
      }
      {
        mime = "video/*";
        use = [
          "video"
          "reveal"
        ];
      }
    ];
  };

  nix.package = pkgs.nix;
  nix.settings = {
    experimental-features = [
      "nix-command"
      "flakes"
    ];
    accept-flake-config = true;
    builders = "ssh://eu.nixbuild.net aarch64-linux - 100 4 benchmark,big-parallel";
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
