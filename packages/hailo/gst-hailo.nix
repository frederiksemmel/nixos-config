{
  lib,
  stdenv,
  cmake,
  pkg-config,
  glib,
  gst_all_1,
  hailort,
}:

stdenv.mkDerivation {
  pname = "gst-hailo";
  inherit (hailort) version src;

  sourceRoot = "source/hailort/libhailort/bindings/gstreamer";

  postPatch = ''
    substituteInPlace CMakeLists.txt \
      --replace-fail '/''${CMAKE_SYSTEM_PROCESSOR}-linux-gnu/gstreamer-1.0/' '/gstreamer-1.0/'
  '';

  nativeBuildInputs = [
    cmake
    pkg-config
  ];

  buildInputs = [
    glib
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    hailort
  ];

  meta = {
    description = "GStreamer elements for Hailo AI accelerators";
    homepage = "https://github.com/hailo-ai/hailort";
    license = lib.licenses.lgpl21Plus;
    platforms = lib.platforms.linux;
  };
}
