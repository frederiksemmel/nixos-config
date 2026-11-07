{
  lib,
  stdenvNoCC,
  fetchurl,
  generation,
}:

stdenvNoCC.mkDerivation {
  pname = "${generation.name}-firmware";
  inherit (generation) version;

  src = fetchurl {
    url = "https://hailo-hailort.s3.eu-west-2.amazonaws.com/${generation.firmware.path}";
    inherit (generation.firmware) hash;
  };

  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/firmware/hailo
    if [[ $src == *.tar.gz ]]; then
      mkdir $out/lib/firmware/hailo/${generation.name}
      tar -xzf $src -C $out/lib/firmware/hailo/${generation.name}
    else
      install -Dm444 $src $out/lib/firmware/hailo/${generation.name}_fw.bin
    fi
    runHook postInstall
  '';

  passthru.compressFirmware = false;

  meta = {
    description = "Firmware for ${generation.name} AI accelerators";
    homepage = "https://hailo.ai/";
    license = {
      fullName = "Hailo firmware licence";
      url = generation.firmware.licenseUrl;
      free = false;
      redistributable = false;
    };
    sourceProvenance = [ lib.sourceTypes.binaryFirmware ];
    platforms = lib.platforms.linux;
  };
}
