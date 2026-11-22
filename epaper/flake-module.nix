{
  self,
  inputs,
  flake-parts-lib,
  ...
}:
{
  flake.nixosModules.epaper = flake-parts-lib.importApply ./module.nix { inherit (inputs) crane; };

  perSystem =
    { pkgs, system, ... }:
    let
      epaper =
        if system == "aarch64-linux" then
          self.nixosConfigurations.rpi3.config.services.epaper.package
        else
          pkgs.callPackage ./package.nix { inherit (inputs) crane; };
    in
    {
      packages.epaper = epaper;
      checks.epaper-test = epaper.tests.cargo;
    };
}
