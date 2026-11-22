import threading

NOT_SPLIT = "the core and the tv are not split into semmeltv.service and cage-tty1's `semmeltv tv`"


def tv_processes():
    return [int(pid) for pid in tv.succeed("pgrep -u semmeltv -f '^/nix/store/[^ ]*/bin/semmeltv tv$' || true").split()]


def split_tv():
    core = tv.succeed("systemctl is-active semmeltv.service || true").strip()
    core_line = tv.succeed("tr '\\0' ' ' < /proc/$(systemctl show -P MainPID semmeltv.service)/cmdline 2>/dev/null || true")
    tvs = tv_processes()
    assert core == "active" and " core" in core_line and len(tvs) == 1, (
        f"{NOT_SPLIT}: semmeltv.service is {core!r} running {core_line.strip()!r}, `semmeltv tv` in cage-tty1: {tvs}"
    )
    return tvs[0]


def a_new_tv_runs(before, timeout):
    eventually("a new tv process runs", lambda: [pid for pid in tv_processes() if pid != before], timeout=timeout, every=0.2)


def silent_tvs_ended():
    return int(tv.succeed("journalctl -b -u semmeltv.service -o cat | grep -c 'ending the silent tv' || true").strip())


class Probe:
    """Asks /api/playback five times a second from the test driver and keeps the slowest answer and every failure."""

    def __init__(self, cookie):
        self.cookie = cookie
        self.slowest = 0.0
        self.asked = 0
        self.failures: list[str] = []
        self.stopping = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stopping.is_set():
            asked = time.monotonic()
            request = urllib.request.Request(f"{PHONE}/api/playback", headers={"Cookie": f"semmeltv_session={self.cookie}"})
            try:
                with urllib.request.urlopen(request, timeout=1) as response:
                    response.read()
                    if response.status != 200:
                        self.failures.append(f"status {response.status}")
            except Exception as error:
                self.failures.append(f"{type(error).__name__}: {error}")
            took = time.monotonic() - asked
            self.slowest = max(self.slowest, took)
            self.asked += 1
            if took >= 1:
                self.failures.append(f"took {took:.2f} s")
            time.sleep(max(0, 0.2 - took))

    def stop(self):
        self.stopping.set()
        self.thread.join()
        print(f"/api/playback: {self.asked} answers, slowest {self.slowest:.2f} s, failures {self.failures[:5]}")
        assert self.asked > 10, f"the probe asked only {self.asked} times"
        assert not self.failures, f"/api/playback did not always answer within 1 s: {self.failures[:5]}"


def film_plays_at_100():
    phone.api("POST", "/api/play", {"movie": FILM})
    tv_starts_at()
    tv_seek(phone, 100)
    tv_plays_past(105)


def resumes_near(left_at, since, within):
    resumed = eventually("the film plays again", tv_position, timeout=max(0.5, since + within - time.monotonic()), every=0.2)
    print(f"left at {left_at:.1f} s, resumed at {resumed:.1f} s, {time.monotonic() - since:.1f} s later")
    assert abs(resumed - left_at) <= 5, f"left at {left_at:.1f} s, resumed at {resumed:.1f} s"
    eventually("the API shows the film again", lambda: (phone.api("GET", "/api/playback")["body"]["now_playing"] or {}).get("kind") == "movie")


PROBE_COOKIE = session_cookie("Frederik")

with scenario("a film resumes within 10 s where it was after the tv process is killed, and the API answers throughout"):
    before = split_tv()
    film_plays_at_100()
    probe = Probe(PROBE_COOKIE)
    CLEANUP.append(probe.stopping.set)
    time.sleep(2)
    left_at = tv_position() or 0
    tv.succeed(f"kill -9 {before}")
    killed = time.monotonic()
    a_new_tv_runs(before, 10)
    resumes_near(left_at, killed, 10)
    tv_plays_past(left_at + 2, timeout=10)
    time.sleep(2)
    probe.stop()
    CLEANUP.remove(probe.stopping.set)
    stop_everything(phone)

with scenario("a film plays on while the core restarts, and the restarted core takes it over"):
    split_tv()
    film_plays_at_100()
    tv.systemctl("stop semmeltv.service")
    stopped_at = tv_position() or 0
    tv_plays_past(stopped_at + 3, timeout=15)
    tv.systemctl("start semmeltv.service")
    tv.wait_for_open_port(8090)
    took_over = eventually(
        "the restarted core shows the film it took over",
        lambda: (lambda playing: playing if playing and playing.get("kind") == "movie" and playing["position"] > stopped_at else None)(
            phone.api("GET", "/api/playback")["body"]["now_playing"]
        ),
        timeout=30,
        every=0.5,
    )
    print(f"stopped the core at {stopped_at:.1f} s, the restarted one shows {took_over['position']:.1f} s")
    tv_plays_past(stopped_at + 6, timeout=15)
    stop_everything(phone)

with scenario("a frozen tv process leaves the API up, and the core ends it so the tv and the film come back"):
    before = split_tv()
    ended = silent_tvs_ended()
    film_plays_at_100()
    probe = Probe(PROBE_COOKIE)
    thaw = lambda: tv.execute(f"kill -CONT {before}")
    CLEANUP.extend([probe.stopping.set, thaw])
    time.sleep(2)
    left_at = tv_position() or 0
    tv.succeed(f"kill -STOP {before}")
    frozen = time.monotonic()
    eventually("the core ends the silent tv", lambda: silent_tvs_ended() > ended, timeout=30, every=1)
    restarted = time.monotonic()
    print(f"the core ended the frozen tv {restarted - frozen:.1f} s after the freeze")
    a_new_tv_runs(before, 10)
    resumes_near(left_at, restarted, 10)
    time.sleep(2)
    probe.stop()
    CLEANUP.remove(probe.stopping.set)
    CLEANUP.remove(thaw)
    stop_everything(phone)
