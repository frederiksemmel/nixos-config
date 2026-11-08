{
  lib,
  inter,
  fontconfig,
  libGL,
  libxkbcommon,
  wayland,
  ...
}:
{
  SEMMELTV_FONT_DIR = "${inter}/share/fonts/truetype";
  RUSTFLAGS = "-C link-arg=-Wl,-rpath,${
    lib.makeLibraryPath [
      fontconfig
      libGL
      libxkbcommon
      wayland
    ]
  }";
}
