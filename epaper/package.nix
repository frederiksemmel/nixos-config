{
  lib,
  pkgs,
  crane,
  symlinkJoin,
  dejavu_fonts,
  noto-fonts-monochrome-emoji,
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
    EPAPER_FONT_DIR = "${
      symlinkJoin {
        name = "epaper-fonts";
        paths = [
          dejavu_fonts
          noto-fonts-monochrome-emoji
        ];
      }
    }/share/fonts";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    doCheck = false;

    passthru.tests.cargo = craneLib.cargoTest (commonArgs // { inherit cargoArtifacts; });

    meta = {
      description = "Draw on the Waveshare 2.7\" e-Paper HAT (B), by hand or through a Telegram bot";
      mainProgram = "epaper";
      platforms = lib.platforms.linux;
    };
  }
)
