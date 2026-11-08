{
  stdenvNoCC,
  python3,
}:

let
  python = python3.withPackages (ps: [
    ps.numpy
    ps.sherpa-onnx
    ps.wyoming
  ]);
in
stdenvNoCC.mkDerivation {
  pname = "parakeet-wyoming";
  version = "0.1.0";

  src = ./parakeet.py;
  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/bin
    { echo "#!${python.interpreter}"; cat $src; } > $out/bin/parakeet-wyoming
    chmod +x $out/bin/parakeet-wyoming
    runHook postInstall
  '';

  meta = {
    description = "Wyoming speech-to-text server running NVIDIA Parakeet on the CPU";
    mainProgram = "parakeet-wyoming";
  };
}
