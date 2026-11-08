{
  stdenvNoCC,
  python3,
}:

let
  python = python3.withPackages (ps: [ ps.aiohttp ]);
in
stdenvNoCC.mkDerivation {
  pname = "phone-mic";
  version = "0.1.0";

  src = ./phone_mic.py;
  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    { echo "#!${python.interpreter}"; cat $src; } > $out/bin/phone-mic
    chmod +x $out/bin/phone-mic
    runHook postInstall
  '';

  meta = {
    description = "Web page that streams a phone's microphone into an ALSA loopback";
    mainProgram = "phone-mic";
  };
}
