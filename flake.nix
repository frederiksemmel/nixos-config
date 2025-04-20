{
  inputs = {
    # Principle inputs (updated by `nix run .#update`)
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    nixos-hardware.url = "github:NixOS/nixos-hardware";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

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
        nixosConfigurations."fstux" = self.nixos-unified.lib.mkLinuxSystem { home-manager = true; } {
          nixpkgs.hostPlatform = "x86_64-linux";
          imports = [
            # Your machine's configuration.nix goes here
            ./configurations/nixos/fstux
            # Setup home-manager in NixOS config
            {
              home-manager.users.frederik = {
                imports = [ self.homeModules.default ];
                home.stateVersion = "25.05";
              };
            }
          ];
        };

        # home-manager configuration goes here.
        homeModules.default = ./home/default.nix;
      };
    };
}
