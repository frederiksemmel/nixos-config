{
  lib,
  pkgs,
  crane,
  symlinkJoin,
  dejavu_fonts,
  noto-fonts-monochrome-emoji,
  ffmpeg-headless,
  makeWrapper,
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
    nativeBuildInputs = [ makeWrapper ];
    postInstall = ''
      wrapProgram $out/bin/epaper --prefix PATH : ${lib.makeBinPath [ ffmpeg-headless ]}
    '';

    passthru.tests.cargo = craneLib.cargoTest (
      commonArgs
      // {
        inherit cargoArtifacts;
        nativeBuildInputs = [ ffmpeg-headless ];
      }
    );

    meta = {
      description = "Draw on the Waveshare 2.7\" e-Paper HAT (B), by hand or through a Telegram bot";
      mainProgram = "epaper";
      platforms = lib.platforms.linux;
    };
  }
)
