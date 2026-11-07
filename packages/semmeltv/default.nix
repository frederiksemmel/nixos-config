{
  lib,
  pkgs,
  crane,
  makeBinaryWrapper,
  pkg-config,
  inter,
  fontconfig,
  libGL,
  libxkbcommon,
  wayland,
  mpv,
  wlr-randr,
}:

let
  craneLib = crane.mkLib pkgs;

  commonArgs = {
    pname = "semmeltv";
    version = "0.1.0";

    src = lib.fileset.toSource {
      root = ./.;
      fileset = lib.fileset.unions [
        ./Cargo.lock
        ./Cargo.toml
        ./build.rs
        ./src
        ./ui
      ];
    };

    strictDeps = true;

    nativeBuildInputs = [
      makeBinaryWrapper
      pkg-config
    ];
    buildInputs = [ fontconfig ];

    env = {
      SEMMELTV_FONT_DIR = "${inter}/share/fonts/truetype";
      RUSTFLAGS = "-C link-arg=-Wl,-rpath,${
        lib.makeLibraryPath [
          fontconfig
          libGL
          libxkbcommon
          wayland
        ]
      }";
    };
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
        ]
      }
    '';

    passthru.devShell = craneLib.devShell {
      inherit (commonArgs) buildInputs env;
      packages = [ pkg-config ];
    };

    meta = {
      description = "Apple TV style interface for a personal video library";
      license = lib.licenses.gpl3Only;
      mainProgram = "semmeltv";
      platforms = lib.platforms.linux;
    };
  }
)
