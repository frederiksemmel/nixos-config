{
  lib,
  pkgs,
  outputs,
  ...
}:
let
  inherit (outputs) nixosConfigurations;
  writeNushellApp = pkgs.callPackage ../../lib/writeShellApplication.nix { };
  toNu = v: "(\"${lib.escape [ "\"" "\\" ] (builtins.toJSON v)}\" | from json)";
  targetSystems = lib.filterAttrs (
    systemName: _config: builtins.match "bern-.*" systemName != null
  ) nixosConfigurations;
  targetInstallers = builtins.attrValues (
    builtins.mapAttrs (name: conf: {
      inherit name;
      inherit (conf.config.system.build) diskoScript;
      inherit (conf.config.system.build) toplevel;
    }) targetSystems
  );
  installer = writeNushellApp {
    name = "installer";
    runtimeInputs = with pkgs; [
      nixos-install-tools
      nix
    ]; # it needs nix here, because apparently nixos-install needs it
    text = ''
      ##!/usr/bin/nu added only for syntax highlighting
      # Define the list of available systems.
      let installers = ${toNu targetInstallers}
      print $installers

      let chosen = $installers | get name | input list "Select target system:" | str trim

      if ($chosen == "") {
        print "No system selected. Exiting."
        exit 1
      }

      print "You selected:" $chosen

      let diskoScript = $installers | where name == $chosen | first | get diskoScript
      let toplevel = $installers | where name == $chosen | first | get toplevel

      bash $diskoScript  # this is the prebuild script to format and mount all the disks which already is based on the right disk config
      nixos-install --no-root-passwd --no-channel-copy --system $toplevel
    '';
  };
in
{
  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];
  nixpkgs = {
    overlays = [
      outputs.overlays.additions
      outputs.overlays.modifications
      (final: _prev: {
        inherit installer;
      })

    ];
    config = {
      allowUnfree = true;
      cudaSupport = true;
    };
  };

  boot.loader.systemd-boot.enable = true;

  environment.systemPackages = with pkgs; [
    helix
    fd
    ripgrep
    google-cloud-sdk
    nix-tree

    installer
  ];

  services.openssh = {
    enable = true;
    ports = [ 22 ];
    settings = {
      PasswordAuthentication = true;
    };
  };
  users.users = {
    root.openssh.authorizedKeys.keyFiles = [ ./id_spadesvision_root.pub ];
  };

  systemd.network.networks."09-internet-lan" = {
    matchConfig.Type = "ether";
    networkConfig.DHCP = "yes";
  };
  networking.wireless.enable = false;
  networking.wireless.iwd.enable = true; # adds wifi networking to installer iso

  nixpkgs.hostPlatform = "x86_64-linux";
  system.stateVersion = "24.11";
}
