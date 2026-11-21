{ inputs, flake-parts-lib, ... }:
{
  flake.nixosModules.epaper = flake-parts-lib.importApply ./module.nix { inherit (inputs) crane; };

  perSystem =
    { pkgs, ... }:
    let
      epaper = pkgs.callPackage ./package.nix { inherit (inputs) crane; };
    in
    {
      packages.epaper = epaper;
      checks.epaper-test = epaper.tests.cargo;
    };
}
