import hashlib
import json
import os
import tempfile
from pathlib import Path


def bencode(value):
    if isinstance(value, int):
        return b"i%de" % value
    if isinstance(value, bytes):
        return b"%d:%s" % (len(value), value)
    if isinstance(value, dict):
        return b"d" + b"".join(bencode(key) + bencode(value[key]) for key in sorted(value)) + b"e"
    raise TypeError(value)


PIECE = 256 * 1024
data = os.urandom(32 * 1024 * 1024)
pieces = b"".join(hashlib.sha1(data[start:start + PIECE]).digest() for start in range(0, len(data), PIECE))
host = Path(tempfile.mkdtemp())
(host / "film.mkv").write_bytes(data)
(host / "film.torrent").write_bytes(bencode({b"info": {b"name": b"film.mkv", b"length": len(data), b"piece length": PIECE, b"pieces": pieces}}))

STATS = "curl -fsS http://127.0.0.1:3030/torrents/0/stats/v1"


def add(machine, peers=None):
    query = "overwrite=true" + (f"&initial_peers={peers}" if peers else "")
    machine.succeed(f"curl -fsS --data-binary @/root/film.torrent 'http://127.0.0.1:3030/torrents?{query}'")


def stats(machine):
    return json.loads(machine.succeed(STATS))


start_all()
for machine in (seeder, tv, leecher):
    machine.wait_for_unit("rqbit.service")
    machine.wait_for_open_port(3030)
    machine.copy_from_host(str(host / "film.torrent"), "/root/film.torrent")

with subtest("SemmelTV's rqbit starts with uploading disabled"):
    tv.succeed("journalctl -u rqbit.service | grep -q 'uploading disabled'")

with subtest("the seeder has the whole film"):
    seeder.copy_from_host(str(host / "film.mkv"), "/var/lib/rqbit/downloads/film.mkv")
    seeder.succeed("chown -R rqbit:rqbit /var/lib/rqbit/downloads")
    seeder.succeed(f"iptables -I INPUT -s {LEECHER_IP} -j DROP && iptables -I OUTPUT -d {LEECHER_IP} -j DROP")
    add(seeder)
    seeder.wait_until_succeeds(f"{STATS} | jq -e .finished", timeout=60)

with subtest("a peer that asks the TV for pieces mid-download gets none"):
    add(tv, SEEDER)
    tv.wait_until_succeeds(f"{STATS} | jq -e '.progress_bytes > 0'", timeout=60)
    add(leecher, TV)
    leecher.wait_until_succeeds(f"{STATS} | jq -e '.live.snapshot.peer_stats.live >= 1'", timeout=60)
    assert not stats(tv)["finished"], "the TV was still downloading while the leecher was connected"
    tv.wait_until_succeeds(f"{STATS} | jq -e .finished", timeout=300)
    assert stats(seeder)["uploaded_bytes"] >= len(data), "the same rqbit uploads without RQBIT_DISABLE_UPLOAD"
    assert stats(tv)["uploaded_bytes"] == 0, f"the TV uploaded {stats(tv)['uploaded_bytes']} bytes"
    assert stats(leecher)["progress_bytes"] == 0, f"the leecher got {stats(leecher)['progress_bytes']} bytes from the TV"
