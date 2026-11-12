"""Removes my /tmp entries that nothing touched or holds open for two days.

An entry is a top-level file or directory in /tmp that I own, or one Claude
session directory under /tmp/claude-<uid>/<project>/. It goes when every file
in it is older than the cutoff, it holds no socket and no X lock, and no live
process has its cwd, an open file or a mapped file inside it. Pass --dry-run
to only print what would go.
"""

import os
import shutil
import stat
import sys
import time
from pathlib import Path

TMP = Path("/tmp")
MAX_AGE = 2 * 86400
UID = os.getuid()
CLAUDE_TMP = TMP / f"claude-{UID}"
KEEP = {CLAUDE_TMP, TMP / f"cc-daemon-{UID}"}


def held_paths():
    held = set()
    for proc in Path("/proc").glob("[0-9]*"):
        for link in [proc / "cwd", *proc.glob("fd/*")]:
            try:
                held.add(os.readlink(link).removesuffix(" (deleted)"))
            except OSError:
                pass
        try:
            for line in (proc / "maps").read_text().splitlines():
                parts = line.split(maxsplit=5)
                if len(parts) == 6 and parts[5].startswith("/tmp/"):
                    held.add(parts[5].removesuffix(" (deleted)"))
        except OSError:
            pass
    return held


def keep(st, cutoff):
    return max(st.st_mtime, st.st_ctime) > cutoff or stat.S_ISSOCK(st.st_mode)


def in_use(path, cutoff):
    if path.name.startswith(".X"):
        return True
    try:
        st = path.lstat()
    except OSError:
        return True
    if keep(st, cutoff):
        return True
    if not stat.S_ISDIR(st.st_mode):
        return False
    for root, dirs, files in os.walk(path, onerror=lambda e: None):
        for name in dirs + files:
            try:
                st = os.lstat(os.path.join(root, name))
            except OSError:
                continue
            if keep(st, cutoff):
                return True
    return False


def candidates():
    for entry in TMP.iterdir():
        try:
            if entry.lstat().st_uid != UID or entry in KEEP:
                continue
        except OSError:
            continue
        yield entry
    if CLAUDE_TMP.is_dir():
        for project in CLAUDE_TMP.iterdir():
            if project.is_dir() and not project.is_symlink():
                yield from (s for s in project.iterdir() if s.is_dir() and not s.is_symlink())


def make_writable(func, path, _exc):
    parent = os.path.dirname(path)
    os.chmod(parent, os.lstat(parent).st_mode | stat.S_IWUSR | stat.S_IXUSR)
    func(path)


def remove(path):
    if path.is_dir() and not path.is_symlink():
        for root, dirs, _ in os.walk(path):
            for d in dirs:
                p = os.path.join(root, d)
                if not os.path.islink(p):
                    os.chmod(p, os.lstat(p).st_mode | stat.S_IWUSR | stat.S_IXUSR)
        os.chmod(path, path.lstat().st_mode | stat.S_IWUSR | stat.S_IXUSR)
        shutil.rmtree(path, onexc=make_writable)
    else:
        path.unlink()


def main():
    dry_run = "--dry-run" in sys.argv
    cutoff = time.time() - MAX_AGE
    held = held_paths()
    for path in sorted(candidates()):
        prefix = f"{path}/"
        if any(h == str(path) or h.startswith(prefix) for h in held):
            continue
        if in_use(path, cutoff):
            continue
        print(f"{'would remove' if dry_run else 'removing'} {path}", flush=True)
        if not dry_run:
            try:
                remove(path)
            except OSError as e:
                print(f"  failed: {e}", file=sys.stderr)


if __name__ == "__main__":
    main()
