{
  runCommand,
  xcursorgen,
}:

runCommand "semmeltv-blank-cursor" { nativeBuildInputs = [ xcursorgen ]; } ''
  theme=$out/share/icons/blank
  mkdir -p $theme/cursors
  echo '[Icon Theme]' > $theme/index.theme
  echo 'Name=blank' >> $theme/index.theme

  echo 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=' \
    | base64 -d > blank.png
  echo '24 0 0 blank.png' > blank.config
  xcursorgen blank.config $theme/cursors/default

  for name in left_ptr arrow pointer hand1 hand2 grab grabbing text xterm wait watch progress \
    help crosshair move all-scroll not-allowed col-resize row-resize ew-resize ns-resize; do
    ln -s default $theme/cursors/$name
  done
''
