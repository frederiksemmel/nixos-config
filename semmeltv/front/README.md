# SemmelTV front

The front is the one door to a TV's web app and API: `tailscale serve` and
the LAN name's nginx both send everything to it on 127.0.0.1:8093. It passes
`/api` to the core on 8090 and the rest to the web app on 8091, bodies,
`Range` requests and WebSocket upgrades included, with the visitor's headers
unchanged. It is its own small unit, `semmeltv-front`, so it stays up while
the core and the web app restart.

While SemmelTV can't serve, the front answers for it with the bakery's
closed screen and a guess of when it is back:

- a page navigation gets the screen as a page (503), which polls
  `/api/maintenance` and reloads itself once SemmelTV is back;
- an API call or a web app fetch (`X-Topcoat-Runtime`) gets a 503 with
  `Retry-After`, an `X-SemmelTV-Maintenance` header naming the cause and the
  same status as `/api/maintenance` as JSON;
- the web app's scripts, styles and live socket still reach the web app.

`/api/maintenance` (the status), `/api/maintenance/screen` (the screen, for
an iframe; it posts `{ semmeltv: "open" }` to its parent once SemmelTV is
back) and the screen's two pictures are the front's own.

There are three causes:

- **planned**: `semmeltv-maintenance on 20m "new streaming limits"` closes
  SemmelTV until `semmeltv-maintenance off`, guessing the given duration.
  `status` tells what the front sees. Planned work survives a restart of the
  front.
- **restarting**: the core's unit runs `semmeltv-maintenance restarting` as
  its `ExecStop`, so every deploy, restart and stop announces itself before
  the core stops answering. Requests still pass while the core answers.
- **down**: the core refuses a connection, or doesn't answer three probes
  in a row (one every 5 s, every second while it is away).

A restart or outage is guessed at the 80th percentile of the last 20,
measured from the announcement or the first failure to the core's first
answer and kept in `/var/lib/semmeltv-front/bakery.json`; a minute until
there are any. Once a guess runs out, the screen says "Dauert länger als
gedacht" and guesses again, half the first guess (at least 30 s) further on
each time.

The control port on 127.0.0.1:8094 is reachable by every local user: `GET
/status`, `PUT /planned` with `{"seconds": 1200, "reason": "…"}`, `DELETE
/planned` and `POST /restarting`.

    cargo test
