{
  lib,
  runCommand,
  fetchurl,
  imagemagick,
}:

let
  # nixpkgs' rerun installs only bin/rerun and lib/librerun_c.a — upstream ships
  # no Linux desktop entry or hicolor icon, because their app icon only ever
  # reaches eframe's NativeOptions.icon_data, which is still unimplemented on
  # Wayland (rerun-io/rerun#1108). Launchers resolve `Icon=rerun` through the
  # icon theme instead, so install the source asset there ourselves.
  #
  # Pinned to a fixed tag rather than rerun.version: the icon has been stable
  # across releases, and tracking the version would break this hash on every
  # rerun bump.
  appIcon = fetchurl {
    url = "https://raw.githubusercontent.com/rerun-io/rerun/0.34.1/crates/viewer/re_viewer/data/app_icon.png";
    hash = "sha256-RrNkMyUTEMoS6hpSvvbcoMV140ks1N/S23IoVZZFTk8=";
  };

  # The asset is 512x512, and hicolor declares 512x512/apps with MinSize=64, so
  # a launcher asking for 48px only finds it via the closest-size fallback pass.
  # Downscaling into every common bucket keeps it on the exact-match pass.
  sizes = [
    48
    64
    128
    256
    512
  ];

  installSize =
    size:
    let
      dir = "$out/share/icons/hicolor/${toString size}x${toString size}/apps";
    in
    ''
      mkdir -p ${dir}
      magick ${appIcon} -resize ${toString size}x${toString size} ${dir}/rerun.png
    '';
in
runCommand "rerun-icon" {
  nativeBuildInputs = [ imagemagick ];

  meta = {
    description = "hicolor application icon for rerun, which upstream does not ship on Linux";
    homepage = "https://github.com/rerun-io/rerun";
    license = lib.licenses.asl20;
    platforms = lib.platforms.all;
  };
} (lib.concatMapStringsSep "\n" installSize sizes)
