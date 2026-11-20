"""ffmpeg's arguments for a phone stream, as semmeltv/app/src/stream/remux.rs
builds them for a browser that takes HEVC and 5.1 AAC (Firefox, Chrome):
the video copied, the sound converted to 5.1, every text subtitle track
written out as WebVTT.

    python3 -I remux.py FILE AT DIR    # runs ffmpeg into DIR
"""

import json
import os
import subprocess
import sys

SURROUND = "aformat=channel_layouts=5.1,alimiter=limit=-1dB:level=false"

# How many media segments remux.rs waits for before it answers an open.
READY_SEGMENTS = int(os.environ.get("READY_SEGMENTS", "3"))


def segments(playlist):
    return sum(1 for line in playlist.splitlines() if not line.startswith("#") and line.endswith(".m4s"))


def text_subtitles(source):
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "s", "-show_entries", "stream=codec_name", "-of", "json", source],
        capture_output=True,
        check=True,
        text=True,
    )
    streams = json.loads(probe.stdout)["streams"]
    return [index for index, stream in enumerate(streams) if stream["codec_name"] in ("subrip", "ass", "ssa", "webvtt", "mov_text")]


def arguments(source, directory, at, subtitles, audio=0):
    args = ["-nostdin", "-hide_banner", "-loglevel", "error"]
    if at > 0:
        args += ["-ss", f"{at:.3f}", "-noaccurate_seek"]
    args += ["-copyts", "-readrate", "2", "-readrate_initial_burst", "60", "-i", source]
    args += ["-map", "0:v:0", "-map", f"0:a:{audio}", "-c:v", "copy", "-tag:v", "hvc1"]
    args += ["-af", SURROUND, "-c:a", "aac", "-b:a", "384k"]
    args += ["-sn", "-dn", "-max_muxing_queue_size", "4096", "-f", "hls", "-hls_time", "6", "-hls_list_size", "0"]
    args += ["-hls_playlist_type", "event", "-hls_segment_type", "fmp4", "-hls_flags", "independent_segments+temp_file"]
    args += ["-hls_fmp4_init_filename", "init.mp4", "-hls_segment_filename", os.path.join(directory, "%d.m4s")]
    args.append(os.path.join(directory, "video.m3u8"))
    for track in subtitles:
        args += ["-map", f"0:s:{track}", "-c:s", "webvtt", "-flush_packets", "1", "-f", "webvtt"]
        args.append(os.path.join(directory, f"sub{track}.vtt"))
    return args


def start(source, directory, at, subtitles=None):
    os.makedirs(directory, exist_ok=True)
    if subtitles is None:
        subtitles = text_subtitles(source)
    log = open(os.path.join(directory, "ffmpeg.log"), "w")
    return subprocess.Popen(["ffmpeg", *arguments(source, directory, at, subtitles)], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log)


def first_time(directory):
    joined = f"concat:{os.path.join(directory, 'init.mp4')}|{os.path.join(directory, '0.m4s')}"
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=start_time", "-of", "csv=p=0", joined],
        capture_output=True,
        text=True,
    )
    try:
        return float(probe.stdout.strip())
    except ValueError:
        return None


if __name__ == "__main__":
    start(sys.argv[1], sys.argv[3], float(sys.argv[2])).wait()
