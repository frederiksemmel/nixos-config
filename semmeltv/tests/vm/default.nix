{
  pkgs,
  semmeltvModule,
  semmeltv,
}:

let
  library = pkgs.callPackage ./library.nix { inherit semmeltv; };
  serviceToken = pkgs.writeText "semmeltv-service-token" "vm-test-service-token";
in
pkgs.testers.runNixOSTest {
  name = "semmeltv-vm";
  enableOCR = true;
  extraPythonPackages = ps: [ ps.playwright ];

  nodes.tv = {
    imports = [ semmeltvModule ];

    virtualisation = {
      memorySize = 4096;
      cores = 2;
      qemu.options = [
        "-vga none"
        "-device virtio-gpu-pci"
      ];
    };

    boot.kernelModules = [ "snd-dummy" ];

    services.semmeltv = {
      enable = true;
      adminUser = "root";
      package = semmeltv;
      api.enable = true;
      web = {
        enable = true;
        signIn = {
          origins = [ "http://localhost:8080" ];
          serviceTokenFile = "${serviceToken}";
        };
      };
    };

    systemd.tmpfiles.rules = [ "L+ /var/lib/semmeltv/library - - - - ${library}" ];

    services.nginx = {
      enable = true;
      virtualHosts.phone = {
        default = true;
        listen = [
          {
            addr = "0.0.0.0";
            port = 80;
          }
        ];
        locations."/" = {
          proxyPass = "http://127.0.0.1:8091";
          proxyWebsockets = true;
        };
      };
    };
    networking.firewall.allowedTCPPorts = [ 80 ];
  };

  testScript = ''
    import os
    os.environ["PLAYWRIGHT_BROWSERS_PATH"] = "${pkgs.playwright-driver.browsers}"
    os.environ["PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS"] = "true"
    os.environ["FONTCONFIG_FILE"] = "${pkgs.makeFontsConf { fontDirectories = [ pkgs.inter ]; }}"
  ''
  + builtins.readFile ./test.py
  + builtins.readFile ./stream.py;
}
