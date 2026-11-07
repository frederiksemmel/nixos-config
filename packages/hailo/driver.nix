{
  lib,
  stdenv,
  fetchFromGitHub,
  kernel,
  kernelModuleMakeFlags,
  generation,
}:

stdenv.mkDerivation {
  pname = "${generation.kernelModule}-${kernel.version}";
  inherit (generation) version;

  src = fetchFromGitHub {
    owner = "hailo-ai";
    repo = "hailort-drivers";
    rev = "v${generation.version}";
    hash = generation.driverHash;
  };

  nativeBuildInputs = kernel.moduleBuildDependencies;
  hardeningDisable = [ "pic" ];
  enableParallelBuilding = true;

  makeFlags =
    kernelModuleMakeFlags
    ++ generation.driverMakeFlags
    ++ [ "-C${kernel.dev}/lib/modules/${kernel.modDirVersion}/build" ];

  preBuild = ''
    makeFlagsArray+=(M=$PWD/${generation.driverDir})
  '';
  buildFlags = [ "modules" ];

  installPhase = ''
    runHook preInstall
    install -Dm444 -t $out/lib/modules/${kernel.modDirVersion}/extra \
      ${generation.driverDir}/${generation.kernelModule}.ko
    runHook postInstall
  '';

  meta = {
    description = "Linux PCIe driver for Hailo AI accelerators (${generation.kernelModule})";
    homepage = "https://github.com/hailo-ai/hailort-drivers";
    license = lib.licenses.gpl2Only;
    platforms = lib.platforms.linux;
  };
}
