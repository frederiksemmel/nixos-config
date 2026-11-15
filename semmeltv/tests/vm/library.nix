{
  ffmpeg-headless,
  runCommand,
  semmeltv,
}:

runCommand "semmeltv-test-library"
  {
    nativeBuildInputs = [
      ffmpeg-headless
      semmeltv
    ];
  }
  ''
    clip() {
      mkdir -p "$(dirname "$1")"
      ffmpeg -loglevel error \
        -f lavfi -i "testsrc2=size=640x360:rate=25:duration=$2" \
        -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=$2" \
        -filter_complex "[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]" \
        -map 0:v -map "[a]" -c:v libx264 -preset ultrafast -crf 35 -c:a aac -b:a 96k \
        "$1"
    }

    srt() {
      n=1
      t=0
      while [ "$t" -lt "$1" ]; do
        printf '%d\n00:%02d:%02d,000 --> 00:%02d:%02d,000\n%s %d\n\n' \
          "$n" $((t / 60)) $((t % 60)) $(((t + 4) / 60)) $(((t + 4) % 60)) "$2" "$n"
        n=$((n + 1))
        t=$((t + 5))
      done
    }

    mkdir -p $out
    cd $out

    clip plain.mkv 240
    srt 240 "Film line" > embedded.srt
    mkdir -p Movies/Testfilm
    ffmpeg -loglevel error -i plain.mkv -i embedded.srt -map 0 -map 1 -c copy -c:s srt \
      -metadata:s:s:0 language=eng Movies/Testfilm/Testfilm.2021.mkv
    rm plain.mkv embedded.srt

    for episode in 1 2; do
      clip Series/Testserie/S01/Testserie.S01E0$episode.mkv 180
      srt 180 "Episode $episode line" > Series/Testserie/S01/Testserie.S01E0$episode.en.srt
    done

    semmeltv --manifest library.db library from-toml ${./library.toml}
  ''
