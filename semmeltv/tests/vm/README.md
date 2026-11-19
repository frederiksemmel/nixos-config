# SemmelTV VM test

`checks.x86_64-linux.semmeltv-vm` boots one NixOS VM with the real kiosk module:
cage on tty1 with the Slint app and mpv, the API, and the web app behind nginx on
port 8080. A generated library holds a short series with sidecar subtitles and a
film with an embedded subtitle track. The test drives the TV with its keyboard,
reads its screen with OCR and asks mpv over its IPC socket where it is. The
phone is Playwright in the test driver: Chromium as an Android phone and a
laptop, and WebKit as an iPhone where no video has to play, since Playwright's
WebKit build has no H.264 or AAC decoder. WebKit drops `Secure` cookies on plain
HTTP, so a phone is signed in by redeeming an invite and handing the browser the
session cookie.

    nix build -L .#checks.x86_64-linux.semmeltv-vm

Screenshots of the TV and the phone land in the result.

Left to the cargo tests, because they need the Pi's hardware or the internet:
hardware decoding (drm, v4l2m2m), HDMI-CEC power, input and remote, HDMI display
modes, Bluetooth headphones, Hailo voice and the wake word, the Samsung UPnP
volume, the pCloud mount and the offline cache, downloads (Jackett, rqbit,
Mullvad), and Tailscale, TLS and deSEC.

`checks.x86_64-linux.semmeltv-no-upload` runs the TV's rqbit, built and
configured by `modules/rqbit.nix`, between a seeder and a third peer that asks
it for pieces mid-download, and fails if the TV uploads a single byte. The
seeder runs the same build without `RQBIT_DISABLE_UPLOAD` and must upload.
