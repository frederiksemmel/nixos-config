import json
import subprocess
import urllib.error
import urllib.request

from timezonefinder import TimezoneFinder

BEACONDB = "https://api.beacondb.net/v1/geolocate"
IWD = "net.connman.iwd"


def busctl(*args):
    out = subprocess.run(
        ["busctl", "--system", "--json=short", *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)["data"]


def access_points():
    objects = busctl("call", IWD, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects")[0]
    aps = {}
    for path, interfaces in objects.items():
        if f"{IWD}.Station" not in interfaces:
            continue
        for network, signal in busctl("call", IWD, path, f"{IWD}.Station", "GetOrderedNetworks")[0]:
            ess = objects.get(network, {}).get(f"{IWD}.Network", {}).get("ExtendedServiceSet")
            for bss in ess["data"] if ess else []:
                address = objects[bss][f"{IWD}.BasicServiceSet"]["Address"]["data"]
                aps[address] = {"macAddress": address, "signalStrength": signal // 100}
    return list(aps.values())


def vpn_connected():
    status = subprocess.run(["mullvad", "status"], capture_output=True, text=True).stdout
    return not status.startswith("Disconnected")


def locate(aps, use_ip):
    body = {"considerIp": use_ip, "fallbacks": {"ipf": use_ip, "lacf": False}}
    if len(aps) >= 2:
        body["wifiAccessPoints"] = aps
    request = urllib.request.Request(
        BEACONDB,
        json.dumps(body).encode(),
        {"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        raise


def main():
    aps = access_points()
    use_ip = not vpn_connected()
    if len(aps) < 2 and not use_ip:
        print(f"{len(aps)} access points and the VPN is up, keeping the timezone")
        return
    try:
        result = locate(aps, use_ip)
    except urllib.error.URLError as e:
        print(f"BeaconDB unreachable ({e.reason}), keeping the timezone")
        return
    if result is None:
        print(f"BeaconDB knows none of {len(aps)} access points, keeping the timezone")
        return

    location = result["location"]
    source = "IP" if result.get("fallback") == "ipf" else f"{len(aps)} access points"
    timezone = TimezoneFinder().timezone_at(lat=location["lat"], lng=location["lng"])
    if timezone is None:
        print(f"No timezone at {location}, keeping the timezone")
        return

    current = subprocess.run(
        ["timedatectl", "show", "--property=Timezone", "--value"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if timezone == current:
        print(f"Timezone stays {timezone} (from {source}, ±{result['accuracy']:.0f} m)")
        return
    subprocess.run(["timedatectl", "set-timezone", timezone], check=True)
    print(f"Set timezone {current} -> {timezone} (from {source}, ±{result['accuracy']:.0f} m)")


main()
