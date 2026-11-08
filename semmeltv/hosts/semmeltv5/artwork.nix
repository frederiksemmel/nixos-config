{ config, flake, ... }:
{
  imports = [ flake.inputs.agenix.nixosModules.default ];

  age.secrets.tmdb_token = {
    file = ../../../secrets/tmdb_token.age;
    owner = "semmeltv";
  };

  services.semmeltv.artwork = {
    enable = true;
    tmdbTokenFile = config.age.secrets.tmdb_token.path;
  };

  systemd.services.semmeltv-artwork.after = [ "semmeltv-mount.service" ];
}
