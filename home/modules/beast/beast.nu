# shpool sessions on beast, in terminal windows that reattach after a dropped connection.

use session-picker.nu *

const self = path self
const shpool_defs = path self shpool-sessions.nu
const connect_options = [-o ConnectTimeout=5 -o BatchMode=yes]
# A shpool session reaches the agent through the attaching client's SSH_AUTH_SOCK,
# and `ssh beast <command>` runs without nushell's env file, which sets it.
const agent_env = '$env.SSH_AUTH_SOCK = $"($env.XDG_RUNTIME_DIR)/gcr/ssh";'
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

def complete-phone-sessions [] {
  let sessions = try { on-beast phone-sessions } catch { [] }
  {
    options: { sort: false, completion_algorithm: fuzzy }
    completions: ($sessions | each {|s| { value: $s.session, description: $s.title } })
  }
}

def complete-groups [] {
  let groups = try { on-beast session-groups } catch { [] }
  { options: { sort: false, completion_algorithm: fuzzy }, completions: $groups }
}

def complete-repos [] {
  let repos = try { on-beast shpool-repos } catch { [] }
  {
    options: { completion_algorithm: fuzzy }
    completions: ($repos | each {|r| { value: $r.name, description: $r.path } })
  }
}

def free-name [prefix: string, taken: list<string>] {
  1..(($taken | length) + 1)
  | each {|n| $"($prefix)($n)" }
  | where $it not-in $taken
  | first
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
      ^ssh -t ...$connect_options beast $agent_env shpool attach ...$mode $name
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
#
# Subcommands:
#   list     Lists the sessions on beast, newest activity first.
#   attach   Attaches this window to a session and keeps it attached across dropped connections.
#   new      Starts a session: Claude with Remote Control in a repo, inside devenv when the repo has one.
#   spare    Attaches this window to the newest detached Remote Control spare.
#   take     Moves a session started in the app into shpool and attaches this window to it.
#   group    Puts a session in a group, or gives every session without one a group.
#   clean    Kills the sessions matching every given filter, after listing them and asking.
#   restore  Opens every detached session fullscreen on its own workspace, skipping unused spares.
export def main [] {
  let found = on-beast "{ sessions: (shpool-sessions), phone: (phone-sessions) }"
  let listed = $found.sessions | append $found.phone | by-group
  let new_entry = "new session"
  let kill_entry = "kill sessions…"
  let count = $listed | length
  let choice = if $count == 0 { $count } else {
    session-rows $listed
    | append [$new_entry $kill_entry]
    | fzf-pick "type to filter, esc for a plain shell"
    | get -o 0
  }
  if $choice == null {
    do --ignore-errors { ^ssh -t beast "hide-env SSH_CONNECTION; exec nu --login" }
    exit
  } else if $choice < $count {
    let session = $listed | get $choice
    if $session.status == "phone" {
      take $session.session
    } else {
      connect $session.name (window-title $session)
    }
  } else if $choice == $count {
    new
  } else {
    let sessions = $listed | where status != "phone"
    let names = session-rows $sessions
    | fzf-pick --multi "tab marks, enter kills, esc cancels"
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
  let default_name = free-name $prefix $taken
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

def pick-phone-session [] {
  let sessions = on-beast phone-sessions
  if ($sessions | is-empty) {
    error make --unspanned { msg: "no phone sessions on beast" }
  }
  let columns = (term size).columns - 4
  let choice = $sessions
  | each {|s|
    [
      ($s.title | str substring ..<60 | fill -w 60)
      ($s.claude | fill -w 4)
      ($s.ago | str replace " ago" "" | fill -w 14)
      ($s.dir | path basename)
    ] | str join "  " | str substring ..<$columns
  }
  | input list --fuzzy --index "take which session (type to filter, esc cancels)"
  if $choice != null { $sessions | get $choice | get session }
}

# Moves a session started in the app into shpool and attaches this window to it.
#
# Without a session it asks for one. Stops the session's process and resumes the conversation in the same directory
# with Remote Control on. The app gets a new conversation for it and the old one
# stops updating. A session that is working is refused until it is idle. The
# generated name becomes the shpool, Claude and Remote Control session name.
export def take [
  session?: string@complete-phone-sessions # session id, default asks for one
  --name (-n): string                      # session name, default generated from its opening request
] {
  let session = if $session == null { pick-phone-session } else { $session }
  if $session == null { return }
  let taken = list | get name
  if $name in $taken {
    error make --unspanned { msg: $"($name) already exists, use beast attach" }
  }
  let claude = on-beast $"stop-phone-session ($session | to json)"
  let wanted = $name | default $claude.name
  let session_name = if $wanted == null {
    free-name $"($claude.repo)-" $taken
  } else if $wanted in $taken {
    free-name $"($wanted)-" $taken
  } else {
    $wanted
  }
  let command = $"claude --resume ($session) --name ($session_name) --remote-control ($session_name)"
  connect $session_name $session_name --create [--dir ($claude.dir | to json) --cmd ($command | to json)]
}

# Puts a session in a group, or gives every session without one a group.
#
# Claude groups a session by itself after its first turn, and the picker sorts by group.
@example "Group the sessions that have none yet" { beast group }
@example "Move a session" { beast group testcases-v5 zurich-casino }
export def group [
  session?: string@complete-sessions # shpool session name or Claude session id
  group?: string@complete-groups     # group to put it in
] {
  if $session == null {
    on-beast label-sessions
  } else if $group == null {
    error make --unspanned { msg: "give the group to put the session in, or no arguments" }
  } else {
    on-beast $"set-session-group ($session | to json) ($group | to json)"
  }
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
  --group: string@complete-groups # in this group
  --match: string              # name or title matches this regex
  --include-attached           # also consider attached sessions
  --include-busy               # also consider Claude sessions that are working
  --dry-run (-n)               # list the matches without killing them
  --yes (-y)                   # kill without asking
] {
  if ([$no_message_for $detached_for $older_than $repo $group $match] | compact | is-empty) {
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
  | where {|s| $group == null or $s.group == $group }
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
      group: $s.group
      status: $s.status
      "last message": (if $s.message == null { "" } else { $s.message * 1_000_000 | into datetime | date humanize })
    }
  })
  if $dry_run { return }
  if $yes or ((input $"kill these ($matches | length) sessions? [y/N] ") in [y Y]) {
    ^ssh beast shpool kill ...($matches | each { $in.name | to json })
  }
}

def niri-window-ids [] {
  ^niri msg --json windows | from json | get id
}

# Index of the last empty workspace on the focused output; niri always keeps one.
def empty-workspace [] {
  let workspaces = ^niri msg --json workspaces | from json
  let output = $workspaces | where is_focused | first | get output
  $workspaces | where output == $output and active_window_id == null | get idx | math max
}

def wait-for-new-window [known: list<int>] {
  for _ in 1..100 {
    if (niri-window-ids | any {|id| $id not-in $known }) { return }
    sleep 50ms
  }
  error make --unspanned { msg: "no foot window appeared within 5 seconds" }
}

# Opens every detached session fullscreen on its own workspace, skipping unused spares.
export def restore [
  --group: string@complete-groups # only this group
  --dry-run (-n) # list the windows without opening them
] {
  let windows = list
  | where $it.status == "disconnected" and not $it.spare
  | where {|s| $group == null or $s.group == $group }
  | each {|s| { session: $s.name, title: (window-title $s) } }
  if $dry_run { return $windows }
  for window in $windows {
    let command = $"use ($self); beast attach ($window.session | to json) --title ($window.title | to json)"
    let known = niri-window-ids
    # A window lands on the workspace focused when it appears, so each one has
    # to appear before the next workspace gets focus.
    ^niri msg action focus-workspace (empty-workspace)
    ^setsid -f foot --fullscreen --title $window.title nu --no-config-file --commands $command o+e> /dev/null
    wait-for-new-window $known
  }
}
