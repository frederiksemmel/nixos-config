{
  inputs = {
    # Principle inputs (updated by `nix run .#update`)
    # nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable-small";
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    nixos-hardware.url = "github:NixOS/nixos-hardware";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    niri.url = "github:sodiboo/niri-flake";
    niri.inputs.nixpkgs.follows = "nixpkgs";
    zen-browser.url = "github:0xc000022070/zen-browser-flake";
    zen-browser.inputs.nixpkgs.follows = "nixpkgs";
    centerpiece.url = "github:friedow/centerpiece";
    # centerpiece.inputs.nixpkgs.follows = "nixpkgs";  # Not following because it fucking recompiles always
    centerpiece.inputs.treefmt-nix.follows = "treefmt-nix";
    centerpiece.inputs.home-manager.follows = "home-manager";
    agenix.url = "github:ryantm/agenix";
    agenix.inputs.nixpkgs.follows = "nixpkgs";
    # tuxedo-nixos.url = "github:sund3RRR/tuxedo-nixos";
    determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/*";

    flake-parts.url = "github:hercules-ci/flake-parts";
    nixos-unified.url = "github:srid/nixos-unified";
  };

  outputs =
    inputs@{ self, ... }:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      imports = [
        inputs.nixos-unified.flakeModules.default
        inputs.treefmt-nix.flakeModule # `nix fmt`
      ];

      flake = {
        # Configurations for Linux (NixOS) machines
        nixosConfigurations."fstux" = self.nixos-unified.lib.mkLinuxSystem {
          home-manager = true;
        } ./configurations/nixos/fstux;

        # home-manager configuration goes here.
        homeModules.frederik = ./home/users/frederik.nix;
        homeModules.ubuntu = ./home/users/ubuntu.nix;
      };

      perSystem =
        {
          self',
          pkgs,
          system,
          ...
        }:
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
            };
          };
          legacyPackages.homeConfigurations.frederik = self.nixos-unified.lib.mkHomeConfiguration pkgs (
            { ... }:
            {
              imports = with inputs; [
                self.homeModules.frederik
                niri.homeModules.niri # need to do it separately because in fstux we do it via the nixosModule
              ];
              home.stateVersion = "25.05";
            }
          );
          legacyPackages.homeConfigurations.ubuntu = self.nixos-unified.lib.mkHomeConfiguration pkgs (
            { ... }:
            {
              imports = [
                self.homeModules.ubuntu
              ];
              home.stateVersion = "25.11";
            }
          );
          packages.default = self'.packages.activate;
          treefmt = {
            projectRootFile = "flake.nix"; # optional, default is "treefmt.toml"
            programs = {
              nixfmt.enable = true;
            };
          };
        };
    };
}
