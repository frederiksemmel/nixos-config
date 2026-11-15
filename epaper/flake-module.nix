{ inputs, flake-parts-lib, ... }:
{
  flake.nixosModules.epaper = flake-parts-lib.importApply ./module.nix { inherit (inputs) crane; };

  perSystem =
    { pkgs, ... }:
    {
      packages.epaper = pkgs.callPackage ./package.nix { inherit (inputs) crane; };
    };
}
