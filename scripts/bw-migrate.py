"""Migrate the pass stores, SSH keys and .env secrets into Bitwarden.

Run in a real terminal (gpg pinentry, bw login and SSH passphrases prompt there):

  nix shell nixpkgs#python3 -c python3 scripts/bw-migrate.py <command>

  check                 decrypt and parse everything, print counts only, write nothing
  import                convert, import what the vault is missing, verify, create the Dev collection
  verify                compare every vault item against its pass entry or key file
  env REPO KEY...       move KEYs from REPO/.env into Bitwarden via secretspec
  retire-keys [--yes]   delete private key files the Bitwarden SSH agent already serves

Every command is safe to re-run. Plaintext only ever touches $XDG_RUNTIME_DIR/bw-migration
(tmpfs), which `import` empties before it exits.
"""

import argparse
import base64
import getpass
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import uuid
from pathlib import Path

HOME = Path.home()
OUT = Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "bw-migration"
SERVER = "https://vault.bitwarden.eu"
ORG = "spadesvision"
SHARED_COLLECTION = "Shared"
DEV_COLLECTION = "Dev"
PASS_FOLDER = "Imported from pass"
SSH_FOLDER = "SSH keys"

PERSONAL_STORE = HOME / ".password-store"
ORG_STORE = HOME / ".password-store-spadesvision"

PERSONAL_SSH = [
    ".ssh/beast_login",
    ".ssh/id_github",
    ".ssh/id_sourcehut",
    ".ssh/google_compute_engine",
    ".ssh/my-nixbuild-key",
]
ORG_SSH = [
    ".ssh/id_spadesvision_lambda",
    ".ssh/id_spadesvision_warren",
    *sorted(
        str(p.relative_to(HOME))
        for p in (HOME / ".ssh/spadesvision").glob("*/id_*")
        if p.suffix != ".pub"
    ),
]
KEEP_ON_DISK = {".ssh/my-nixbuild-key", ".ssh/id_github", ".ssh/google_compute_engine"}

USER_KEYS = {"login", "user", "username", "user name", "email", "e-mail", "mail", "benutzer", "benutzername"}
URL_KEYS = {"url", "uri", "website", "site", "link", "web"}
OTP_KEYS = {"otp", "totp", "otpauth", "2fa"}

KV = re.compile(r"^([A-Za-z][A-Za-z0-9 _.-]{0,29}?)\s*:\s*(.*)$")
HTTP = re.compile(r"^https?://", re.I)
DOMAIN = re.compile(r"^[a-z0-9-]+(\.[a-z0-9-]+)+$", re.I)


# ---------------------------------------------------------------- pass -> items


def field(name, value, hidden=True):
    return {"name": name, "value": value, "type": 1 if hidden else 0, "linkedId": None}


def base_item(name, notes, fields):
    return {
        "id": str(uuid.uuid4()),
        "organizationId": None,
        "folderId": None,
        "collectionIds": None,
        "reprompt": 0,
        "name": name,
        "notes": notes or None,
        "favorite": False,
        "fields": fields,
    }


def decrypt(path):
    r = subprocess.run(["gpg", "--quiet", "--decrypt", str(path)], capture_output=True)
    if r.returncode != 0:
        sys.exit(f"gpg failed for {path}: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def store_entries(store):
    for f in sorted(store.rglob("*.gpg")):
        rel = f.relative_to(store)
        if rel.parts[0] in (".git", ".password-store-spadesvision"):
            continue
        yield str(rel)[: -len(".gpg")], f


def parse_entry(rel, text, stats):
    lines = text.replace("\r\n", "\n").split("\n")
    while lines and not lines[-1].strip():
        lines.pop()
    marker = field("pass-path", rel, hidden=False)
    parts = [p for p in rel.split("/") if p]
    name = "/".join(parts)

    first = lines[0].strip() if lines else ""
    body = lines[1:]
    if not first or first.startswith("#") or first.startswith("-----BEGIN"):
        stats["secure_notes"].append(rel)
        item = base_item(name, "\n".join(lines).strip("\n"), [marker])
        item.update(type=2, secureNote={"type": 0})
        return item

    username = totp = None
    uris, fields, notes = [], [], []
    for line in body:
        s = line.strip()
        if s.lower().startswith("otpauth://") and totp is None:
            totp = s
            continue
        if HTTP.match(s):
            uris.append(s)
            continue
        m = KV.match(s) if s else None
        if m:
            k, v = m.group(1).strip(), m.group(2).strip()
            kl = k.lower()
            if kl in USER_KEYS and username is None and v:
                username = v
                continue
            if kl in URL_KEYS and v:
                uris.append(v if re.match(r"^[a-z][a-z0-9+.-]*://", v, re.I) else f"https://{v}")
                continue
            if kl in OTP_KEYS and totp is None and v:
                totp = v
                continue
            fields.append(field(k, v))
            continue
        notes.append(line)

    path_user = next((p for p in reversed(parts[1:]) if "@" in p), None)
    if path_user:
        if username is None:
            username = path_user
        parts = [p for p in parts if p != path_user]
        name = "/".join(parts)
    if not uris and DOMAIN.match(parts[0]):
        uris.append(f"https://{parts[0]}")

    item = base_item(name, "\n".join(notes).strip("\n"), fields + [marker])
    item.update(
        type=1,
        login={
            "uris": [{"match": None, "uri": u} for u in uris],
            "username": username,
            "password": lines[0] if lines else None,
            "totp": totp,
        },
    )
    stats["logins"] += 1
    stats["with_username"] += username is not None
    stats["with_uri"] += bool(uris)
    stats["with_totp"] += totp is not None
    return item


def step(msg):
    print(f"-> {msg}", flush=True)


def convert_store(store, skip_identical_to=None):
    step(f"decrypting {store}")
    stats = {
        "entries": 0, "logins": 0, "with_username": 0, "with_uri": 0, "with_totp": 0,
        "secure_notes": [], "binary": [], "dup_identical": [], "dup_differs": [],
    }
    items = []
    for rel, f in store_entries(store):
        stats["entries"] += 1
        raw = decrypt(f)
        if skip_identical_to is not None and rel in skip_identical_to:
            if skip_identical_to[rel] == hashlib.sha256(raw).digest():
                stats["dup_identical"].append(rel)
                continue
            stats["dup_differs"].append(rel)
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            text = None
        if text is None or "\x00" in text:
            stats["binary"].append(rel)
            continue
        items.append(parse_entry(rel, text, stats))
    return items, stats


def store_hashes(store):
    step(f"hashing {store} to find duplicates")
    return {rel: hashlib.sha256(decrypt(f)).digest() for rel, f in store_entries(store)}


# ---------------------------------------------------------------- ssh keys -> items


def keygen(*args, passphrase=None, askpass=None):
    env = dict(os.environ)
    if passphrase is not None:
        env.update(SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE="force", BWMIG_PASSPHRASE=passphrase)
    return subprocess.run(["ssh-keygen", *args], capture_output=True, text=True, env=env, stdin=subprocess.DEVNULL)


def ssh_inventory(rels):
    for rel in rels:
        key = HOME / rel
        if not key.exists():
            continue
        pub = key.with_name(key.name + ".pub")
        fingerprint = keygen("-lf", str(pub)).stdout.split()[1]
        encrypted = keygen("-y", "-P", "", "-f", str(key)).returncode != 0
        yield rel, key, pub, fingerprint, encrypted


def ssh_items(rels, passphrases, wanted):
    askpass = OUT / "askpass.sh"
    workdir = OUT / "work"
    askpass.write_text('#!/bin/sh\nprintf "%s\\n" "$BWMIG_PASSPHRASE"\n')
    askpass.chmod(0o700)
    workdir.mkdir(mode=0o700, exist_ok=True)

    items = []
    try:
        for rel, key, pub, fingerprint, encrypted in ssh_inventory(rels):
            if not wanted(rel):
                continue
            step(f"reading ~/{rel}")
            if encrypted:
                work = workdir / key.name
                private = None
                candidates = list(passphrases)
                while private is None:
                    if not candidates:
                        candidates.append(getpass.getpass(f"Passphrase for ~/{rel}: "))
                    pw = candidates.pop(0)
                    shutil.copyfile(key, work)
                    work.chmod(stat.S_IRUSR | stat.S_IWUSR)
                    if keygen("-p", "-N", "", "-f", str(work), passphrase=pw, askpass=askpass).returncode == 0:
                        private = work.read_text()
                        if pw not in passphrases:
                            passphrases.append(pw)
                    else:
                        print(f"  wrong passphrase for ~/{rel}", file=sys.stderr)
                    work.unlink()
            else:
                private = key.read_text()

            item = base_item(f"ssh {key.name}", None, [field("ssh-path", f"~/{rel}", hidden=False)])
            item.update(
                type=5,
                sshKey={"privateKey": private, "publicKey": pub.read_text().strip(), "keyFingerprint": fingerprint},
            )
            items.append(item)
    finally:
        askpass.unlink(missing_ok=True)
        shutil.rmtree(workdir, ignore_errors=True)
    return items


# ---------------------------------------------------------------- bitwarden cli


def bw(*args, check=True):
    r = subprocess.run(["bw", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"bw {' '.join(args[:2])} failed: {r.stderr.strip() or r.stdout.strip()}")
    return r.stdout


def bw_status():
    return json.loads(bw("status"))


def ensure_session():
    status = bw_status()
    step("unlocking bw")
    if status["status"] == "unauthenticated":
        bw("config", "server", SERVER)
        session = subprocess.run(["bw", "login", "--raw"], stdout=subprocess.PIPE, text=True).stdout.strip()
    elif status["serverUrl"] != SERVER:
        sys.exit(f"bw is logged in to {status['serverUrl']}, expected {SERVER}; run `bw logout` first")
    elif status["status"] == "locked":
        session = subprocess.run(["bw", "unlock", "--raw"], stdout=subprocess.PIPE, text=True).stdout.strip()
    else:
        session = os.environ["BW_SESSION"]
    if not session:
        sys.exit("bw login/unlock failed")
    os.environ["BW_SESSION"] = session
    step("syncing vault")
    bw("sync")
    return bw_status()


def org_id():
    orgs = json.loads(bw("list", "organizations"))
    match = [o for o in orgs if o["name"].lower() == ORG]
    if not match:
        sys.exit(f"no organization named {ORG!r}: create it at {SERVER} (New organization -> Free)")
    return match[0]["id"]


def vault_index():
    index = {}
    for item in json.loads(bw("list", "items")):
        for f in item.get("fields") or []:
            if f.get("name") in ("pass-path", "ssh-path"):
                owner = "org" if item.get("organizationId") else "personal"
                index[(owner, f["name"], f.get("value"))] = item
    return index


def marker_of(item, owner):
    f = next(f for f in item["fields"] if f["name"] in ("pass-path", "ssh-path"))
    return (owner, f["name"], f["value"])


def components(item):
    login = item.get("login") or {}
    ssh = item.get("sshKey") or {}
    norm = lambda v: (v or "").strip()
    return {
        "type": str(item.get("type")),
        "name": norm(item.get("name")),
        "notes": norm(item.get("notes")),
        "fields": sorted(f"{f.get('name')}={norm(f.get('value'))}" for f in item.get("fields") or []),
        "username": norm(login.get("username")),
        "password": login.get("password") or "",
        "totp": norm(login.get("totp")),
        "uris": sorted(norm(u.get("uri")) for u in login.get("uris") or []),
        "privateKey": norm(ssh.get("privateKey")),
        "publicKey": norm(ssh.get("publicKey")),
    }


def diff(expected, actual):
    a, b = components(expected), components(actual)
    return [k for k in a if a[k] != b[k]]


def write_private(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as fh:
        json.dump(data, fh, ensure_ascii=False)


def bw_import(items, org, folders_or_collections):
    path = OUT / ("org.json" if org else "personal.json")
    if org:
        payload = {"encrypted": False, "collections": folders_or_collections, "items": items}
    else:
        payload = {"encrypted": False, "folders": folders_or_collections, "items": items}
    write_private(path, payload)
    try:
        args = ["import", *(["--organizationid", org] if org else []), "bitwardenjson", str(path)]
        step(f"uploading {len(items)} items to {'the spadesvision organization' if org else 'your personal vault'}")
        print(bw(*args).strip())
    finally:
        path.unlink(missing_ok=True)


def ensure_dev_collection(org, email):
    existing = json.loads(bw("list", "org-collections", "--organizationid", org))
    if any(c["name"] == DEV_COLLECTION for c in existing):
        return
    members = json.loads(bw("list", "org-members", "--organizationid", org))
    me = next(m["id"] for m in members if m.get("email", "").lower() == email.lower())
    payload = {
        "organizationId": org,
        "name": DEV_COLLECTION,
        "externalId": None,
        "groups": [],
        "users": [{"id": me, "readOnly": False, "hidePasswords": False, "manage": True}],
    }
    encoded = base64.b64encode(json.dumps(payload).encode()).decode()
    bw("create", "org-collection", "--organizationid", org, encoded)
    print(f"created collection {DEV_COLLECTION!r}")


# ---------------------------------------------------------------- commands


def print_stats(label, s):
    print(f"\n== {label}")
    print(f"  entries: {s['entries']}  logins: {s['logins']}  secure notes: {len(s['secure_notes'])}")
    print(f"  logins with username: {s['with_username']}  with uri: {s['with_uri']}  with totp: {s['with_totp']}")
    for k in ("secure_notes", "binary", "dup_identical", "dup_differs"):
        if s[k]:
            print(f"  {k}: {', '.join(s[k])}")


def expected_pass_items():
    org_items, org_stats = convert_store(ORG_STORE)
    personal_items, personal_stats = convert_store(PERSONAL_STORE, store_hashes(ORG_STORE))
    return org_items, org_stats, personal_items, personal_stats


def cmd_check(_):
    org_items, org_stats, personal_items, personal_stats = expected_pass_items()
    print_stats(f"spadesvision store ({ORG_STORE})", org_stats)
    print_stats(f"personal store ({PERSONAL_STORE})", personal_stats)
    for label, rels in (("personal", PERSONAL_SSH), ("spadesvision", ORG_SSH)):
        print(f"\n== ssh keys -> {label}")
        for rel, _, _, fingerprint, encrypted in ssh_inventory(rels):
            print(f"  ~/{rel}: {'passphrase' if encrypted else 'plain'} {fingerprint}")
    status = bw_status()
    print(f"\n== bitwarden: {status['status']} {status.get('userEmail') or ''} {status.get('serverUrl') or ''}")
    print("\ncheck: nothing written")


def cmd_import(_):
    OUT.mkdir(mode=0o700, exist_ok=True)
    try:
        status = ensure_session()
        org = org_id()
        index = vault_index()
        org_items, org_stats, personal_items, personal_stats = expected_pass_items()
        print_stats("spadesvision store", org_stats)
        print_stats("personal store", personal_stats)

        missing_org = [i for i in org_items if marker_of(i, "org") not in index]
        missing_personal = [i for i in personal_items if marker_of(i, "personal") not in index]

        have = {k[2] for k in index if k[1] == "ssh-path"}
        passphrases = []
        missing_personal += ssh_items(PERSONAL_SSH, passphrases, lambda rel: f"~/{rel}" not in have)
        missing_org += ssh_items(ORG_SSH, passphrases, lambda rel: f"~/{rel}" not in have)

        pass_folder, ssh_folder, shared = (str(uuid.uuid4()) for _ in range(3))
        for item in missing_personal:
            item["folderId"] = ssh_folder if item["type"] == 5 else pass_folder
        for item in missing_org:
            item["collectionIds"] = [shared]

        print(f"\nimporting {len(missing_personal)} personal and {len(missing_org)} spadesvision items")
        if missing_personal:
            bw_import(missing_personal, None, [
                {"id": pass_folder, "name": PASS_FOLDER},
                {"id": ssh_folder, "name": SSH_FOLDER},
            ])
        if missing_org:
            bw_import(missing_org, org, [
                {"id": shared, "organizationId": None, "name": SHARED_COLLECTION, "externalId": None},
            ])
        bw("sync")
    finally:
        shutil.rmtree(OUT, ignore_errors=True)

    if run_verify(org_items, personal_items):
        ensure_dev_collection(org, status["userEmail"])


def run_verify(org_items, personal_items):
    step("verifying every item against its source")
    index = vault_index()
    bad = 0
    expected = [(i, "org") for i in org_items] + [(i, "personal") for i in personal_items]
    for item, owner in expected:
        key = marker_of(item, owner)
        actual = index.get(key)
        if actual is None:
            print(f"  MISSING   {owner}: {key[2]}")
            bad += 1
        elif d := diff(item, actual):
            print(f"  DIFFERENT {owner}: {key[2]} ({', '.join(d)})")
            bad += 1
    ssh_have = {k[2]: v for k, v in index.items() if k[1] == "ssh-path"}
    for owner, rels in (("personal", PERSONAL_SSH), ("org", ORG_SSH)):
        for rel, _, pub, _, _ in ssh_inventory(rels):
            actual = ssh_have.get(f"~/{rel}")
            if actual is None:
                print(f"  MISSING   {owner}: ~/{rel}")
                bad += 1
            elif (actual.get("sshKey") or {}).get("publicKey", "").split()[:2] != pub.read_text().split()[:2]:
                print(f"  DIFFERENT {owner}: ~/{rel} (publicKey)")
                bad += 1
    total = len(expected) + sum(1 for _ in ssh_inventory(PERSONAL_SSH + ORG_SSH))
    print(f"\nverify: {total - bad}/{total} items match")
    return bad == 0


def cmd_verify(_):
    ensure_session()
    org_items, _, personal_items, _ = expected_pass_items()
    sys.exit(0 if run_verify(org_items, personal_items) else 1)


def cmd_env(args):
    repo = Path(args.repo).resolve()
    manifest = repo / "secretspec.toml"
    dotenv = repo / ".env"
    project = args.project or repo.name
    present = set(re.findall(r"^([A-Za-z_][A-Za-z0-9_]*)=", dotenv.read_text(), re.M)) if dotenv.exists() else set()
    if missing := [k for k in args.keys if k not in present]:
        sys.exit(f"not in {dotenv}: {', '.join(missing)}")
    if manifest.exists():
        sys.exit(f"{manifest} already exists")

    ensure_session()
    declarations = "\n".join(f'{k} = {{ description = "{k}" }}' for k in args.keys)
    manifest.write_text(
        "#:schema https://secretspec.dev/schema/secretspec.schema.json\n\n"
        f'[project]\nname = "{project}"\nrevision = "1.0"\n\n'
        f'[providers]\n{ORG} = "bw://{ORG}@{DEV_COLLECTION}"\n\n'
        f'[defaults]\nproviders = ["{ORG}"]\n\n'
        f"[profiles.default]\n{declarations}\n"
    )
    run = lambda *a: subprocess.run(["secretspec", *a], cwd=repo, stdin=subprocess.DEVNULL)
    if run("import", "dotenv://.env", "--delete-source").returncode != 0:
        sys.exit("secretspec import failed; .env is untouched")
    if run("check").returncode != 0:
        sys.exit("secretspec check failed")

    devenv_yaml = repo / "devenv.yaml"
    if devenv_yaml.exists() and not re.search(r"^secretspec:", devenv_yaml.read_text(), re.M):
        with devenv_yaml.open("a") as fh:
            fh.write("secretspec:\n  enable: true\n")
    print(f"{project}: {len(args.keys)} secrets now in bw://{ORG}@{DEV_COLLECTION} as secretspec/{project}/default/*")


def cmd_retire_keys(args):
    sock = os.environ.get("SSH_AUTH_SOCK", "")
    if "bitwarden" not in sock:
        sys.exit(f"SSH_AUTH_SOCK={sock} is not the Bitwarden agent; enable profiles.bitwarden.sshAgent first")
    served = {" ".join(l.split()[:2]) for l in subprocess.run(["ssh-add", "-L"], capture_output=True, text=True).stdout.splitlines()}
    for rel in PERSONAL_SSH + ORG_SSH:
        key = HOME / rel
        if rel in KEEP_ON_DISK or not key.exists():
            continue
        pub = " ".join(key.with_name(key.name + ".pub").read_text().split()[:2])
        if pub not in served:
            print(f"  keep    ~/{rel} (agent does not serve it)")
        elif args.yes:
            key.unlink()
            print(f"  removed ~/{rel}")
        else:
            print(f"  would remove ~/{rel}")
    if not args.yes:
        print("\nre-run with --yes to delete")


def main():
    if sys.stdin.isatty():
        os.environ.setdefault("GPG_TTY", os.ttyname(sys.stdin.fileno()))
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    sub.add_parser("import").set_defaults(fn=cmd_import)
    sub.add_parser("verify").set_defaults(fn=cmd_verify)
    env = sub.add_parser("env")
    env.add_argument("repo")
    env.add_argument("keys", nargs="+")
    env.add_argument("--project")
    env.set_defaults(fn=cmd_env)
    retire = sub.add_parser("retire-keys")
    retire.add_argument("--yes", action="store_true")
    retire.set_defaults(fn=cmd_retire_keys)
    args = ap.parse_args()
    args.fn(args)


if __name__ == "__main__":
    main()
