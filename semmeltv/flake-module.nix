{
  inputs,
  lib,
  self,
  flake-parts-lib,
  ...
}:
let
  hosts = [
    "semmeltv3"
    "semmeltv5"
  ];
in
{
  flake = {
    nixosConfigurations = lib.genAttrs hosts (
      name:
      inputs.nixos-raspberrypi.lib.nixosSystem {
        specialArgs = {
          flake = { inherit self inputs; };
          inherit (inputs) nixos-raspberrypi;
        };
        modules = [ ./hosts/${name} ];
      }
    );

    deploy.nodes = lib.genAttrs hosts (name: {
      hostname = name;
      sshUser = "frederik";
      user = "root";
      profiles.system.path =
        inputs.deploy-rs.lib.aarch64-linux.activate.nixos
          self.nixosConfigurations.${name};
    });

    nixosModules = {
      semmeltv = flake-parts-lib.importApply ./modules/kiosk.nix {
        inherit (inputs) crane nixpkgs;
        vpnConfinement = inputs.vpn-confinement.nixosModules.default;
        agenix = inputs.agenix.nixosModules.default;
      };
      hailo = ./hailo/module.nix;
    };
  };

  perSystem =
    { pkgs, system, ... }:
    {
      packages = {
        semmeltv = pkgs.callPackage ./app { inherit (inputs) crane; };
        semmeltv-web = pkgs.callPackage ./web { inherit (inputs) crane; };
        deploy-rs = inputs.deploy-rs.packages.${system}.default;
      }
      // lib.concatMapAttrs (name: hailo: {
        "hailort-${name}" = hailo.hailort;
        "hailo-firmware-${name}" = hailo.firmware;
        "gst-hailo-${name}" = hailo.gst-hailo;
        "pyhailort-${name}" = hailo.pyhailort;
      }) (pkgs.callPackages ./hailo { });
    };
}
