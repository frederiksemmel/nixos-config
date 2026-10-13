{
  lib,
  runCommand,
  writeText,
  writeShellScript,
  writeShellScriptBin,
  imagemagick,
  makeFontsConf,
  dejavu_fonts,
  quickshell,
  niri,
  systemd,
  # The lock screen the greeter mirrors; its fonts and icons are reused.
  veila,
  # Image behind the greeter; blurred at build time.
  wallpaper,
  # The account the greeter logs in.
  user,
  # { command, desktopNames } of the session greetd starts after login.
  session,
  # Body of the greeter niri's `input { }` block.
  input ? "",
}:
let
  fonts = makeFontsConf {
    fontDirectories = [
      "${veila}/share/veila/fonts"
      dejavu_fonts
    ];
  };

  background = runCommand "greeter-background.jpg" { nativeBuildInputs = [ imagemagick ]; } ''
    magick ${wallpaper} -resize 960x -blur 0x7 -filter Lanczos -resize 3840x \
      -attenuate 0.05 +noise Gaussian -quality 92 jpg:$out
  '';

  config = writeText "Config.qml" ''
    pragma Singleton

    import Quickshell

    Singleton {
        readonly property string user: ${builtins.toJSON user}
        readonly property var sessionCommand: ${builtins.toJSON session.command}
        readonly property var sessionEnvironment: ${
          builtins.toJSON [
            "XDG_SESSION_TYPE=wayland"
            "XDG_SESSION_DESKTOP=${builtins.head session.desktopNames}"
            "XDG_CURRENT_DESKTOP=${lib.concatStringsSep ":" session.desktopNames}"
          ]
        }
        readonly property string background: ${builtins.toJSON "${background}"}
        readonly property var suspend: ${
          builtins.toJSON [
            "${systemd}/bin/systemctl"
            "suspend"
          ]
        }
        readonly property var reboot: ${
          builtins.toJSON [
            "${systemd}/bin/systemctl"
            "reboot"
          ]
        }
        readonly property var poweroff: ${
          builtins.toJSON [
            "${systemd}/bin/systemctl"
            "poweroff"
          ]
        }
        readonly property string niri: ${builtins.toJSON (lib.getExe niri)}
    }
  '';

  # veila tints its icons when drawing them; Qt draws currentColor and unset
  # fills black, so they are made white here and dimmed by opacity.
  shell = runCommand "greeter-shell" { } ''
    cp -r ${./shell} $out
    chmod -R u+w $out
    cp ${config} $out/Config.qml
    mkdir $out/icons
    for f in ${veila}/share/veila/icons/{battery,power}/*.svg; do
      if grep -q currentColor "$f"; then
        sed 's/currentColor/#ffffff/g' "$f"
      else
        sed '0,/<svg /s//<svg fill="#ffffff" /' "$f"
      fi > $out/icons/$(basename "$f")
    done
  '';

  # niri outlives its startup command, so the greeter ends the compositor
  # itself; greetd then starts the session, or a fresh greeter if it crashed.
  start = writeShellScript "greeter-start" ''
    ${lib.getExe quickshell} --path ${shell}
    exec ${lib.getExe niri} msg action quit --skip-confirmation
  '';

  niriConfig =
    runCommand "greeter-niri.kdl"
      {
        nativeBuildInputs = [ niri ];
        config = ''
          input {
          ${input}
          }
          spawn-at-startup "${start}"
          hotkey-overlay {
              skip-at-startup
          }
          gestures {
              hot-corners {
                  off
              }
          }
          xwayland-satellite {
              off
          }
          prefer-no-csd
          screenshot-path null
        '';
        passAsFile = [ "config" ];
      }
      ''
        cp $configPath $out
        niri validate --config $out
      '';
in
writeShellScriptBin "greeter" ''
  # The greeter user's home is /var/empty.
  export XDG_CACHE_HOME="$XDG_RUNTIME_DIR/cache"
  export FONTCONFIG_FILE=${fonts}
  exec ${lib.getExe niri} --config ${niriConfig}
''
