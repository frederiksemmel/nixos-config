{
  flake,
  config,
  ...
}:
{
  imports = with flake.inputs; [
    agenix.homeManagerModules.default
    ../modules/theme.nix
    ../modules/dev.nix
    ../modules/desktop.nix
    ../modules/cloud.nix
    ../modules/ssh.nix
    ../modules/bitwarden.nix
    ../modules/beast-sessions.nix
  ];

  # Building the option docs leaks nixpkgs' store path into options.json without
  # string context, which Determinate Nix warns about on every rebuild.
  # https://github.com/nix-community/home-manager/issues/7935
  manual.manpages.enable = false;

  profiles.dev.enable = true;
  profiles.desktop.enable = true;
  profiles.cloud.enable = true;
  profiles.ssh.enable = true;
  profiles.bitwarden.enable = true;

  home.username = "frederik";
  home.homeDirectory = "/home/${config.home.username}";
  home.language.base = "en_US.UTF-8";

  programs.git.settings.user = {
    name = "Frederik Semmel";
    email = "frederiksemmel@gmail.com";
  };

  age.identityPaths = [ "${config.home.homeDirectory}/.ssh/id_github" ];
}
