let
  locked = (builtins.fromJSON (builtins.readFile ../../../flake.lock)).nodes.nixpkgs.locked;
  pkgs = import (builtins.fetchTarball {
    inherit (locked) url;
    sha256 = locked.narHash;
  }) { };
in
pkgs.mkShell {
  packages = [
    (pkgs.python3.withPackages (ps: [
      ps.aiohttp
      ps.playwright
    ]))
    pkgs.ffmpeg-headless
  ];
  PLAYWRIGHT_BROWSERS_PATH = pkgs.playwright-driver.browsers;
  PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
  LD_LIBRARY_PATH = "${pkgs.ffmpeg.lib}/lib";
}
