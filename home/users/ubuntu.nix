{
  flake,
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

  home.packages = [
    flake.inputs.nix-gl-host.defaultPackage.x86_64-linux
  ];

  programs.git.settings.user = {
    name = "Frederik Semmel";
    email = "frederiksemmel@gmail.com";
  };
}
