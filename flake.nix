{
  inputs = {
    # nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable-small";
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    nixpkgs-old-zotero.url = "github:nixos/nixpkgs/4975466d324710c576dc11ad614684e6bd8cad8e";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    nixos-hardware.url = "github:NixOS/nixos-hardware";
    nixos-hardware.inputs.nixpkgs.follows = "nixpkgs";
    disko.url = "github:nix-community/disko";
    disko.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    niri.url = "github:epireyn/niri-flake";
    niri.inputs.nixpkgs.follows = "nixpkgs";
    niri.inputs.nixpkgs-stable.follows = "nixpkgs";
    # centerpiece.url = "github:friedow/centerpiece";
    # centerpiece.inputs.nixpkgs.follows = "nixpkgs";  # Not following because it fucking recompiles always
    # centerpiece.inputs.treefmt-nix.follows = "treefmt-nix";
    # centerpiece.inputs.home-manager.follows = "home-manager";
    agenix.url = "github:ryantm/agenix";
    agenix.inputs.nixpkgs.follows = "nixpkgs";
    veila.url = "github:naurissteins/Veila";
    veila.inputs.nixpkgs.follows = "nixpkgs";
    # tuxedo-nixos.url = "github:sund3RRR/tuxedo-nixos";
    determinate.url = "https://flakehub.com/f/DeterminateSystems/determinate/*";

    # AI coding agents. Deliberately NOT following nixpkgs: the upstream binary
    # cache only has builds against their own nixpkgs pin.
    llm-agents.url = "github:numtide/llm-agents.nix";
    llm-agents.inputs.flake-parts.follows = "flake-parts";
    mcp-servers-nix.url = "github:natsukium/mcp-servers-nix";
    mcp-servers-nix.inputs.nixpkgs.follows = "nixpkgs";

    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  nixConfig = {
    extra-substituters = [ "https://cache.numtide.com" ];
    extra-trusted-public-keys = [
      "niks3.numtide.com-1:DTx8wZduET09hRmMtKdQDxNNthLQETkc/yaX7M4qK0g="
    ];
  };

  outputs =
    inputs@{ self, ... }:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } (
      { withSystem, ... }:
      let
        specialArgs = {
          flake = { inherit self inputs; };
        };
      in
      {
        systems = [
          "x86_64-linux"
          "aarch64-linux"
        ];
        imports = [
          inputs.home-manager.flakeModules.home-manager
          inputs.treefmt-nix.flakeModule # `nix fmt`
        ];

        flake = {
          # Configurations for Linux (NixOS) machines
          nixosConfigurations."fstux" = inputs.nixpkgs.lib.nixosSystem {
            inherit specialArgs;
            modules = [
              ./configurations/nixos/fstux
              inputs.home-manager.nixosModules.home-manager
              { home-manager.extraSpecialArgs = specialArgs; }
            ];
          };

          homeModules.frederik = ./home/users/frederik.nix;

          overlays.default = final: prev: {
            claudebar = final.callPackage ./packages/claudebar.nix { };
            claude-notify = final.callPackage ./packages/claude-notify.nix { };
            workspace-mcp = final.callPackage ./packages/workspace-mcp.nix { };
            plymouth-spades-vision = final.callPackage ./packages/plymouth-spades-vision { };
            # The viewer makes every recording streamed in from an SDK the active one,
            # so it keeps jumping while several recordings stream in at once. The patch
            # adds a "Switch to newly streamed recordings" setting; with it off, such a
            # recording only shows up in the recordings panel while another one is being
            # shown. Upstream issue: rerun-io/rerun#12950.
            rerun = prev.rerun.overrideAttrs (old: {
              patches = (old.patches or [ ]) ++ [ ./packages/rerun-streamed-recordings-setting.patch ];
            });
            # Determinate Nix emits activity type 10113, which nom 2.2.0 rejects with a
            # ParseNixJSONMessageError per message. Drop once nixpkgs ships a nom release
            # with maralorn/nix-output-monitor#321.
            nix-output-monitor = prev.nix-output-monitor.overrideAttrs (old: {
              patches = (old.patches or [ ]) ++ [
                ./packages/nix-output-monitor-unknown-activity-types.patch
              ];
            });
            # When its notification helper fails to start, foot keeps -1 as the
            # helper's pid and closes the notification with kill(-1, SIGINT), which
            # interrupts every process of the user, niri included.
            foot = prev.foot.overrideAttrs (old: {
              patches = (old.patches or [ ]) ++ [ ./packages/foot-notify-close-pid.patch ];
            });
          };

          homeConfigurations =
            let
              mkHome =
                extraModules:
                withSystem "x86_64-linux" (
                  { pkgs, ... }:
                  inputs.home-manager.lib.homeManagerConfiguration {
                    inherit pkgs;
                    extraSpecialArgs = specialArgs;
                    modules = [
                      self.homeModules.frederik
                      inputs.niri.homeModules.niri # need to do it separately because in fstux we do it via the nixosModule
                      { home.stateVersion = "25.05"; }
                    ]
                    ++ extraModules;
                  }
                );
            in
            {
              frederik = mkHome [ ];
              "frederik@beast" = mkHome [ ./home/hosts/beast.nix ];
            };
        };

        perSystem =
          { pkgs, system, ... }:
          {
            _module.args.pkgs = import inputs.nixpkgs {
              inherit system;
              config = {
                allowUnfree = true;
              };
              overlays = [ self.overlays.default ];
            };
            packages = {
              inherit (pkgs) claudebar claude-notify;
            };
            treefmt = {
              projectRootFile = "flake.nix"; # optional, default is "treefmt.toml"
              programs = {
                nixfmt.enable = true;
              };
            };
          };
      }
    );
}
