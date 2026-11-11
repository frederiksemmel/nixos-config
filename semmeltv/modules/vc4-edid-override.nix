{
  lib,
  stdenv,
  kernel,
  kernelModuleMakeFlags,
}:

stdenv.mkDerivation {
  pname = "vc4-edid-override";
  inherit (kernel) version;

  src = kernel.src;
  unpackPhase = ''
    runHook preUnpack
    mkdir -p tree/include/trace tree/drivers/gpu/drm
    cp -r $src/drivers/gpu/drm/vc4 tree/drivers/gpu/drm/vc4
    chmod -R u+w tree
    cd tree/drivers/gpu/drm/vc4
    runHook postUnpack
  '';
  patches = [ ./vc4-hdmi-edid-override.patch ];
  patchFlags = [ "-p5" ];

  nativeBuildInputs = kernel.moduleBuildDependencies;
  hardeningDisable = [ "pic" ];
  enableParallelBuilding = true;

  makeFlags = kernelModuleMakeFlags ++ [
    "-C${kernel.dev}/lib/modules/${kernel.modDirVersion}/build"
  ];
  preBuild = ''
    makeFlagsArray+=(M=$PWD "KCFLAGS=-I$(realpath ../../../../include/trace)")
  '';
  buildFlags = [ "modules" ];

  installPhase = ''
    runHook preInstall
    install -Dm444 vc4.ko $out/lib/modules/${kernel.modDirVersion}/updates/vc4.ko
    runHook postInstall
  '';

  meta = {
    description = "vc4 with HDMI-CEC taken from the EDID override when the TV is silent on DDC";
    license = lib.licenses.gpl2Only;
    platforms = lib.platforms.linux;
  };
}
