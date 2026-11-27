{
  lib,
  pkgs,
  crane,
  makeBinaryWrapper,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    pname = "courts";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ./Cargo.lock
        ./Cargo.toml
        ./src
        ./toasty
        ../feedback/client
      ];
    };
    cargoToml = ./Cargo.toml;
    cargoLock = ./Cargo.lock;
    postUnpack = ''
      cd $sourceRoot/courts
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
      target/release/courts bundle assets
    '';

    postInstall = ''
      mkdir -p $out/share/courts
      cp -r assets $out/share/courts/assets
      wrapProgram $out/bin/courts \
        --set-default COURTS_ASSETS $out/share/courts/assets
    '';

    meta = {
      description = "Free tennis courts in Zurich from GotCourts, with watches that notify";
      license = lib.licenses.gpl3Only;
      mainProgram = "courts";
      platforms = lib.platforms.linux;
    };
  }
)
