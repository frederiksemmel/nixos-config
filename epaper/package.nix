{
  lib,
  pkgs,
  crane,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.toml
        ./Cargo.lock
        ./assets
        ./src
      ];
    };
    strictDeps = true;
  };
in
craneLib.buildPackage (
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

    meta = {
      description = "Draw on the Waveshare 2.7\" e-Paper HAT (B), by hand or through a Telegram bot";
      mainProgram = "epaper";
      platforms = lib.platforms.linux;
    };
  }
)
