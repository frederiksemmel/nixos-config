"""Lists each fMP4 segment of an HLS stream directory: per track, where its
samples start and end in seconds, from the boxes themselves (tfdt + trun),
next to what the playlist says.

    python3 -I inspect_segments.py DIR
"""

import os
import re
import struct
import sys


def boxes(data, start=0, end=None):
    end = len(data) if end is None else end
    at = start
    while at + 8 <= end:
        size, kind = struct.unpack(">I4s", data[at : at + 8])
        header = 8
        if size == 1:
            size = struct.unpack(">Q", data[at + 8 : at + 16])[0]
            header = 16
        elif size == 0:
            size = end - at
        yield kind.decode("latin-1"), at + header, at + size
        at += size


def child(data, start, end, kind):
    for found, body, stop in boxes(data, start, end):
        if found == kind:
            return body, stop
    return None


def timescales(init):
    """track id -> (timescale, handler) from the init segment."""
    result = {}
    moov = child(init, 0, len(init), "moov")
    for kind, body, stop in boxes(init, *moov):
        if kind != "trak":
            continue
        tkhd = child(init, body, stop, "tkhd")
        version = init[tkhd[0]]
        track = struct.unpack(">I", init[tkhd[0] + (20 if version else 12) : tkhd[0] + (24 if version else 16)])[0]
        mdia = child(init, body, stop, "mdia")
        mdhd = child(init, *mdia, "mdhd")
        version = init[mdhd[0]]
        scale = struct.unpack(">I", init[mdhd[0] + (20 if version else 12) : mdhd[0] + (24 if version else 16)])[0]
        hdlr = child(init, *mdia, "hdlr")
        handler = init[hdlr[0] + 8 : hdlr[0] + 12].decode("latin-1")
        result[track] = (scale, handler)
    return result


def trun_duration(data, body, default_duration):
    flags = struct.unpack(">I", data[body : body + 4])[0] & 0xFFFFFF
    count = struct.unpack(">I", data[body + 4 : body + 8])[0]
    at = body + 8
    if flags & 0x1:
        at += 4
    if flags & 0x4:
        at += 4
    total = 0
    for _ in range(count):
        duration = default_duration
        if flags & 0x100:
            duration = struct.unpack(">I", data[at : at + 4])[0]
            at += 4
        if flags & 0x200:
            at += 4
        if flags & 0x400:
            at += 4
        if flags & 0x800:
            at += 4
        total += duration
    return total, count


def segment_tracks(data):
    """track id -> (start ticks, duration ticks, samples), over every moof."""
    result = {}
    for kind, body, stop in boxes(data):
        if kind != "moof":
            continue
        for kind, tbody, tstop in boxes(data, body, stop):
            if kind != "traf":
                continue
            tfhd = child(data, tbody, tstop, "tfhd")
            flags = struct.unpack(">I", data[tfhd[0] : tfhd[0] + 4])[0] & 0xFFFFFF
            track = struct.unpack(">I", data[tfhd[0] + 4 : tfhd[0] + 8])[0]
            at = tfhd[0] + 8
            if flags & 0x1:
                at += 8
            if flags & 0x2:
                at += 4
            default_duration = 0
            if flags & 0x8:
                default_duration = struct.unpack(">I", data[at : at + 4])[0]
            tfdt = child(data, tbody, tstop, "tfdt")
            version = data[tfdt[0]]
            if version:
                decode = struct.unpack(">Q", data[tfdt[0] + 4 : tfdt[0] + 12])[0]
            else:
                decode = struct.unpack(">I", data[tfdt[0] + 4 : tfdt[0] + 8])[0]
            duration, samples = 0, 0
            for kind, rbody, _ in boxes(data, tbody, tstop):
                if kind == "trun":
                    more, count = trun_duration(data, rbody, default_duration)
                    duration += more
                    samples += count
            if track in result:
                first, before, counted = result[track]
                result[track] = (first, before + duration, counted + samples)
            else:
                result[track] = (decode, duration, samples)
    return result


def main(directory):
    init = open(os.path.join(directory, "init.mp4"), "rb").read()
    scales = timescales(init)
    playlist = open(os.path.join(directory, "video.m3u8")).read()
    listed = re.findall(r"#EXTINF:([0-9.]+),\s*\n(\S+)", playlist)
    print("tracks:", {track: f"{handler} 1/{scale}" for track, (scale, handler) in scales.items()})
    head = [line for line in playlist.splitlines() if line.startswith("#") and not line.startswith("#EXTINF")]
    print("playlist:", " ".join(head))
    position = 0.0
    for duration, name in listed:
        data = open(os.path.join(directory, name), "rb").read()
        parts = []
        for track, (start, length, samples) in sorted(segment_tracks(data).items()):
            scale, handler = scales[track]
            parts.append(f"{handler} {start / scale:9.3f}-{(start + length) / scale:9.3f} ({samples})")
        print(f"{name:>7} playlist {position:8.3f}+{float(duration):6.3f}  " + "  ".join(parts))
        position += float(duration)


if __name__ == "__main__":
    main(sys.argv[1])
