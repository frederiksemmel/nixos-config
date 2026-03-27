{
  flake,
  config,
  ...
}:
{
  imports = with flake.inputs; [
    zen-browser.homeModules.beta
    centerpiece.hmModules."x86_64-linux".default
    agenix.homeManagerModules.default
    ./ssh_config.nix
    ./modules/dev.nix
    ./modules/desktop.nix
  ];

  home.username = "frederik";
  home.homeDirectory = "/home/${config.home.username}";
  home.language.base = "en_US.UTF-8";

  age.secrets = {
    spadesvision_gdrive_token.file = ../secrets/spadesvision_gdrive_token.age;
    spadesvision_gdrive_client_secret.file = ../secrets/spadesvision_gdrive_client_secret.age;
  };
  age.identityPaths = [ "${config.home.homeDirectory}/.ssh/id_github" ];
}
