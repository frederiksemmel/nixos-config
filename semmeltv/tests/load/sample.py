"""What a TV spends while streams run: one JSON line per interval.

CPU is in cores (4.0 = all four of a Pi 5), split by process name; network
and disk in Mbit/s; memory in MiB.

    sudo python3 sample.py --seconds 180 --interval 5
"""

import argparse
import json
import os
import time

GROUPS = ("ffmpeg", "rclone", "semmeltv", "mpv", "tailscaled", "nginx", "python3")
TICK = os.sysconf("SC_CLK_TCK")


def processes():
    found = {}
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/stat") as file:
                stat = file.read()
        except OSError:
            continue
        name = stat[stat.index("(") + 1 : stat.rindex(")")]
        fields = stat[stat.rindex(")") + 2 :].split()
        group = next((g for g in GROUPS if name.lstrip(".").startswith(g)), "other")
        found[pid] = (group, int(fields[11]) + int(fields[12]))
    return found


def total_cpu():
    with open("/proc/stat") as file:
        fields = [int(x) for x in file.readline().split()[1:]]
    return sum(fields), fields[3] + fields[4], fields[4]


def network():
    found = {}
    with open("/proc/net/dev") as file:
        for line in file.readlines()[2:]:
            name, data = line.split(":", 1)
            fields = data.split()
            found[name.strip()] = (int(fields[0]), int(fields[8]))
    return found


def disk():
    with open("/proc/diskstats") as file:
        for line in file:
            fields = line.split()
            if fields[2] == "mmcblk0":
                return int(fields[5]) * 512, int(fields[9]) * 512, int(fields[12])
    return 0, 0, 0


def memory():
    with open("/proc/meminfo") as file:
        found = {line.split(":")[0]: int(line.split()[1]) for line in file}
    return found["MemAvailable"] // 1024


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=180)
    parser.add_argument("--interval", type=float, default=5)
    args = parser.parse_args()
    end = time.monotonic() + args.seconds
    before = (time.monotonic(), processes(), total_cpu(), network(), disk())
    while time.monotonic() < end:
        time.sleep(args.interval)
        now = (time.monotonic(), processes(), total_cpu(), network(), disk())
        span = now[0] - before[0]
        cores = {}
        for pid, (group, ticks) in now[1].items():
            earlier = before[1][pid][1] if pid in before[1] else 0
            cores[group] = cores.get(group, 0) + (ticks - earlier) / TICK / span
        total = now[2][0] - before[2][0]
        busy = 1 - (now[2][1] - before[2][1]) / max(total, 1)

        def rate(name, index):
            return round((now[3][name][index] - before[3][name][index]) * 8 / span / 1e6, 1) if name in now[3] else None

        print(
            json.dumps(
                {
                    "t": round(time.time()),
                    "busy_cores": round(busy * os.cpu_count(), 2),
                    "iowait_cores": round((now[2][2] - before[2][2]) / max(total, 1) * os.cpu_count(), 2),
                    "disk_busy": round((now[4][2] - before[4][2]) / 1000 / span, 2),
                    **{f"{g}_cores": round(c, 2) for g, c in sorted(cores.items()) if c >= 0.01},
                    "ffmpegs": sum(1 for group, _ in now[1].values() if group == "ffmpeg"),
                    "lan_in_mbit": rate("end0", 0),
                    "lan_out_mbit": rate("end0", 1),
                    "ts_out_mbit": rate("tailscale0", 1),
                    "disk_read_mbit": round((now[4][0] - before[4][0]) * 8 / span / 1e6, 1),
                    "disk_write_mbit": round((now[4][1] - before[4][1]) * 8 / span / 1e6, 1),
                    "mem_available_mib": memory(),
                }
            ),
            flush=True,
        )
        before = now


if __name__ == "__main__":
    main()
