def shpool-session-name [pid] {
  open --raw $"/proc/($pid)/environ" | split row (char nul) | where $it starts-with "SHPOOL_SESSION_NAME=" | first | str substring 20..
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
  let claudes = glob $"($env.HOME)/.claude/sessions/*.json" | each {|f|
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
      let title = $titles | where type == "custom-title" | get -o customTitle | last
        | default ($titles | where type == "ai-title" | get -o aiTitle | last)
        | default $prompt
        | default "spare"
      { name: (shpool-session-name $s.pid), cname: ($s.name? | default ""), title: $title, status: $s.status, cwd: $s.cwd }
    }
  } | compact
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
    }
  } | sort-by last --reverse
}
