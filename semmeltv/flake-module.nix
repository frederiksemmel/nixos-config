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
    {
      pkgs,
      system,
      self',
      ...
    }:
    {
      checks = lib.optionalAttrs (system == "x86_64-linux") {
        semmeltv-web-browser = pkgs.callPackage ./web/browser-test.nix {
          inherit (self'.packages) semmeltv-web;
        };
        semmeltv-sync = pkgs.callPackage ./hosts/sync-test.nix { };
        semmeltv-vm = import ./tests/vm {
          inherit pkgs;
          inherit (self'.packages) semmeltv;
          semmeltvModule = self.nixosModules.semmeltv;
        };
        semmeltv-no-upload = import ./tests/no-upload {
          inherit pkgs;
          rqbitModule = import ./modules/rqbit.nix { inherit (inputs) nixpkgs; };
        };
      };

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
