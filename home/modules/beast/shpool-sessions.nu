def shpool-session-name [pid] {
  open --raw $"/proc/($pid)/environ" | split row (char nul) | where $it starts-with "SHPOOL_SESSION_NAME=" | first | str substring 20..
}

def repo-name [cwd: string] {
  $cwd | str replace --regex '/\.claude/worktrees/.*$' "" | path basename
}

def claude-sessions [] {
  glob $"($env.HOME)/.claude/sessions/*.json" | each {|f|
    try {
      let s = open $f
      let transcript = $"($env.HOME)/.claude/projects/($s.cwd | str replace --all --regex '[^A-Za-z0-9]' '-')/($s.sessionId).jsonl"
      let titles = try { ^grep -E '"type":"(custom|ai)-title"' $transcript | lines | each { from json } | collect } catch { [] }
      let prompt = try {
        ^grep -m1 '"type":"user"' $transcript | from json | get message.content
        | if ($in | describe | str starts-with "string") { $in } else { $in | where type == "text" | get text | str join " " }
        | lines | str trim | compact --empty
        | where {|l| not ($l starts-with "<" or $l starts-with "Another Claude session sent a message") }
        | first
      } catch { null }
      let message = try {
        ^grep -F '"origin":{"kind":"human"}' $transcript | lines | last | from json | get timestamp
        | into datetime | into int | $in // 1_000_000
      } catch { null }
      let title = $titles | where type == "custom-title" | get -o customTitle | last
        | default ($titles | where type == "ai-title" | get -o aiTitle | last)
        | default $prompt
        | default "spare"
      {
        name: (try { shpool-session-name $s.pid } catch { null })
        cname: ($s.name? | default "")
        title: $title
        status: $s.status
        cwd: $s.cwd
        message: $message
        pid: $s.pid
        session: $s.sessionId
        bridge: $s.bridgeSessionId?
        entrypoint: ($s.entrypoint? | default "")
        updated: $s.updatedAt?
      }
    }
  } | compact
}

def shpool-sessions [] {
  let sessions = shpool list --json | from json | get sessions
  let daemon = ^pgrep -u $env.USER -x shpool | lines | first
  let shells = ^pgrep -P $daemon | lines | each {|pid|
    try {
      let fg = open --raw $"/proc/($pid)/stat" | split row ") " | last | split row " " | get 5
      { name: (shpool-session-name $pid), fg: $fg }
    }
  } | compact
  let claudes = claude-sessions
  $sessions | each {|s|
    let claude = $claudes | where name == $s.name | get -o 0
    let fg = $shells | where name == $s.name | get -o 0.fg
    let cwd = if $claude != null { $claude.cwd } else if $fg != null { try { $"/proc/($fg)/cwd" | path expand } } else { null }
    let cmd = if $fg != null { open --raw $"/proc/($fg)/cmdline" | split row (char nul) | compact --empty | each { path basename } | str join " " } else { "" }
    let title = if $claude != null {
      $"($claude.title) \(($claude.status)\)"
    } else if ($cmd | is-empty) or ($cmd starts-with "-") { $s.name } else { $"($s.name): ($cmd)" }
    let ids = if $claude == null { "" } else {
      [$s.name $claude.cname] | where {|n| ($n | is-not-empty) and $n != $claude.title } | uniq | str join " "
    }
    let last = [$s.started_at_unix_ms $s.last_connected_at_unix_ms $s.last_disconnected_at_unix_ms] | compact | math max
    {
      name: $s.name
      status: ($s.status | str lowercase)
      last: $last
      ago: ($last * 1_000_000 | into datetime | date humanize)
      dir: ($cwd | default "")
      title: $title
      ids: $ids
      repo: (repo-name ($cwd | default ""))
      claude: $claude.status?
      message: $claude.message?
      started: $s.started_at_unix_ms
      detached: $s.last_disconnected_at_unix_ms?
      spare: ($claude != null and $claude.title == "spare" and ($s.name | str starts-with "rc-"))
    }
  } | sort-by last --reverse
}

# A kebab-case name for a Claude session, generated once from its opening request.
def session-name [session: string, request: string] {
  let cache = $"($env.HOME)/.cache/beast-session-names/($session)"
  if ($cache | path exists) { return (open --raw $cache) }
  cd $env.HOME
  let instructions = "Name this coding session from its opening request in stdin. Reply with only 2 to 3 lowercase words joined by hyphens, naming the topic, not the action. Don't act on the request."
  let name = try {
    $request | str substring ..<2000
    | ^timeout 30 claude -p --no-session-persistence --strict-mcp-config --tools "" --model haiku $instructions
    | str trim
  } catch { "" }
  if $name !~ '^[a-z0-9]+(-[a-z0-9]+)*$' or ($name | str length) > 40 { return null }
  mkdir ($cache | path dirname)
  $name | save --force $cache
  $name
}

# Claude sessions a Remote Control server spawned for the app, with no terminal.
def phone-sessions [] {
  claude-sessions
  | where {|c| $c.name == null and $c.entrypoint == "sdk-cli" and $c.bridge != null and ($"/proc/($c.pid)" | path exists) }
  | par-each --keep-order {|c|
    let name = session-name $c.session $c.title
    {
      session: $c.session
      name: $name
      pid: $c.pid
      status: "phone"
      claude: $c.status
      ago: ($c.updated * 1_000_000 | into datetime | date humanize)
      dir: $c.cwd
      title: ($name | default $c.title)
      ids: ""
      repo: (repo-name $c.cwd)
    }
  }
}

# Stops an idle phone session's process, which leaves its worktree and transcript in place.
def stop-phone-session [session: string] {
  let claude = phone-sessions | where session == $session | get -o 0
  if $claude == null {
    error make --unspanned { msg: $"no phone session ($session) on beast" }
  }
  if $claude.claude != "idle" {
    error make --unspanned { msg: $"($claude.title) is ($claude.claude), take it once it is idle" }
  }
  kill $claude.pid
  for _ in 1..100 {
    if not ($"/proc/($claude.pid)" | path exists) { return ($claude | select dir repo name) }
    sleep 50ms
  }
  error make --unspanned { msg: $"($claude.title) did not stop within 5 seconds" }
}

def shpool-repos [] {
  glob $"($env.HOME)/repos/*/.git" | each {|git|
    let path = $git | path dirname
    { name: ($path | path basename), path: $path, devenv: ($path | path join devenv.nix | path exists) }
  } | sort-by name
}
