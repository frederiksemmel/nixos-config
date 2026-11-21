{
  lib,
  pkgs,
  crane,
  dejavu_fonts,
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
    EPAPER_FONT_DIR = "${dejavu_fonts}/share/fonts/truetype";
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
