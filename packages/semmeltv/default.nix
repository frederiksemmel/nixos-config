{
  lib,
  rustPlatform,
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

rustPlatform.buildRustPackage {
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

  cargoLock.lockFile = ./Cargo.lock;

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

  dontPatchELF = true;

  postInstall = ''
    wrapProgram $out/bin/semmeltv --suffix PATH : ${
      lib.makeBinPath [
        mpv
        wlr-randr
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
