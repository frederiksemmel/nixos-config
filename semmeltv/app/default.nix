{
  lib,
  pkgs,
  crane,
  makeBinaryWrapper,
  pkg-config,
  fontconfig,
  mpv,
  wlr-randr,
  alsa-utils,
  bluez,
  bluez-alsa,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    pname = "semmeltv";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        (lib.fileset.fileFilter (
          file: file.hasExt "svg" || file.hasExt "png" && file.name != "semmel-source.png"
        ) ../assets/logo)
        ./Cargo.lock
        ./Cargo.toml
        ./build.rs
        ./src
        ./ui
      ];
    };
    cargoToml = ./Cargo.toml;
    cargoLock = ./Cargo.lock;
    postUnpack = ''
      cd $sourceRoot/app
      sourceRoot=.
    '';

    strictDeps = true;

    nativeBuildInputs = [
      makeBinaryWrapper
      pkg-config
    ];
    buildInputs = [ fontconfig ];

    env = import ./build-env.nix pkgs;
  };

  cargoArtifacts = craneLib.buildDepsOnly (commonArgs // { cargoCheckCommand = "true"; });
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    dontPatchELF = true;

    postInstall = ''
      wrapProgram $out/bin/semmeltv --suffix PATH : ${
        lib.makeBinPath [
          mpv
          wlr-randr
          alsa-utils
          bluez
          bluez-alsa
        ]
      }
    '';

    meta = {
      description = "Apple TV style interface for a personal video library";
      license = lib.licenses.gpl3Only;
      mainProgram = "semmeltv";
      platforms = lib.platforms.linux;
    };
  }
)
