{ nixpkgs }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  newer = nixpkgs.legacyPackages.${pkgs.stdenv.hostPlatform.system};
in
{
  config = lib.mkIf config.services.rqbit.enable {
    services.rqbit.package = newer.rqbit.overrideAttrs (old: {
      cargoBuildFeatures = (old.cargoBuildFeatures or [ ]) ++ [ "disable-upload" ];
      cargoCheckFeatures = (old.cargoCheckFeatures or [ ]) ++ [ "disable-upload" ];
      preBuild = (old.preBuild or "") + ''
        export AWS_LC_SYS_USE_SYSTEM=0
      '';
    });

    systemd.services.rqbit.environment = {
      RQBIT_DISABLE_UPLOAD = "true";
      RQBIT_LSD_DISABLE = "true";
      RQBIT_UPNP_PORT_FORWARD_DISABLE = "true";
    };
  };
}
