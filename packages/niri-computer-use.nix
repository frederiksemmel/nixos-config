{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libxkbcommon,
  grim,
  systemd,
  libnotify,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "niri-computer-use";
  version = "0.1.8";

  src = fetchFromGitHub {
    owner = "ayagmar";
    repo = "niri-computer-use";
    tag = "v${finalAttrs.version}";
    hash = "sha256-yHjibfv/S2JzwTt+WsJo2SMJgTRAxtFwx5U7tY+FE0I=";
  };

  cargoHash = "sha256-hjwiofNUn+z5gk3/KkL8zDRtJVw2SwxBb1DcLVzJSzg=";

  cargoBuildFlags = [
    "--package"
    "niri-computer-use"
  ];

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];
  buildInputs = [ libxkbcommon ];

  doCheck = false;

  postFixup = ''
    wrapProgram $out/bin/niri-computer-use --suffix PATH : ${
      lib.makeBinPath [
        grim
        systemd
        libnotify
      ]
    }
  '';

  meta = {
    description = "MCP server for AI agents to observe and drive a niri Wayland desktop";
    homepage = "https://github.com/ayagmar/niri-computer-use";
    license = lib.licenses.mit;
    mainProgram = "niri-computer-use";
    platforms = lib.platforms.linux;
  };
})
