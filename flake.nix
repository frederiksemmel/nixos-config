{
  nixConfig = {
    extra-substituters = [
      "https://niri.cachix.org"
    ];
    extra-trusted-public-keys = [
      "niri.cachix.org-1:Wv0OmO7PsuocRKzfDoJ3mulSl7Z6oezYhGhR+3W2964="
    ];
  };
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
    niri.url = "github:sodiboo/niri-flake";
    niri.inputs.nixpkgs.follows = "nixpkgs";
    zen-browser.url = "github:0xc000022070/zen-browser-flake";
    zen-browser.inputs.nixpkgs.follows = "nixpkgs";
    centerpiece.url = "github:friedow/centerpiece";
    centerpiece.inputs.nixpkgs.follows = "nixpkgs";
    agenix.url = "github:ryantm/agenix";
    agenix.inputs.nixpkgs.follows = "nixpkgs";
    iwmenu.url = "github:e-tho/iwmenu";
    iwmenu.inputs.nixpkgs.follows = "nixpkgs";
    tuxedo-nixos.url = "github:sund3RRR/tuxedo-nixos";
    # tuxedo-nixos.inputs.nixpkgs.follows = "nixpkgs";  Need duplicated nixpkgs because of electron
    nixai.url = "github:olafkfreund/nix-ai-help";
    nixai.inputs.nixpkgs.follows = "nixpkgs";

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
            ./configurations/nixos/fstux
          ];
        };

        # home-manager configuration goes here.
        homeModules.default = ./home/default.nix;
      };
      perSystem =
        { self', pkgs, ... }:
        {
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
