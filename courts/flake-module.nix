{ inputs, flake-parts-lib, ... }:
{
  flake.nixosModules.courts = flake-parts-lib.importApply ./module.nix { inherit (inputs) crane; };

  perSystem =
    { pkgs, ... }:
    {
      packages.courts = pkgs.callPackage ./package.nix { inherit (inputs) crane; };
    };
}
