{
  lib,
  pkgs,
  crane,
  makeBinaryWrapper,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    pname = "semmeltv-web";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        (lib.fileset.fileFilter (
          file: file.hasExt "svg" || file.hasExt "png" && file.name != "semmel-source.png"
        ) ../assets/logo)
        ./Cargo.lock
        ./Cargo.toml
        ./src
      ];
    };
    cargoToml = ./Cargo.toml;
    cargoLock = ./Cargo.lock;
    postUnpack = ''
      cd $sourceRoot/web
      sourceRoot=.
    '';

    strictDeps = true;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    nativeBuildInputs = [ makeBinaryWrapper ];

    postBuild = ''
      target/release/semmeltv-web bundle assets
    '';

    postInstall = ''
      mkdir -p $out/share/semmeltv-web
      cp -r assets $out/share/semmeltv-web/assets
      wrapProgram $out/bin/semmeltv-web \
        --set-default SEMMELTV_WEB_ASSETS $out/share/semmeltv-web/assets
    '';

    meta = {
      description = "Web app for SemmelTV, built on the TV's HTTP API";
      license = lib.licenses.gpl3Only;
      mainProgram = "semmeltv-web";
      platforms = lib.platforms.linux;
    };
  }
)
