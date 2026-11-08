{
  stdenvNoCC,
  python313,
  pyhailort,
}:

let
  python = python313.withPackages (ps: [
    pyhailort
    ps.faster-whisper
    ps.numpy
    ps.onnxruntime
    ps.tokenizers
    ps.wyoming
  ]);
in
stdenvNoCC.mkDerivation {
  pname = "hailo-whisper";
  version = "0.1.0";

  src = ./hailo_whisper.py;
  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    { echo "#!${python.interpreter}"; cat $src; } > $out/bin/hailo-whisper
    chmod +x $out/bin/hailo-whisper
    runHook postInstall
  '';

  meta = {
    description = "Wyoming speech-to-text server running Whisper's encoder on a Hailo-8L";
    mainProgram = "hailo-whisper";
  };
}
