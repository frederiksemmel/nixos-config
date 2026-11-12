{ bluez-alsa }:
bluez-alsa.overrideAttrs (old: {
  configureFlags = old.configureFlags ++ [ "--enable-cli" ];
})
