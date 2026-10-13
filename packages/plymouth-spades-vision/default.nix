{ runCommand, plymouth }:
let
  modes = [
    "boot-up"
    "shutdown"
    "reboot"
    "updates"
    "system-upgrade"
    "firmware-upgrade"
    "system-reset"
  ];
in
# The Spades Vision logo spinner from sv-panel, looping on black. The frames are
# rendered by render.py; Plymouth plays them at a fixed 30 fps.
runCommand "plymouth-spades-vision" { } ''
  theme=$out/share/plymouth/themes/spades-vision
  mkdir -p $theme
  cp ${./frames}/throbber-*.png $theme/
  # two-step refuses to load without the password prompt images.
  cp ${plymouth}/share/plymouth/themes/spinner/{lock,entry,bullet}.png $theme/

  cat > $theme/spades-vision.plymouth <<EOF
  [Plymouth Theme]
  Name=Spades Vision
  Description=Spades Vision logo spinner on black
  ModuleName=two-step

  [two-step]
  ImageDir=$theme
  HorizontalAlignment=.5
  VerticalAlignment=.5
  Transition=none
  BackgroundStartColor=0x000000
  BackgroundEndColor=0x000000
  ${
    # Without UseEndAnimation=false two-step plays throbber-* once as an end
    # animation instead of looping it.
    builtins.concatStringsSep "\n" (map (m: "\n[${m}]\nUseEndAnimation=false") modes)
  }
  EOF
''
