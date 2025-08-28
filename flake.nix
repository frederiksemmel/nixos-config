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
    centerpiece.inputs.nixpkgs.follows = "nixpkgs";
    agenix.url = "github:ryantm/agenix";
    agenix.inputs.nixpkgs.follows = "nixpkgs";
    tuxedo-nixos.url = "github:sund3RRR/tuxedo-nixos";

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
          nixpkgs = {
            hostPlatform = "x86_64-linux";
            overlays = [
              (final: prev: {
                rerun = prev.rerun.overrideAttrs (oldAttrs: {
                  cargoBuildFeatures = [
                    "native_viewer"
                    "nasm"
                    # "web_viewer" removed
                  ];
                  cargoCheckFeatures = [
                    "native_viewer"
                    "nasm"
                    # "web_viewer" removed
                  ];
                  buildWebViewerFeatures = [ ]; # not strictly neccessary but cleaner
                });
              })
            ];
          };
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
