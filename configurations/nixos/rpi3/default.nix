{
  config,
  flake,
  nixos-raspberrypi,
  ...
}:
{
  imports = [
    ../raspberry-pi.nix
    nixos-raspberrypi.nixosModules.raspberry-pi-3.base
    flake.inputs.agenix.nixosModules.default
    flake.self.nixosModules.epaper
    flake.self.nixosModules.beachparty
  ];

  networking.hostName = "rpi3";

  services.epaper = {
    enable = true;
    tokenFile = config.age.secrets.epaper_telegram_token.path;
    rotation = 270;
    whatsapp.enable = true;
  };
  users.users.frederik.extraGroups = [ "epaper" ];

  services.beachparty = {
    enable = true;
    domain = "beachparty.semmeltv.ch";
    target = "semmeltv5.semmeltv.ch";
    relayedFrom = "192.168.1.178";
    desecTokenFile = config.age.secrets.desec_token.path;
  };

  age.secrets.epaper_telegram_token.file = ../../../secrets/epaper_telegram_token.age;
  age.secrets.desec_token.file = ../../../secrets/desec_token.age;

  systemd.services.power-led-off = {
    description = "Turn off the red power LED";
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = ''
      echo none > /sys/class/leds/PWR/trigger
      echo 0 > /sys/class/leds/PWR/brightness
    '';
  };

  zramSwap = {
    enable = true;
    memoryPercent = 100;
  };
}
