{
  lib,
  buildGoModule,
}:

buildGoModule {
  pname = "tvcast";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./go.mod
      ./go.sum
      ./main.go
      ./tvcast.lua
    ];
  };

  vendorHash = "sha256-RMrHffd4BxALos8BWy4aF1PAbdoFUSgc6ogD9hVfwHw=";

  env.CGO_ENABLED = 0;

  postInstall = ''
    install -D tvcast.lua $out/share/mpv/scripts/tvcast.lua
    substituteInPlace $out/share/mpv/scripts/tvcast.lua --replace-fail @tvcast@ $out/bin/tvcast
  '';

  passthru.scriptName = "tvcast.lua";

  meta = {
    description = "Cast local files to DLNA TVs, with an mpv script that drives it";
    license = lib.licenses.mit;
    mainProgram = "tvcast";
  };
}
