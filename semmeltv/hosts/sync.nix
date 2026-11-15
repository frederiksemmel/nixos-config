{
  writeShellApplication,
  rclone,
  coreutils,
  findutils,
}:
writeShellApplication {
  name = "semmeltv-sync-from";
  runtimeInputs = [
    rclone
    coreutils
    findutils
  ];
  text = ''
    remote=$1
    meta=$2
    umask 022
    sum=$(rclone hashsum sha1 "$remote" --include /library.db | cut -d' ' -f1)
    if [ -z "$sum" ]; then
      echo "$remote/library.db not found"
      exit 1
    fi
    have=$(sha1sum "$meta/library.db" 2>/dev/null | cut -d' ' -f1 || true)
    if [ "$sum" = "$have" ] && [ -d "$meta/Artwork" ]; then
      rclone sync "$remote/Artwork" "$meta/Artwork" --fast-list --stats 0
      echo "library.db unchanged ($sum), $(find "$meta/Artwork" -type f | wc -l) artwork files in sync"
      exit 0
    fi
    echo "library.db changed (''${have:-none} -> $sum), syncing"
    rclone copyto "$remote/library.db" "$meta/.library.db.new"
    rclone copy "$remote/Artwork" "$meta/Artwork" --fast-list --stats 0
    mv -f "$meta/.library.db.new" "$meta/library.db"
    rclone sync "$remote/Artwork" "$meta/Artwork" --fast-list --stats 0
    rm -f "$meta/library.toml"
    echo "synced library.db $(sha1sum "$meta/library.db" | cut -d' ' -f1) and $(find "$meta/Artwork" -type f | wc -l) artwork files"
  '';
}
