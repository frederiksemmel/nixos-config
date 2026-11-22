def api_answers():
    code = tv.execute("curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8090/api/playback")[1].strip()
    return code not in ("", "000")


def core_said(line):
    return int(tv.succeed(f"journalctl -b -u semmeltv.service -o cat | grep -c {shlex.quote(line)} || true").strip())


with scenario("a TV running one process switches to the core and the tv as two units, and the API answers within 30 s"):
    assert tv.succeed("systemctl is-active semmeltv.service || true").strip() != "active", "the VM should boot as one process"
    assert api_answers(), "the single process serves before the switch"
    switched = time.monotonic()
    tv.succeed("/run/current-system/specialisation/split/bin/switch-to-configuration test >&2")
    eventually("the core's API answers", lambda: api_answers() or None, timeout=30, every=0.5)
    took = time.monotonic() - switched
    print(f"the API answered {took:.1f} s after the switch began")
    assert took < 30, f"the API answered only after {took:.1f} s"
    tv.wait_for_unit("semmeltv.service")
    core = tv.succeed("tr '\\0' ' ' < /proc/$(systemctl show -P MainPID semmeltv.service)/cmdline")
    assert " core" in core, core
    eventually("the tv connects to the core", lambda: core_said("tv: connected") or None, timeout=30)
    eventually("the core's start is told", lambda: core_said("start: serving after") or None, timeout=5)

with scenario("a core that can't take its port says why, and serves once it can"):
    tv.systemctl("stop semmeltv.service")
    tv.succeed("systemd-run --unit=hold-8090 $(command -v socat) -u TCP-LISTEN:8090,bind=127.0.0.1,reuseaddr,fork OPEN:/dev/null")
    tv.wait_for_open_port(8090)
    tv.succeed("systemctl start --no-block semmeltv.service")
    eventually(
        "the core says why it can't serve",
        lambda: core_said("the API can't listen: 127.0.0.1:8090") or None,
        timeout=30,
    )
    tv.systemctl("stop hold-8090")
    eventually("the core serves once the port is free", lambda: api_answers() or None, timeout=30, every=0.5)
    tv.wait_for_unit("semmeltv.service")
