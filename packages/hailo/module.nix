{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.hardware.hailo;
  hailo = (pkgs.callPackage ./. { }).${cfg.generation};
in
{
  options.hardware.hailo = {
    enable = lib.mkEnableOption "the Hailo AI accelerator driver, firmware and runtime";

    generation = lib.mkOption {
      type = lib.types.enum [
        "hailo8"
        "hailo10h"
      ];
      example = "hailo10h";
      description = ''
        Accelerator generation. `hailo8` covers the Hailo-8 and Hailo-8L of the
        AI Kit and the 13/26 TOPS AI HAT+. `hailo10h` covers the Hailo-10H of the
        40 TOPS AI HAT+ 2.
      '';
    };

    packages = lib.mkOption {
      type = lib.types.raw;
      readOnly = true;
      default = hailo;
      defaultText = lib.literalMD "the package set of the selected generation";
      description = ''
        Packages of the selected generation, for use by services that need
        `pyhailort` or `gst-hailo` at the release matching the driver.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    boot.extraModulePackages = [ (hailo.driverFor config.boot.kernelPackages) ];
    hardware.firmware = [ hailo.firmware ];
    services.udev.packages = [ hailo.udevRules ];
    environment.systemPackages = [ hailo.hailort ];
  };
}
