{
  lib,
  stdenvNoCC,
  fetchzip,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "nordwand-mono";
  version = "1.3.1";

  src = fetchzip {
    url = "https://github.com/tywr/Nordwand-Mono/releases/download/${finalAttrs.version}/NordwandMono-TTF.zip";
    hash = "sha256-1gxHsMedpzsPxjRYFsTjaVOSgVFxD3VNtC3WQR9j9uI=";
    stripRoot = false;
  };

  installPhase = ''
    runHook preInstall
    install -Dm644 -t $out/share/fonts/truetype *.ttf
    runHook postInstall
  '';

  meta = {
    description = "Compact monospace neo-grotesque font designed for developers";
    homepage = "https://github.com/tywr/Nordwand-Mono";
    license = lib.licenses.ofl;
    platforms = lib.platforms.all;
  };
})
