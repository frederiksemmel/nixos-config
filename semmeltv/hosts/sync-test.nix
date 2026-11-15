{
  callPackage,
  runCommand,
  lib,
}:
let
  sync = lib.getExe (callPackage ./sync.nix { });
in
runCommand "semmeltv-sync-test" { } ''
  set -eu
  export HOME=$TMPDIR RCLONE_CONFIG=$TMPDIR/rclone.conf
  touch "$RCLONE_CONFIG"
  remote=$TMPDIR/pcloud
  meta=$TMPDIR/meta
  mkdir -p "$remote/Artwork/movie/1" "$meta"
  echo one > "$remote/library.db"
  echo poster > "$remote/Artwork/movie/1/poster.jpg"
  same() { cmp -s "$1" "$2" || { echo "FAIL: $3"; exit 1; }; }
  gone() { [ ! -e "$1" ] || { echo "FAIL: $2"; exit 1; }; }

  ${sync} "$remote" "$meta"
  same "$remote/library.db" "$meta/library.db" "a first run copies library.db"
  same "$remote/Artwork/movie/1/poster.jpg" "$meta/Artwork/movie/1/poster.jpg" "a first run copies the artwork"

  echo backdrop > "$remote/Artwork/movie/1/backdrop.jpg"
  echo "a bigger poster" > "$remote/Artwork/movie/1/poster.jpg"
  said=$(${sync} "$remote" "$meta")
  grep -q "library.db unchanged" <<< "$said" || { echo "FAIL: library.db counts as unchanged"; exit 1; }
  same "$remote/Artwork/movie/1/backdrop.jpg" "$meta/Artwork/movie/1/backdrop.jpg" "a new image arrives while library.db stays the same"
  same "$remote/Artwork/movie/1/poster.jpg" "$meta/Artwork/movie/1/poster.jpg" "a replaced image arrives while library.db stays the same"

  rm "$remote/Artwork/movie/1/backdrop.jpg"
  ${sync} "$remote" "$meta"
  gone "$meta/Artwork/movie/1/backdrop.jpg" "an image removed on pCloud is removed on the TV"

  echo two > "$remote/library.db"
  touch "$meta/library.toml"
  said=$(${sync} "$remote" "$meta")
  grep -q "library.db changed" <<< "$said" || { echo "FAIL: library.db counts as changed"; exit 1; }
  same "$remote/library.db" "$meta/library.db" "a changed library.db replaces the local one"
  gone "$meta/library.toml" "the retired library.toml is removed"

  rm "$remote/library.db"
  if ${sync} "$remote" "$meta"; then echo "FAIL: a missing library.db is an error"; exit 1; fi
  same <(echo two) "$meta/library.db" "a missing library.db on pCloud keeps the local one"

  touch $out
''
