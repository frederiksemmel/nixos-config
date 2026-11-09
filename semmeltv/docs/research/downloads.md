# Downloading films and series on request: design proposal

Status: research, for Frederik to decide. No code or config exists for this yet.
Written 2026-11-09.

Goal: Frederik says "download Dune Part Two" to the TV, or types it into the web app.
SemmelTV then finds the best torrent, downloads it through Mullvad in a sandbox, and the
title appears in the library.

In this document:

- **Measured** means it was read on semmeltv5 or in this repo on 2026-11-09.
- **Unverified** means no primary source was found for the claim.
- Every other claim links to its source.

## Recommendation in one picture

```
 phone (web app text field) ─┐
 voice (wyoming → intent.rs) ─┤
                              ▼
          semmeltv app API (127.0.0.1:8090)            host network: kiosk, Tailscale,
          1. resolve request → TMDB id (+ IMDb id)      CEC/UPnP TV control, pCloud
          2. Torznab search by id / SxxEyy
          3. parse + score releases (no model)
          4. confirm on TV overlay or web page
          5. add magnet, poll progress
          6. hand finished file to the library
                              │  veth 192.168.15.1 (portMappings)
   ┌──────────────────────────┼──────────────────────────────┐
   │ netns "mullvad" (VPN-Confinement)                        │
   │   wg interface → Mullvad, default route only via wg      │
   │   Jackett   (Torznab, all indexers)                      │
   │   rqbit     (HTTP API, hardened systemd unit)            │
   │   writes only /var/lib/semmeltv/downloads                │
   └──────────────────────────────────────────────────────────┘
                              │ finished file
                              ▼
          rclone upload → pcloud:Incoming (phase 1) or straight to
          pcloud:SemmelTV/Movies|Series + import (phase 3)
```

- **Search:** Jackett, through its single Torznab "all indexers" endpoint. The app speaks
  only Torznab, so changing a source is a Jackett or Nix config change, never a code change.
- **VPN:** VPN-Confinement, a network namespace whose only route out is a WireGuard tunnel
  to Mullvad. The Mullvad config is a static wg-quick file kept in agenix. Not the Mullvad
  app.
- **Sandbox:** a hardened systemd service inside that namespace. Not a microVM. The Pi 5
  rules out cloud-hypervisor, and Firecracker cannot share a directory. A VM would cost
  RAM and buy protection only against kernel exploits.
- **Torrent client:** rqbit. It is written in Rust, its HTTP API is small, it has a
  hardened NixOS module, and it uses tens of MB of RAM. Transmission 4 is the fallback.
- **Picking a release:** a hand-written scorer that reuses the app's existing `tags.rs` and
  `parse.rs`. The request resolves to a TMDB id first, then the search is by that id, so a
  model adds nothing. Details are in section 5.

## 1. Search source

### RARBG today

- **RARBG is gone.** It shut down in 2023 and has no official successor. Its torrentapi
  is gone too, and the remaining "RARBG API" wrappers scrape clones
  ([RarbgAPI on PyPI](https://pypi.org/project/RarbgAPI)).
- **TheRARBG** (therarbg.to / .com) is an unofficial clone and is alive.
  - Jackett ships a definition for it with `movie-search: [q, imdbid]` and
    `tv-search: [q, season, ep]`
    ([therarbg.yml](https://github.com/Jackett/Jackett/blob/master/src/Jackett.Common/Definitions/therarbg.yml)).
  - It also has an undocumented JSON endpoint, which a probe on 2026-11-09 answered. It
    returns name, size, seeders, leechers, infohash, IMDb id and tags.
  - Clones come and go. So the design must not depend on any single one.

### Aggregators

Both aggregators speak Torznab: `GET …/api?t=search|movie|tvsearch&q=&imdbid=&season=&ep=&cat=`.
Results come back as RSS with `torznab:attr` fields for `seeders`, `infohash`, `magneturl`
and `size` ([Torznab spec](https://torznab.github.io/spec-1.3-draft/torznab/Specification-v1.3.html)).

| | Jackett | Prowlarr |
|---|---|---|
| One endpoint over many indexers | `/api/v2.0/indexers/all/results/torznab/api`, or a tag filter such as `indexers/tag:semmeltv` ([README](https://github.com/Jackett/Jackett#api-usage)) | Aggregated search is its own JSON API (`/api/v1/search`). Torznab is per indexer ([SearchController.cs](https://github.com/Prowlarr/Prowlarr/blob/develop/src/Prowlarr.Api.V1/Search/SearchController.cs)) |
| ID search | `imdbid`, `tmdbid`, `season`, `ep` params | `{ImdbId:tt…}` tokens in `query` |
| nixpkgs | `jackett` + `services.jackett` ([module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/misc/jackett.nix)) | `prowlarr` + `services.prowlarr` ([module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/misc/servarr/prowlarr.nix)) |
| aarch64-linux | in `meta.platforms`, cached on cache.nixos.org | evaluates, cached |
| Runtime | .NET 9 | .NET 8 |
| RAM | not documented (unverified) | not documented (unverified) |

**Recommendation: Jackett.** One Torznab URL covers every indexer, and the app needs
nothing else. Prowlarr's strength is syncing indexers into Sonarr and Radarr, which this
design doesn't use.

There is one caveat. Jackett stores its indexer choices and its API key in its own state
directory, which is set through its web UI, not through Nix. That has to be measured or
checked during phase 0:

- **RAM:** whether .NET's RAM use on the Pi is acceptable.
- **16K pages:** whether .NET runs on the Pi's 16K-page kernel. This is unverified; no
  source was found either way.

**Which public indexers honour an id**, read from their Jackett definitions:

- **By IMDb id:** TheRARBG and YTS ([yts.yml](https://github.com/Jackett/Jackett/blob/master/src/Jackett.Common/Definitions/yts.yml)).
  YTS is movies only, and mostly small encodes.
- **Text only, with `season`/`ep`:** EZTV, The Pirate Bay (apibay), Knaben and TorrentsCSV.
- **Need FlareSolverr:** 1337x, EZTV and Uindex carry `info_flaresolverr`. FlareSolverr
  starts a Chrome per request and is memory-heavy
  ([FlareSolverr](https://github.com/FlareSolverr/FlareSolverr)). Skip those indexers
  rather than run Chrome on the TV.

**Later option: bitmagnet.** bitmagnet is a self-hosted DHT crawler with a Torznab endpoint
and IMDb/TMDB id support ([repo](https://github.com/bitmagnet-io/bitmagnet),
[servarr guide](https://bitmagnet.io/guides/servarr-integration.html)). It is packaged as
`services.bitmagnet`. It needs Postgres and a long crawl, which is too heavy for the TV Pi.
It is the answer if public indexers dry up, perhaps run on another host.

### Indexer-agnostic design

- **The app only knows Torznab.** A Nix option holds a list of Torznab endpoints, for
  example `services.semmeltv.downloads.torznab = [ "http://192.168.15.1:9117/api/v2.0/indexers/all/results/torznab/api" ];`.
  The app queries all of them, merges the results by infohash, and ranks them itself.
- **Sources are config.** Adding or dropping a source happens in Jackett's indexer list or
  in that option.
- **The app never scrapes a site.** Any code that knows about a particular site stays out
  of it.

## 2. VPN: Mullvad for the downloader only

### Static WireGuard config vs the Mullvad app

| | Static WireGuard + VPN-Confinement | Mullvad app (`services.mullvad-vpn`) |
|---|---|---|
| Scope | Only the services placed in the namespace | Whole host. Its split tunnelling only *excludes* apps, and Mullvad says "inverse split tunneling" is not supported ([FAQ](https://mullvad.net/en/help/faq)) |
| Kill switch | The namespace has no route except through wg, and new connections out of the veth are dropped ([vpn-up.nix](https://github.com/Maroka-chan/VPN-Confinement/blob/master/modules/vpn-up.nix)) | System-wide and always on ("cannot be disabled", [FAQ](https://mullvad.net/en/help/faq)), so it would capture Tailscale and the kiosk |
| Host side effects | A bridge, a veth, iptables-nft NAT rules and `net.ipv4.ip_forward=1` ([vpn-netns.nix](https://github.com/Maroka-chan/VPN-Confinement/blob/master/modules/vpn-netns.nix)) | Turns on `services.resolved` by default ([module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/networking/mullvad-vpn.nix)), and its firewall can drop Tailscale traffic ([Tailscale KB](https://tailscale.com/kb/1105/other-vpns)) |
| Key rotation | Manual. No documented expiry for keys registered by hand (unverified) | Automatic, weekly ([blog](https://mullvad.net/en/blog/introducing-names-for-easier-device-management)) |
| DAITA / QUIC obfuscation | Not available (community consensus, unverified) | Available |

**Recommendation: VPN-Confinement** ([repo](https://github.com/Maroka-chan/VPN-Confinement),
active with commits in 2026-10).

How it works:

- It creates the wg interface in the host namespace and moves it into the namespace. The
  encrypted UDP socket therefore stays on the host LAN, the pattern from
  [wireguard.com/netns](https://www.wireguard.com/netns/).
- A service joins the namespace with
  `systemd.services.X.vpnConfinement = { enable = true; vpnNamespace = "mullvad"; }`
  ([systemd.nix](https://github.com/Maroka-chan/VPN-Confinement/blob/master/modules/systemd.nix)).
  The service then `bindsTo` the namespace unit, so if the tunnel unit stops, the client
  stops with it.
- DNS inside the namespace goes only to the `DNS =` server in the config, which is
  Mullvad's 10.64.0.1 inside the tunnel. A `dns-leak` chain drops port 53 on the veth.
- IPv6 has its own ip6tables rules and its own v6 default route into wg.

Caveats:

- Only plain UDP DNS has been leak-tested (per the README).
- The namespace unit has no `Restart=`. Its up script pings the endpoint and fails if the
  network isn't up yet. This fails closed (nothing downloads), but the unit needs a restart
  policy or a timer.
- **Tailscale is unverified.** A network namespace has its own routes and firewall
  ([network_namespaces(7)](https://man7.org/linux/man-pages/man7/network_namespaces.7.html)),
  so the kiosk, Tailscale, CEC and UPnP stay on `end0` as they are now. But nobody has
  tested whether VPN-Confinement's host NAT rules and `ip_forward` interfere with
  Tailscale's own nftables rules. Phase 1 checks this: `tailscale status` and Funnel must
  still work, and `curl ifconfig.me` on the host must still show the home IP.

**Mullvad changes that matter:**

- **No port forwarding.** It ended in 2023
  ([blog](https://mullvad.net/en/blog/2023/5/29/removing-the-support-for-forwarded-ports)).
  So no peer can connect in, and the client can only reach connectable peers. Popular
  torrents still download fine. Rare ones are slower.
- **OpenVPN is gone** as of 2026-01-15
  ([blog](https://mullvad.net/en/blog/2024/11/8/removing-openvpn-15th-january-2026)).
  WireGuard is the only option anyway.
- **One device slot.** A static key uses one of the account's 5 device slots
  ([FAQ](https://mullvad.net/en/help/faq)).

### Secret

**`secrets/mullvad_wireguard.age`**, encrypted for `frederik` and `semmeltv5`:

- **Content:** a complete wg-quick file. VPN-Confinement's `wireguardConfigFile` expects
  exactly this, and the `DNS` line is required:

  ```
  [Interface]
  PrivateKey = …
  Address = 10.x.x.x/32, fc00:bbbb:bbbb:bb01::x/128
  DNS = 10.64.0.1
  [Peer]
  PublicKey = …
  AllowedIPs = 0.0.0.0/0, ::/0
  Endpoint = <relay ip>:51820
  ```

- **How to make it:** run Mullvad's own script on beast
  ([mullvad-wg.sh](https://github.com/mullvad/mullvad-wg.sh/blob/main/mullvad-wg.sh)), or
  use the website's [config generator](https://mullvad.net/en/help/easy-wireguard-mullvad-setup-linux).
  Pick one Swiss or nearby relay. Then pipe the file into agenix with
  `agenix -e mullvad_wireguard.age`.
- **The account number never goes on the Pi.** It is only needed to register or revoke the
  key. If Frederik wants it in the repo at all, it goes in `secrets/mullvad_account.age`,
  encrypted for `frederik` only.

## 3. Sandbox

The threat model is the torrent client parsing untrusted input: peers, trackers, torrent
metadata and file names. Historic bugs are userland: path traversal
([CVE-2010-0012](https://nvd.nist.gov/vuln/detail/CVE-2010-0012)), RPC DNS rebinding
([CVE-2018-5702](https://nvd.nist.gov/vuln/detail/CVE-2018-5702)), command injection from a
torrent name ([CVE-2019-13640](https://nvd.nist.gov/vuln/detail/CVE-2019-13640)), and TLS not
validated ([CVE-2024-51774](https://nvd.nist.gov/vuln/detail/CVE-2024-51774)). They are not
kernel escapes.

**Pi 5 facts that decide this** (measured on semmeltv5 unless linked):

- `/dev/kvm` exists and the kernel has `CONFIG_KVM=y`. The kernel is 6.18.34.
- The kernel uses **16K pages** (`CONFIG_ARM64_16K_PAGES=y`).
- The interrupt controller is a GIC-400, which is GICv2
  ([bcm2712.dtsi](https://github.com/raspberrypi/linux/blob/rpi-6.18.y/arch/arm64/boot/dts/broadcom/bcm2712.dtsi)).
  KVM only offers a guest GICv3 on a GICv3 host
  ([kernel docs](https://docs.kernel.org/virt/kvm/devices/arm-vgic-v3.html)).
- The kernel cmdline has **`cgroup_disable=memory`**. It comes from the Pi DTB bootargs
  ([bcm2712-rpi.dtsi](https://github.com/raspberrypi/linux/blob/rpi-6.18.y/arch/arm64/boot/dts/broadcom/bcm2712-rpi.dtsi)),
  and `/sys/fs/cgroup/cgroup.controllers` lists no `memory` controller. So systemd
  `MemoryMax=` is silently not enforced. The Pi kernel's `cgroup_enable=memory` turns it
  back on ([raspberrypi/linux#6980](https://github.com/raspberrypi/linux/issues/6980)).
- Memory is 7.8 GiB in total and about 4.2 GiB available while a video plays. There is no
  swap.

### microVM hypervisors on this Pi

[microvm.nix](https://github.com/microvm-nix/microvm.nix) is maintained, with commits in
2026-10. Which of its hypervisors work here:

| Hypervisor | Works on this Pi? | Can share the download directory? |
|---|---|---|
| cloud-hypervisor | **No.** It needs GICv3 and fails on a Pi with `CreateGic … No such device` ([README](https://github.com/cloud-hypervisor/cloud-hypervisor#prerequisites-for-aarch64), [#6524](https://github.com/cloud-hypervisor/cloud-hypervisor/issues/6524)) | |
| firecracker | Yes, it falls back to GICv2 ([gic/mod.rs](https://github.com/firecracker-microvm/firecracker/blob/main/src/vmm/src/arch/aarch64/gic/mod.rs)). A 16K-page bug was fixed in 2025-01 ([#4905](https://github.com/firecracker-microvm/firecracker/issues/4905)) | **No.** microvm.nix gives it neither 9p nor virtiofs, so downloads would have to go to a block image |
| qemu | Probably. microvm.nix passes `gic-version=max` ([qemu.nix](https://github.com/microvm-nix/microvm.nix/blob/main/lib/runners/qemu.nix)). No report of it running on a 16K Pi 5 was found (unverified) | Yes, virtiofs or 9p |
| crosvm, kvmtool, alioth, stratovirt | All have a GICv2 path in their source. Not reported on a Pi 5 (unverified) | Varies |

### The three options compared

| | microVM (qemu + virtiofs) | NixOS container (nspawn) | systemd unit in netns + hardening |
|---|---|---|---|
| Isolation | Own kernel. Survives a kernel exploit | Shares the kernel. The manual warns root in the container can affect the host ([manual](https://github.com/NixOS/nixpkgs/blob/nixos-unstable/nixos/doc/manual/administration/containers.chapter.md)). With `privateUsers = "pick"` it gets a user namespace | Shares the kernel. `DynamicUser`, `ProtectSystem=strict`, one `ReadWritePaths`, `PrivateUsers`, `SystemCallFilter=@system-service`, empty `CapabilityBoundingSet`, `NetworkNamespacePath` ([systemd.exec](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html)) |
| VPN plumbing | wg in the guest, or tap into a host netns. Both are custom work | Move the wg interface into the container (`networking.wireguard.interfaces.<n>.interfaceNamespace`) | VPN-Confinement does it all |
| RAM | 512 MB guest by default, fixed ([README](https://github.com/microvm-nix/microvm.nix)). It is the only option with a hard memory cap, given `cgroup_disable=memory` | Small | Smallest |
| Complexity | High. Unproven on a 16K Pi 5, a second NixOS system to build, and virtiofsd | Medium. A second NixOS config | Low. Two module options |
| Files reach the app | virtiofs share, read-only on the app side | `bindMounts.<p>.isReadOnly` | A group-readable directory. The app only reads it |

**Recommendation: the systemd unit.** The nixpkgs `services.rqbit` module is already
hardened. VPN-Confinement adds the namespace. The download directory is
`/var/lib/semmeltv/downloads`, writable only by the client. The `semmeltv` user and the
upload job get read access through a group, and nothing on the host executes downloaded
files.

Two things are worth doing on top:

- Re-enable the memory cgroup (`cgroup_enable=memory` on the cmdline) and give the client
  `MemoryMax`. Without it, nothing stops a misbehaving client from pushing the kiosk into
  OOM.
- Check `systemd-analyze security rqbit`.

A microVM can come later if Frederik wants kernel-exploit isolation. If so, qemu is the
only candidate, and it has to be proven on this kernel first.

## 4. Torrent client

| | rqbit | Transmission 4.1 | qBittorrent-nox 5.3 |
|---|---|---|---|
| Language | Rust (memory-safe) | C++ (libtransmission) | C++ (libtorrent 2.1) |
| API | Plain HTTP: `POST /torrents` (magnet, `output_folder`, `only_files_regex`), `GET /torrents/{id}/stats/v1` (progress, `time_remaining`), `pause`/`forget`/`delete` ([README](https://github.com/ikatson/rqbit)). Basic auth via env | JSON-RPC 2.0: `torrent_add` (`download_dir`, `files_unwanted`), `torrent_get` (`percent_done`, `eta`), CSRF header + host whitelist ([rpc-spec](https://github.com/transmission/transmission/blob/main/docs/rpc-spec.md)) | WebUI API v2, cookie or API key, `torrents/add`, `torrents/info`, `filePrio`, share limits ([wiki](https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-5.0))) |
| Rust client | Plain `ureq`, or embed [librqbit](https://crates.io/crates/librqbit) | [transmission-rpc](https://crates.io/crates/transmission-rpc) 0.5 (2025-02, predates 4.1's JSON-RPC 2.0) | [qbit-rs](https://crates.io/crates/qbit-rs) 0.6 |
| NixOS module | `services.rqbit` (new in 26.05, hardened, `httpHost`/`httpPort`/`downloadDir`, [module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/torrent/rqbit.nix)) | `services.transmission` (strongest hardening: `RootDirectory`, empty caps, `MemoryDenyWriteExecute`, [module](https://github.com/NixOS/nixpkgs/blob/nixos-unstable/nixos/modules/services/torrent/transmission.nix)) | `services.qbittorrent` (new in 25.11, lighter hardening, [module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/torrent/qbittorrent.nix)) |
| aarch64 | yes | yes | yes |
| RAM | "usually within a few tens of megabytes" (README) | "one of the lowest" footprints, no figures ([site](https://transmissionbt.com/)) | No Pi figures. libtorrent 2 uses mmap I/O |
| Gaps | No MSE encryption ([#617](https://github.com/ikatson/rqbit/issues/617)). uTP off by default. Sequential download only. No seeding limits, so the app pauses or forgets the torrent itself. 9.1.0 (2026-10-29) fixed several remote peer crashes | Rust crate lags the protocol | Track record of WebUI security bugs, the latest an auth bypass fixed in 5.3.0 ([news](https://www.qbittorrent.org/news)) |

**Recommendation: rqbit.**

- It is memory-safe while parsing peer input, it is the smallest, and it has the simplest
  API.
- Its gaps don't matter here. Encryption is moot inside a VPN tunnel. Seeding limits
  become one `pause` call from the app.
- It also has a `/stream/{file}` endpoint, which could later allow "start watching while it
  downloads". That is out of scope here.

**Fallback: Transmission 4.** The app should talk to the client through a small trait
(add, status, pause, remove), so switching clients costs one file.

## 5. Picking the best release

### Resolve first, then search narrowly

1. **Request → TMDB.** The app already has `search_movie`, `search_tv`, `details` and
   `season` in `app/src/tmdb.rs`, and voice already turns "download the next season of
   Outlander" into `Action::Download { title, season }` (`app/src/intent.rs`). A request
   that matches several TMDB titles becomes a question ("Dune (2021) or Dune: Part Two
   (2024)?"), like the existing "Did you mean A or B?" refusal.
2. **TMDB → IMDb id.** This uses TMDB's `external_ids`, which `tmdb.rs` does not fetch yet.
3. **Search.**
   - A movie searches `t=movie&imdbid=tt…`, plus `t=search&q=<title> <year>` for indexers
     that only take text.
   - A season searches `t=tvsearch&q=<title>&season=N`. Season packs are preferred.
   - An episode searches `t=tvsearch&q=<title>&season=N&ep=M`.

With an exact id and the parsed fields checked against TMDB, the remaining choice is a
quality and availability trade-off with no language in it. So a model has nothing to
decide. Matching title and year against TMDB is the job `semmeltv import` already does
today.

### The scorer

It reuses `parse::parse_release` (title and year) and `tags.rs` (codec, resolution, 10-bit,
audio), which already parse real release names. SxxEyy parsing exists in `parse.rs` for
episodes. If more fields are wanted (source such as CAM/TS/WEB-DL/BluRay, group, HDR,
"sample"), there are two options:

- **Extend the app's own parser.** Preferred: it is already tested against Frederik's
  library.
- **Adopt a crate.** [`parse-torrent-title`](https://github.com/TheBeastLT/parse-torrent-title)
  is a Rust rewrite of the maintained JS parser, but very new on crates.io.
  [`hunch`](https://github.com/lijunzh/hunch) is guessit-inspired and small.
  `torrent-name-parser` is stale since 2023.

**Hard filters** (a release is dropped):

- The parsed title doesn't match the TMDB title or one of its alternative titles, after
  `library::normalize`.
- The year is off by more than one.
- The season or episode doesn't match.
- The source is CAM, TS, TC or SCR. Samples and trailers are dropped too.
- It has fewer than 3 seeders.
- The size is outside sane bounds for the TMDB runtime: under about 8 MB per minute for
  1080p is suspicious, and anything over the free-space quota is impossible.

**Score** (higher is better; the weights are a starting point to tune):

| Term | Values |
|---|---|
| Resolution | 1080p 1.0 · 720p 0.6 · 2160p 0.3 (needless size and decode load on this TV path) · lower 0.1 |
| Codec | HEVC 1.0 (hardware-decoded on the Pi 5) · AV1 0.4 · H.264 0.3 (software decode costs 40–89% of a core, as measured earlier on semmeltv5) · other 0 |
| Seeders | `min(1, log10(1 + seeders) / 2)`, saturating at about 100 seeders |
| Size fit | Peaks near about 25–40 MB per minute of TMDB runtime for 1080p HEVC, and falls off both ways |
| Extras | +small for 5.1 audio and for a BluRay or WEB-DL source. Ties go to more seeders |

The top three candidates are shown with their facts ("1080p HEVC · 5.1 · 6.2 GB · 412
seeders"). The top one is preselected, and one confirmation starts it.

### A model?

**Not needed for picking.** If a model were added anyway, here is what's realistic on this
Pi:

- **Embedding models and small rerankers** can run on the CPU through
  [fastembed-rs](https://github.com/Anush008/fastembed-rs). Examples are all-MiniLM-L6-v2
  (23M parameters), bge-small (33M), jina-reranker-v1-tiny (33M) and bge-reranker-base
  (278M). No Pi 5 latency figures were found, so it would have to be measured.
- **Small LLMs** (Gemma 3 270M/1B, Qwen3 0.6B, Llama 3.2 1B) run through llama.cpp. For
  scale, a Pi 5 runs Llama 3.2 **3B** at about 4.6 tokens/s
  ([geerlingguy/ai-benchmarks](https://github.com/geerlingguy/ai-benchmarks)).
- **Where a model could help** is the request side ("that new film with the sandworms"),
  where TMDB's text search fails. That is a different feature.

**Hailo-8L:**

- **No LLMs.** The AI HAT+ (Hailo-8/8L) lists LLM/VLM support as "Not supported"; it needs
  the Hailo-10H ([Raspberry Pi docs](https://www.raspberrypi.com/documentation/accessories/ai-hat-plus.html),
  [hailo_model_zoo_genai](https://github.com/hailo-ai/hailo_model_zoo_genai)).
- **One embedding model.** The Model Zoo does have all-MiniLM-L6-v2 compiled for hailo8l
  ([table](https://github.com/hailo-ai/hailo_model_zoo/blob/master/docs/public_models/HAILO8L/HAILO8L_sentence_embedding_generation.rst)),
  but no reranker.
- **Not worth the plumbing for this.** It is irrelevant unless a model is wanted at all.

**"Jev".** It is unclear which model Frederik meant. These are candidates, not a guess:

- **Gemma / EmbeddingGemma:** Google's small LLMs and a 303M embedding model.
- **Jina:** jina-embeddings and jina-reranker. Note that v2 and v3 of the reranker are
  CC-BY-NC.
- **GTE or BGE**, spoken as letters: embedding models and rerankers.
- **GLiNER:** small entity extraction, which could pull title, year and quality out of
  text. It has a Rust port, gline-rs.
- **GEPA:** a prompt optimiser.
- **JEPA:** Meta's video models.
- **GGUF/GGML:** the llama.cpp format.
- **Jan-nano:** a 4B search-tuned model.

## 6. Entry points and request flow

### From request to library

1. **Input.** Two ways in:
   - **Web:** a text field on a new "Download" page in `semmeltv/web`, posting to
     `POST /api/downloads/search`.
   - **Voice:** the existing path is wyoming → `voice.rs` → `intent.rs`, which ends in
     `Action::Download`. `app.rs` currently answers "(not set up yet)". It would instead
     start the same search.
2. **Resolve.** TMDB id, IMDb id, season and episode (section 5). Ambiguity becomes a
   question on whichever screen asked.
3. **Search and pick.** The Torznab endpoints are searched, then the results are filtered
   and scored.
4. **Confirm.**
   - **Voice:** a TV overlay with the chosen release and two alternatives, accepted with the
     remote's OK or "yes".
   - **Web:** the same three cards with a "Download" button.
   - Nothing downloads without a confirmation.
5. **Download.** `POST /torrents` to rqbit with the magnet and `output_folder`. For a
   season pack, `only_files_regex` skips extras.
6. **Progress.** `GET /api/downloads` on the app returns percent, speed, ETA and state. The
   web page polls it or follows the app's existing WebSocket event stream (`web/src/live.rs`). The TV can show
   a quiet line.
7. **Finish.** When rqbit reports `finished`, the app pauses or forgets the torrent, so it
   stops seeding (see risks). Then the hand-off to the library happens.
8. **Into the library.** `/var/lib/semmeltv/library` is a **read-only rclone mount of
   pCloud**, and `library.toml` is synced *from* pCloud every 15 minutes
   (`semmeltv/hosts/semmeltv5/mount.nix`). So the file has to go to pCloud:
   - **Phase 1:** `rclone copy` it to `pcloud:Incoming` (normal network, not the VPN), check
     it with `rclone check`, then delete it locally. The existing `/semmeltv-incoming` skill
     on beast sorts and imports it.
   - **Phase 3:** the app knows the TMDB identity, so it can upload straight to the
     canonical path (`Movies/Title (Year)/Title (Year).ext`), run `semmeltv import` and
     `semmeltv artwork` on a local copy of `library.toml`, and upload artwork, then the
     manifest, as the skill does.
   - Phase 3 needs the TMDB token and the MDBList key on semmeltv5. Today
     `secrets/tmdb_token.age` and `mdblist_api_key.age` are encrypted for `frederik` only.
     It also makes the Pi a second writer of `library.toml` besides beast sessions, so the
     two need a lock or a compare-before-upload step.
   - Uploading and then playing means the bytes cross the line twice: up to pCloud, then
     back into the VFS cache. On ethernet that's acceptable for a start.

### Disk on semmeltv5 (measured 2026-11-09)

- **Root filesystem:** the 117 GB SD card (`/dev/mmcblk0p2`) has 61 GB used and 52 GB
  free.
- **rclone VFS cache:** 42 GB today, capped at `offlineCacheGiB = 80`, with
  `--vfs-cache-min-free-space 5G`. So the cache alone may grow to fill about all the free
  space.
- **Download sizes:** a 1080p HEVC film is about 2–8 GB, and a season about 5–25 GB.
- **Proposal:** a downloads quota (for example 30 GiB) checked before starting, and
  deletion right after upload. Lower `offlineCacheGiB` to about 60, or downloads and the
  cache will fight.
- **SD card wear:** torrent writes are random and sustained, which is the worst load for
  an SD card, both for wear and for slowing the kiosk while writing. A USB 3 SSD for
  `/var/lib/semmeltv/downloads` (and maybe the VFS cache) would remove the problem. The
  PCIe slot is taken by the Hailo-8L.

### Secrets the design needs

| File | Recipients | Content |
|---|---|---|
| `secrets/mullvad_wireguard.age` | frederik, semmeltv5 | wg-quick config (section 2) |
| `secrets/rqbit_http_auth.age` | semmeltv5 | `RQBIT_HTTP_BASIC_AUTH_USERPASS=user:pass` env file. Optional, since the API only listens on the namespace veth |
| `secrets/tmdb_token.age` | add semmeltv5 | Already exists. Needed from phase 2 |
| `secrets/mdblist_api_key.age` | add semmeltv5 | Already exists. Only for the phase 3 auto-import |
| Jackett API key | — | Lives in Jackett's state, `/var/lib/jackett`. Whether the module can set it declaratively is unverified; check in phase 0 |

## 7. Build plan

- **Phase 0, checks on the Pi (no app code):**
  - Add VPN-Confinement and a `mullvad` namespace, then confirm on semmeltv5:
    - The namespace's public IP is a Mullvad IP.
    - `ip netns exec mullvad curl` fails when wg is down.
    - Tailscale, Funnel, the kiosk and the TV volume still work.
  - Run Jackett in the namespace and measure its RSS, confirming .NET runs on the 16K
    kernel.
  - Decide on `cgroup_enable=memory` and on a USB SSD.
- **Phase 1, manual pipeline:**
  - rqbit in the namespace.
  - A `semmeltv-downloads-upload` unit that moves finished files to `pcloud:Incoming`.
  - Downloads are started by hand with a magnet, through `curl` to rqbit.
- **Phase 2, the app picks:**
  - Torznab client, `external_ids`, the scorer with unit tests on real release names,
    and the `/api/downloads` endpoints.
  - The web page with search, three cards and progress.
  - The voice `Action::Download` hooked up to the TV confirm overlay.
- **Phase 3, automatic import:**
  - Canonical upload path, import and artwork on the Pi, a lock against beast runs, and
    `semmeltv-sync` triggered immediately.

## 8. Risks and open questions

**Risks:**

- **Legal.** Downloading copyrighted films is illegal in many places. Uploading (seeding)
  is what's usually prosecuted. The VPN hides the IP, but stopping right after the
  download (no seeding) lowers both the exposure and the share ratio. Public trackers don't
  enforce ratio.
- **Malicious payloads.** Fake releases (`.exe`, `.lnk`, password-protected `.rar`) exist.
  The library only imports video extensions, and nothing executes downloaded files. The
  scorer should drop releases whose file list has no video, which rqbit can show with
  `list_only` before downloading.
- **Clones and indexers disappear.** That is handled by Torznab and config (section 1).
- **SD card.** See section 6.
- **Unverified items:** .NET on the 16K-page kernel; VPN-Confinement next to Tailscale's
  nftables; qemu microVMs on this Pi.

**Questions for Frederik:**

1. "Jev": which of Gemma, Jina, GTE/BGE, GLiNER, or something else? Is it worth a model at
   all, given resolve-then-search?
2. Seed after downloading (better for the swarm, more exposure), or stop at 100%?
3. Should finished downloads go to `pcloud:Incoming` for the beast skill (phase 1), or be
   imported automatically from the Pi (phase 3, needs TMDB and MDBList keys on the Pi)?
4. Is a USB SSD for downloads OK, or should everything stay on the SD card with a quota?
5. Mullvad: create the WireGuard config (one of the 5 device slots) with the website
   generator, then hand over the `.conf` for agenix. The account number is not needed on
   the Pi.
