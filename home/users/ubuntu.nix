{
  config,
  ...
}:
{
  imports = [
    ../modules/dev.nix
  ];

  profiles.dev.enable = true;

  home.username = "ubuntu";
  home.homeDirectory = "/home/${config.home.username}";
  home.language.base = "en_US.UTF-8";

  programs.git.settings.user = {
    name = "Frederik Semmel";
    email = "frederiksemmel@gmail.com";
  };
}
