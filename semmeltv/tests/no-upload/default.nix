{
  pkgs,
  rqbitModule,
}:

let
  peer = {
    services.rqbit = {
      enable = true;
      httpHost = "0.0.0.0";
      openFirewall = true;
    };
    environment.systemPackages = [
      pkgs.curl
      pkgs.jq
    ];
  };
  plainPeer =
    { nodes, ... }:
    {
      imports = [ peer ];
      services.rqbit.package = nodes.tv.services.rqbit.package;
      systemd.services.rqbit.environment.RQBIT_DHT_DISABLE = "true";
    };
in
pkgs.testers.runNixOSTest {
  name = "semmeltv-no-upload";

  nodes = {
    seeder = {
      imports = [ plainPeer ];
      systemd.services.rqbit.environment.RQBIT_RATELIMIT_UPLOAD = toString (512 * 1024);
    };
    tv.imports = [
      peer
      rqbitModule
    ];
    leecher.imports = [ plainPeer ];
  };

  testScript =
    { nodes, ... }:
    ''
      SEEDER = "${nodes.seeder.networking.primaryIPAddress}:${toString nodes.seeder.services.rqbit.peerPort}"
      TV = "${nodes.tv.networking.primaryIPAddress}:${toString nodes.tv.services.rqbit.peerPort}"
      LEECHER_IP = "${nodes.leecher.networking.primaryIPAddress}"
    ''
    + builtins.readFile ./test.py;
}
