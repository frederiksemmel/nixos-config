"""Disk cleanup on beast.

Every run reads how full / is and, if a question is open on Linear, whether it
got an answer. Only when there is something to do does it gather the facts
(sizes, worktree state, gc roots) and start a Claude session with them.
"""

import concurrent.futures
import datetime
import fcntl
import json
import os
import pwd
import subprocess
import sys
import urllib.request
from pathlib import Path

HOME = Path.home()
STATE = Path(os.environ.get("XDG_STATE_HOME", HOME / ".local/state")) / "disk-cleanup"
PENDING = STATE / "pending.json"
ISSUE_FILE = STATE / "issue"
PREFIX = "beast disk cleanup:"
START_AT = 90
REMIND_AFTER = datetime.timedelta(days=2)
GIB = 1024**3
BUILD_DIRS = ["target", "node_modules", ".devenv", ".venv", ".direnv"]
CACHES = [".cache", ".cargo/registry", ".cargo/git", ".npm", ".local/share/pnpm"]

DUA = os.environ["DUA"]
DISK_FILL = os.environ["DISK_FILL"]
CLAUDE = os.environ["CLAUDE"]
PROMPT = Path(os.environ["PROMPT"])
LINEAR_KEY = Path(os.path.expandvars(os.environ["LINEAR_KEY_FILE"]))


def run(*cmd, cwd=None, timeout=120, check=True):
    try:
        r = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except (subprocess.TimeoutExpired, OSError):
        return None
    return r.stdout.strip() if r.returncode == 0 or not check else None


def now():
    return datetime.datetime.now(datetime.timezone.utc)


def fill():
    return int(run(DISK_FILL).rstrip("%"))


def gib(n):
    return f"{n / GIB:.1f} GiB"


def age(ts):
    days = (now().timestamp() - ts) / 86400
    return f"{days:.0f}d"


def owner(path):
    try:
        return pwd.getpwuid(path.lstat().st_uid).pw_name
    except (KeyError, OSError):
        return "?"


def sizes(paths):
    """Disk usage per path, scanned in parallel by dua."""
    paths = [str(p) for p in paths]
    if not paths:
        return {}
    r = subprocess.run(
        [DUA, "--format", "bytes", "aggregate", *paths], capture_output=True, text=True, timeout=900
    )
    out = {}
    for line in r.stdout.splitlines():
        size, sep, path = line.strip().partition(" b ")
        if sep and path != "total":
            out[path] = int(size)
    return out


def busy_dirs():
    """Working directories of live processes and Claude sessions."""
    dirs = set()
    for proc in Path("/proc").glob("[0-9]*"):
        try:
            dirs.add(os.readlink(proc / "cwd"))
        except OSError:
            pass
    for f in (HOME / ".claude/sessions").glob("*.json"):
        try:
            s = json.loads(f.read_text())
            if Path(f"/proc/{s['pid']}").exists():
                dirs.add(s["cwd"])
        except (OSError, ValueError, KeyError):
            pass
    return dirs


def newest_mtime(tree):
    prune = []
    for name in [".git", "result", *BUILD_DIRS]:
        prune += ["-name", name, "-o"]
    out = run(
        "find",
        str(tree),
        "(",
        *prune[:-1],
        ")",
        "-prune",
        "-o",
        "-type",
        "f",
        "-printf",
        "%T@\n",
        timeout=60,
    )
    times = [float(t) for t in (out or "").split()]
    return max(times, default=tree.stat().st_mtime)


def pull_requests(repo):
    """Branch name to PR state, from GitHub."""
    out = run(
        "gh",
        "pr",
        "list",
        "--state",
        "all",
        "--limit",
        "500",
        "--json",
        "headRefName,state,number",
        cwd=repo,
        timeout=60,
    )
    prs = {}
    for pr in json.loads(out or "[]"):
        prs.setdefault(pr["headRefName"], f"#{pr['number']} {pr['state']}")
    return prs


def worktree(wt, prs, busy):
    def git(*args):
        return run("git", *args, cwd=wt)

    branch = git("rev-parse", "--abbrev-ref", "HEAD") or "?"
    base = git("rev-parse", "--abbrev-ref", "origin/HEAD") or "origin/main"
    cherry = git("cherry", base, "HEAD")
    status = git("status", "--porcelain")
    return {
        "path": str(wt),
        "branch": branch,
        "pr": prs.get(branch, "none"),
        "unmerged_commits": None if cherry is None else cherry.count("+"),
        "unpushed_commits": git("rev-list", "--count", "HEAD", "--not", "--remotes"),
        "changed_files": None if status is None else len(status.splitlines()),
        "last_change": age(newest_mtime(wt)),
        "in_use": any(d == str(wt) or d.startswith(f"{wt}/") for d in busy),
    }


def gc_roots():
    out = run("nix-store", "--gc", "--print-roots", timeout=300, check=False) or ""
    roots = []
    for line in out.splitlines():
        link, sep, target = line.partition(" -> ")
        link = link.strip('"')
        if not sep or not link.startswith(str(HOME)):
            continue
        link, target = Path(link), Path(target)
        system = target / "system"
        try:
            arch = system.read_text().strip() if system.is_file() else ""
            mtime = link.lstat().st_mtime
        except OSError:
            continue
        roots.append(
            {
                "link": str(link),
                "target": target.name[33:],
                "age": age(mtime),
                "aarch64_system": arch == "aarch64-linux",
            }
        )
    return roots


def table(rows, cols):
    lines = ["| " + " | ".join(cols) + " |", "|" + "---|" * len(cols)]
    lines += ["| " + " | ".join(str(r.get(c, "")) for c in cols) + " |" for r in rows]
    return "\n".join(lines)


def facts(used):
    repos = [p for p in (HOME / "repos").iterdir() if (p / ".git").exists()]
    worktrees = sorted(p for r in repos for p in (r / ".claude/worktrees").glob("*") if p.is_dir())
    data = [p for d in ("/srv/sv/data", "/srv/sv/eval") for p in Path(d).glob("*") if p.is_dir()]
    data2 = [p for d in data for p in d.glob("*") if p.is_dir()]
    caches = [p for c in CACHES for p in (HOME / c).glob("*") if p.is_dir()]
    builds = [r / b for r in repos for b in BUILD_DIRS if (r / b).is_dir()]

    with concurrent.futures.ThreadPoolExecutor(16) as pool:
        for r in repos:
            pool.submit(run, "git", "fetch", "--quiet", "origin", cwd=r, timeout=60)
    busy = busy_dirs()
    with concurrent.futures.ThreadPoolExecutor(16) as pool:
        size_jobs = [pool.submit(sizes, ps) for ps in (worktrees, data, data2, caches, builds)]
        prs = dict(zip(repos, pool.map(pull_requests, repos), strict=True))
        roots_job = pool.submit(gc_roots)
        wts = list(pool.map(lambda wt: worktree(wt, prs[wt.parents[2]], busy), worktrees))
        size = {k: v for job in size_jobs for k, v in job.result().items()}
        roots = roots_job.result()

    def sized(paths, at_least=0):
        rows = [
            {
                "path": str(p),
                "size": size.get(str(p), 0),
                "owner": owner(p),
                "dir_mtime": age(p.stat().st_mtime),
            }
            for p in paths
        ]
        rows = sorted((r for r in rows if r["size"] >= at_least), key=lambda r: -r["size"])
        return [dict(r, size=gib(r["size"])) for r in rows]

    for w in wts:
        w["size"] = gib(size.get(w["path"], 0))
    wts.sort(key=lambda w: -size.get(w["path"], 0))
    plain = [r for r in roots if "/.claude/worktrees/" not in r["link"]]
    in_worktrees = {}
    for r in roots:
        if "/.claude/worktrees/" in r["link"]:
            wt = (
                r["link"].split("/.claude/worktrees/")[0]
                + "/.claude/worktrees/"
                + r["link"].split("/.claude/worktrees/")[1].split("/")[0]
            )
            in_worktrees[wt] = in_worktrees.get(wt, 0) + 1

    return "\n\n".join(
        [
            f"Fill: {used}% (disk-fill). Sizes are disk usage from dua; extents shared by"
            " reflink copies count in each copy, so `btrfs filesystem du -s` (Exclusive)"
            " tells what one deletion frees when it matters.",
            "## Worktrees\n\n"
            + table(
                wts,
                [
                    "path",
                    "size",
                    "branch",
                    "pr",
                    "unmerged_commits",
                    "unpushed_commits",
                    "changed_files",
                    "last_change",
                    "in_use",
                ],
            ),
            "## /srv/sv, first level\n\n"
            + table(sized(data), ["path", "size", "owner", "dir_mtime"]),
            "## /srv/sv, second level, 5 GiB and more\n\n"
            + table(sized(data2, 5 * GIB), ["path", "size", "owner", "dir_mtime"]),
            "## Caches, 100 MiB and more\n\n"
            + table(sized(caches, GIB // 10), ["path", "size", "owner", "dir_mtime"]),
            "## Build outputs in main checkouts\n\n"
            + table(sized(builds), ["path", "size", "dir_mtime"]),
            "## gc roots outside worktrees\n\n"
            + table(plain, ["link", "target", "age", "aarch64_system"]),
            "## gc roots inside worktrees (count)\n\n"
            + "\n".join(f"- {wt}: {n}" for wt, n in sorted(in_worktrees.items())),
        ]
    )


def linear(query, **variables):
    req = urllib.request.Request(
        "https://api.linear.app/graphql",
        data=json.dumps({"query": query, "variables": variables}).encode(),
        headers={
            "Authorization": LINEAR_KEY.read_text().strip(),
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        body = json.load(r)
    if body.get("errors"):
        raise RuntimeError(body["errors"])
    return body["data"]


ISSUE = """
query($id: String!) {
  issue(id: $id) {
    id identifier url state { type }
    comments(first: 100) { nodes { body createdAt user { name } } }
  }
}"""

COMMENT = """
mutation($issue: String!, $body: String!) {
  commentCreate(input: { issueId: $issue, body: $body }) { success }
}"""


def claude(section):
    mcp = Path(os.environ["XDG_RUNTIME_DIR"]) / "disk-cleanup-mcp.json"
    config = {
        "mcpServers": {
            "linear": {
                "type": "http",
                "url": "https://mcp.linear.app/mcp",
                "headers": {"Authorization": f"Bearer {LINEAR_KEY.read_text().strip()}"},
            }
        }
    }
    fd = os.open(mcp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump(config, f)
    prompt = f"{PROMPT.read_text()}\n# This run\n\n{section}\n"
    subprocess.run(
        [
            CLAUDE,
            "-p",
            "--permission-mode",
            "auto",
            "--strict-mcp-config",
            "--mcp-config",
            str(mcp),
        ],
        input=prompt,
        text=True,
        cwd=HOME,
        check=True,
    )


def comment(identifier, body):
    issue = linear(ISSUE, id=identifier)["issue"]
    linear(COMMENT, issue=issue["id"], body=f"{PREFIX} {body}")


def collect_garbage():
    """Run the Nix GC after the session and report it on the run's issue."""
    if not ISSUE_FILE.exists():
        return
    before = fill()
    out = run("nix-collect-garbage", "--delete-older-than", "14d", timeout=3600, check=False)
    summary = (out or "no output").splitlines()[-1]
    comment(
        ISSUE_FILE.read_text().strip(),
        f"Nix garbage collection done: {summary}. / went from {before}% to {fill()}% full.",
    )
    ISSUE_FILE.unlink()


def answer(pending, used):
    issue = linear(ISSUE, id=pending["issue"])["issue"]
    seen = pending["seen"]
    new = [
        c
        for c in issue["comments"]["nodes"]
        if c["createdAt"] > seen and not c["body"].startswith(PREFIX)
    ]
    closed = issue["state"]["type"] in ("completed", "canceled")
    if new or closed:
        pending["seen"] = max([seen] + [c["createdAt"] for c in issue["comments"]["nodes"]])
        PENDING.write_text(json.dumps(pending, indent=2))
        replies = "\n\n".join(f"{c['user']['name']} at {c['createdAt']}:\n{c['body']}" for c in new)
        linear(
            COMMENT,
            issue=issue["id"],
            body=f"{PREFIX} got the answer, working on it now. / is {used}% full.",
        )
        claude(
            f"Mode: answer. Issue {issue['identifier']} ({issue['url']})"
            f"{' was closed by a person' if closed else ''}.\n\n"
            f"Pending question:\n{json.dumps(pending, indent=2)}\n\n"
            f"New comments from people:\n\n{replies or '(none)'}\n\n{facts(used)}"
        )
        return
    asked = datetime.datetime.fromisoformat(pending["asked"])
    if used >= START_AT and not pending.get("reminded") and now() - asked > REMIND_AFTER:
        linear(
            COMMENT,
            issue=issue["id"],
            body=f"{PREFIX} reminder: / is {used}% full, the questions above are still open.",
        )
        pending["reminded"] = True
        PENDING.write_text(json.dumps(pending, indent=2))


def main():
    STATE.mkdir(parents=True, exist_ok=True)
    lock = open(STATE / "lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return
    used = fill()
    if "--facts" in sys.argv:
        print(facts(used))
    elif PENDING.exists():
        answer(json.loads(PENDING.read_text()), used)
        collect_garbage()
    elif used >= START_AT or "--force" in sys.argv:
        claude(f"Mode: cleanup.\n\n{facts(used)}")
        collect_garbage()


if __name__ == "__main__":
    main()
