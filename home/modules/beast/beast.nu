# shpool sessions on beast, in terminal windows that reattach after a dropped connection.

const self = path self
const shpool_defs = path self shpool-sessions.nu
const connect_options = [-o ConnectTimeout=5 -o BatchMode=yes]
const reset_terminal = "\e[?25h\e[?1000;1002;1003;1006;2004l\e[<99u"

# Evaluates a nushell expression on beast, with shpool-sessions.nu loaded.
def on-beast [expression: string] {
  ^ssh -o ConnectTimeout=5 beast $"(open --raw $shpool_defs | decode utf-8)\n($expression) | to json" | from json
}

def complete-sessions [] {
  let sessions = try { list } catch { [] }
  {
    options: { sort: false, completion_algorithm: fuzzy }
    completions: ($sessions | each {|s| { value: $s.name, description: $s.title } })
  }
}

def complete-repos [] {
  let repos = try { on-beast shpool-repos } catch { [] }
  {
    options: { completion_algorithm: fuzzy }
    completions: ($repos | each {|r| { value: $r.name, description: $r.path } })
  }
}

def window-title [session: record] {
  $session.title | str replace --regex ' \([a-z]+\)$' ""
}

def set-title [title: string] {
  print --no-newline $"(ansi title)($title)(char bel)"
}

# Shows why beast is out of reach and waits out the delay; enter ends it early.
def wait-to-retry [title: string, delay: duration, reason: string] {
  set-title $"($title) \(reconnecting\)"
  print --no-newline $"($reset_terminal)\r\n\e[?7l(ansi attr_dimmed)reconnecting to beast in ($delay) \(enter retries now\): ($reason)(ansi reset)\e[?7h"
  do --ignore-errors { ^timeout --foreground $"($delay / 1sec)s" head -n 1 } | ignore
  print --no-newline "\r\e[K"
}

# Keeps a window on one session and exits with it. Only ssh failing (exit 255)
# reconnects. With --create, the first attach may make the session from those
# `shpool attach` arguments; otherwise a missing session is an error.
def connect [session: string, title: string, --create: list<string>] {
  let name = $session | to json
  mut creating = $create != null
  mut delay = 1sec
  loop {
    set-title $title
    let listing = do --ignore-errors { ^ssh ...$connect_options beast shpool list --json } | complete
    if $listing.exit_code != 0 {
      wait-to-retry $title $delay ($listing.stderr | str trim | lines | last)
      $delay = [($delay * 2) 30sec] | math min
      continue
    }
    let exists = $session in ($listing.stdout | from json | get sessions.name)
    if $exists {
      $creating = false
    } else if not $creating {
      error make --unspanned { msg: $"no session ($session) on beast" }
    }
    let mode = if $exists { [-f] } else { $create }
    let started = date now
    let code = try {
      ^ssh -t ...$connect_options beast shpool attach ...$mode $name
      0
    } catch {|failure|
      $failure.exit_code? | default 1
    }
    if $code != 255 {
      exit $code
    }
    if (date now) - $started < 10sec {
      wait-to-retry $title $delay "connection lost"
      $delay = [($delay * 2) 30sec] | math min
    } else {
      $delay = 1sec
    }
  }
}

# Picks a session on beast and attaches this window to it.
#
# Esc opens a plain shell on beast instead.
export def main [] {
  let sessions = list
  let new_entry = "new session"
  let kill_entry = "kill sessions…"
  let rows = if ($sessions | is-empty) { [] } else {
    let title_width = [($sessions.title | str length | math max) 40] | math min
    let ids_width = $sessions.ids | str length | math max
    let columns = (term size).columns - 4
    $sessions | each {|s|
      [
        ($s.title | str substring ..<$title_width | fill -w $title_width)
        ($s.ids | fill -w $ids_width)
        ($s.ago | str replace " ago" "" | fill -w 14)
        ($s.status | str replace "disconnected" "detached" | fill -w 8)
        ($s.dir | path basename)
      ] | str join "  " | str substring ..<$columns
    }
  }
  let count = $sessions | length
  let choice = if ($rows | is-empty) { $count } else {
    $rows | append [$new_entry $kill_entry]
    | input list --fuzzy --index "beast session (type to filter, esc for a plain shell)"
  }
  if $choice == null {
    do --ignore-errors { ^ssh -t beast "hide-env SSH_CONNECTION; exec nu --login" }
    exit
  } else if $choice < $count {
    let session = $sessions | get $choice
    connect $session.name (window-title $session)
  } else if $choice == $count {
    new
  } else {
    let names = $rows
    | input list --fuzzy --multi --index "kill which sessions (tab marks, enter kills, esc cancels)"
    | default []
    | each {|i| $sessions | get $i | get name | to json }
    if ($names | is-not-empty) { ^ssh beast shpool kill ...$names }
    main
  }
}

# Lists the sessions on beast, newest activity first.
export def list [] {
  on-beast shpool-sessions
}

# Attaches this window to a session and keeps it attached across dropped connections.
export def attach [
  session: string@complete-sessions # session to attach
  --title: string                   # window title, default the session name
] {
  connect $session ($title | default $session)
}

# Starts a session: Claude with Remote Control in a repo, inside devenv when the repo has one.
#
# Without a repo it asks for one, or for a plain shell.
export def new [
  repo?: string@complete-repos # repo under ~/repos on beast, or a unique part of its name
  --name (-n): string          # session name, default <repo>-<number>
] {
  let repos = on-beast shpool-repos
  let target = if $repo == null {
    let choice = ["plain shell" ...$repos.name] | input list --fuzzy --index "new session in (esc cancels)"
    if $choice == null { return }
    if $choice == 0 { null } else { $repos | get ($choice - 1) }
  } else {
    let exact = $repos | where name == $repo
    let found = if ($exact | is-empty) { $repos | where name =~ $repo } else { $exact }
    if ($found | length) != 1 {
      error make --unspanned { msg: $"($repo) matches ($found | length) of: ($repos.name | str join ' ')" }
    }
    $found | first
  }
  let taken = list | get name
  let prefix = if $target == null { "s" } else { $"($target.name)-" }
  let default_name = 1..(($taken | length) + 1)
  | each {|n| $"($prefix)($n)" }
  | where $it not-in $taken
  | first
  let session = if $name != null {
    $name
  } else if $repo != null {
    $default_name
  } else {
    input $"session name \(enter for ($default_name)\): "
    | str trim
    | str replace --all --regex '\s+' '-'
    | if ($in | is-empty) { $default_name } else { $in }
  }
  if $session in $taken {
    error make --unspanned { msg: $"($session) already exists, use beast attach" }
  }
  let create = if $target == null { [] } else {
    let claude = $"claude --remote-control --remote-control-session-name-prefix ($target.name)"
    let command = if $target.devenv { $"devenv shell --no-tui -- ($claude)" } else { $claude }
    [--dir ($target.path | to json) --cmd ($command | to json)]
  }
  connect $session $session --create $create
}

# Attaches this window to the newest detached Remote Control spare.
export def spare [] {
  let spares = list | where $it.spare and $it.status == "disconnected"
  if ($spares | is-empty) {
    error make --unspanned { msg: "no detached spare on beast yet, claude-rc-spare starts one within a few seconds" }
  }
  attach ($spares | first | get name)
}

# Kills the sessions matching every given filter, after listing them and asking.
#
# Attached sessions and busy Claude sessions are kept unless included. Unused
# spares are always kept.
@example "See what has been silent for three days" { beast clean --no-message-for 3day --dry-run }
@example "Clean one repo" { beast clean --no-message-for 2day --repo blackjackeye }
export def clean [
  --no-message-for: duration   # no message from you for this long; shells count attaches and detaches
  --detached-for: duration     # detached for this long
  --older-than: duration       # started this long ago
  --repo: string@complete-repos # in this repo
  --match: string              # name or title matches this regex
  --include-attached           # also consider attached sessions
  --include-busy               # also consider Claude sessions that are working
  --dry-run (-n)               # list the matches without killing them
  --yes (-y)                   # kill without asking
] {
  if ([$no_message_for $detached_for $older_than $repo $match] | compact | is-empty) {
    error make --unspanned { msg: "give at least one filter, see beast clean --help" }
  }
  let age = {|ms| (date now) - ($ms * 1_000_000 | into datetime) }
  let matches = list
  | where not $it.spare
  | where {|s| $include_attached or $s.status != "attached" }
  | where {|s| $include_busy or $s.claude != "busy" }
  | where {|s| $no_message_for == null or (do $age ($s.message | default $s.last)) >= $no_message_for }
  | where {|s| $detached_for == null or ($s.status != "attached" and $s.detached != null and (do $age $s.detached) >= $detached_for) }
  | where {|s| $older_than == null or (do $age $s.started) >= $older_than }
  | where {|s| $repo == null or $s.repo == $repo }
  | where {|s| $match == null or $s.name =~ $match or $s.title =~ $match }
  if ($matches | is-empty) {
    print "no sessions match"
    return
  }
  print ($matches | each {|s|
    {
      name: $s.name
      title: ($s.title | str substring ..<50)
      repo: $s.repo
      status: $s.status
      "last message": (if $s.message == null { "" } else { $s.message * 1_000_000 | into datetime | date humanize })
    }
  })
  if $dry_run { return }
  if $yes or ((input $"kill these ($matches | length) sessions? [y/N] ") in [y Y]) {
    ^ssh beast shpool kill ...($matches | each { $in.name | to json })
  }
}

# Opens a foot window for every detached session, skipping unused spares.
export def restore [
  --dry-run (-n) # list the windows without opening them
] {
  let windows = list
  | where $it.status == "disconnected" and not $it.spare
  | each {|s| { session: $s.name, title: (window-title $s) } }
  if $dry_run { return $windows }
  for window in $windows {
    let command = $"use ($self); beast attach ($window.session | to json) --title ($window.title | to json)"
    ^setsid -f foot --title $window.title nu --no-config-file --commands $command o+e> /dev/null
  }
}
