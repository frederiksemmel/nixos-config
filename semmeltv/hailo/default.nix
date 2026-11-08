{
  lib,
  callPackage,
  python313Packages,
  writeTextDir,
}:

lib.mapAttrs (
  name: spec:
  let
    generation = spec // {
      inherit name;
    };
  in
  rec {
    hailort = callPackage ./hailort.nix { inherit generation; };
    firmware = callPackage ./firmware.nix { inherit generation; };
    gst-hailo = callPackage ./gst-hailo.nix { inherit hailort; };
    pyhailort = python313Packages.callPackage ./pyhailort.nix { inherit generation hailort; };
    udevRules = writeTextDir "lib/udev/rules.d/51-hailo-udev.rules" ''
      SUBSYSTEM=="${generation.deviceClass}", MODE="0666"
    '';
    driverFor = linuxPackages: linuxPackages.callPackage ./driver.nix { inherit generation; };
  }
) (import ./generations.nix)
