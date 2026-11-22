{
  flake.nixosModules.beachparty = ./module.nix;

  perSystem =
    { pkgs, ... }:
    {
      packages.beachparty = pkgs.callPackage ./package.nix { };
    };
}
