{ config, pkgs, ... }:
let
  python3Packages = pkgs.python3Packages.overrideScope (
    final: prev: {
      pyopen-wakeword = prev.pyopen-wakeword.overridePythonAttrs (old: {
        doCheck = false;
        meta = old.meta // {
          broken = false;
        };
      });
    }
  );
in
{
  services.cage.environment.SEMMELTV_WAKE = config.services.wyoming.openwakeword.uri;

  services.wyoming.openwakeword = {
    enable = true;
    package = pkgs.wyoming-openwakeword.override { inherit python3Packages; };
    uri = "tcp://127.0.0.1:10400";
    customModelsDirectories = [
      (builtins.path {
        name = "semmeltv-wake-models";
        path = ../../wake/models;
      })
    ];
    threshold = 0.9;
  };
}
