{
  lib,
  pkgs,
  crane,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    pname = "semmeltv-front";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ../.;
      fileset = lib.fileset.unions [
        ../assets/logo/favicon.svg
        ../assets/logo/semmeltv-dark.svg
        ./Cargo.lock
        ./Cargo.toml
        ./src
      ];
    };
    cargoToml = ./Cargo.toml;
    cargoLock = ./Cargo.lock;
    postUnpack = ''
      cd $sourceRoot/front
      sourceRoot=.
    '';

    strictDeps = true;
  };
in
craneLib.buildPackage (
  commonArgs
  // {
    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

    postInstall = ''
      ln -s semmeltv-front $out/bin/semmeltv-maintenance
    '';

    meta = {
      description = "SemmelTV's front: the proxy before the core and the web app, and the bakery's closed screen";
      license = lib.licenses.gpl3Only;
      mainProgram = "semmeltv-front";
      platforms = lib.platforms.linux;
    };
  }
)
