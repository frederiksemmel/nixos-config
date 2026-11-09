# Claude sessions on beast, from the laptop or on beast itself.
#
# A session is one Claude conversation. The daemon (`claude --bg`) hosts it, the app
# reaches it through Remote Control, and a terminal attaches with `claude attach`.

const self = path self
const sessions_nu = path self sessions.nu
const connect_options = [-o ConnectTimeout=5]
const reset_terminal = "\e[?25h\e[?1000;1002;1003;1006;2004l\e[<99u"

def on-beast-itself [] {
  (sys host).hostname == "beast"
}

# Evaluates a nushell expression on beast with sessions.nu loaded.
def on-beast [expression: string] {
  let script = $"(open --raw $sessions_nu | decode utf-8)\n($expression) | to json"
  if (on-beast-itself) {
    ^nu --no-config-file --commands $script | from json
  } else {
    ^ssh ...$connect_options beast $script | from json
  }
}

def complete-sessions [] {
  let sessions = try { on-beast sessions } catch { [] }
  {
    options: { sort: false, completion_algorithm: fuzzy }
    completions: ($sessions | each {|s| { value: $s.id, description: $s.title } })
  }
}

def complete-groups [] {
  let groups = try { on-beast session-groups } catch { [] }
  { options: { sort: false, completion_algorithm: fuzzy }, completions: $groups }
}

def complete-repos [] {
  let repos = try { on-beast repos } catch { [] }
  {
    options: { completion_algorithm: fuzzy }
    completions: ($repos | each {|r| { value: $r.name, description: $r.path } })
  }
}

# Orders sessions by group, the group with the newest activity first, and gives
# each group a colour. Sessions without a group come last.
def by-group [] {
  let sessions = $in
  let colours = [cyan yellow magenta green blue]
  $sessions
  | where group != null
  | group-by group --to-table
  | sort-by {|g| $g.items.last | math max } --reverse
  | enumerate
  | each {|g|
    let colour = $colours | get ($g.index mod ($colours | length))
    $g.item.items | sort-by last --reverse | insert colour $colour
  }
  | flatten
  | append ($sessions | where group == null | sort-by last --reverse | insert colour null)
}

def ago [ms: int] {
  $ms * 1_000_000 | into datetime | date humanize | str replace " ago" ""
}

# Picker lines for by-group sessions. Every line carries its group name so typing
# it filters to the group: bold on the group's first line, dimmed on the rest,
# with a bar in the group's colour joining them. A session outside the daemon is
# marked with what hosts it, opening it moves it into the daemon.
def session-rows [sessions: list] {
  if ($sessions | is-empty) { return [] }
  let group_width = $sessions.group | compact | each { str length } | append 0 | math max
  let title_width = [($sessions.title | str length | math max) 50] | math min
  let columns = (term size).columns - $group_width - 8
  $sessions | enumerate | each {|row|
    let s = $row.item
    let group = $s.group | default "" | fill -w $group_width
    let leads = $row.index == 0 or ($sessions | get ($row.index - 1) | get group) != $s.group
    let label = if $s.colour == null { $"($group)  " } else {
      let style = if $leads { ansi $"($s.colour)_bold" } else { ansi attr_dimmed }
      $"($style)($group)(ansi reset) (ansi $s.colour)▌(ansi reset)"
    }
    let marker = if $s.host == "bg" { "" } else { $s.host }
    let copies = if $s.processes > 1 { $"($s.processes) processes!" } else { "" }
    let line = [
      ($s.title | str substring ..<$title_width | fill -w $title_width)
      (ago $s.last | fill -w 14)
      ($s.state | fill -w 7)
      ($marker | fill -w 4)
      $s.repo
      $copies
    ] | str join "  " | str substring ..<$columns
    $"($label) ($line)"
  }
}

# Picks lines in fzf and returns their indexes, none on esc.
def fzf-pick [header: string, --multi] {
  let lines = $in | enumerate | each {|line| $"($line.index)\t($line.item)" } | str join "\n"
  try {
    $lines
    | ^fzf ...(if $multi { [--multi] } else { [] }) --ansi --no-sort --layout reverse --height "~100%" --gutter " " --info inline-right --delimiter "\t" --with-nth "2.." --accept-nth 1 --header $header
    | lines
    | each { into int }
  } catch { [] }
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

# Keeps this terminal on one background session and exits with it. Only ssh
# failing (exit 255) reconnects; on beast itself there is no connection to lose.
def connect [id: string, title: string] {
  set-title $title
  if (on-beast-itself) {
    ^claude attach $id
    return
  }
  mut delay = 1sec
  loop {
    set-title $title
    let started = date now
    let code = try {
      ^ssh -t ...$connect_options beast claude attach $id
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

# Picks a session on beast and attaches this terminal to it.
#
# Esc leaves a plain shell on beast instead.
#
# Subcommands:
#   list     Lists the sessions on beast, newest message first.
#   attach   Attaches this terminal to a session, across dropped connections.
#   new      Starts a session in a repo on beast and attaches to it.
#   group    Puts a session in a group, or groups every session that has none.
#   clean    Ends the sessions matching every given filter, after listing them.
#   restore  Opens every running session in its own window on its own workspace.
export def main [] {
  let sessions = on-beast sessions | by-group
  let rows = session-rows $sessions
  let count = $sessions | length
  let choice = $rows | append ["new session" "end sessions…"] | fzf-pick "type to filter, esc for a plain shell" | get -o 0
  if $choice == null {
    if not (on-beast-itself) {
      do --ignore-errors { ^ssh -t beast "hide-env SSH_CONNECTION; exec nu --login" }
      exit
    }
  } else if $choice < $count {
    let session = $sessions | get $choice
    connect (on-beast $"open-session ($session.session | to json)" | get id) $session.title
  } else if $choice == $count {
    new
  } else {
    let picked = $rows | fzf-pick --multi "tab marks, enter ends them, esc cancels"
    if ($picked | is-not-empty) {
      on-beast $"stop-sessions ($picked | each {|i| $sessions | get $i | get session } | to json --raw)" | ignore
    }
    main
  }
}

# Lists the sessions on beast, newest message first.
export def list [] {
  on-beast sessions | update last {|s| $s.last * 1_000_000 | into datetime }
}

# Attaches this terminal to a session, across dropped connections.
#
# A session started from the app or in a terminal moves into the daemon first: its
# process stops and the conversation resumes in the background with Remote
# Control. The app shows that as a new conversation; don't write in the old one.
# A session that is working is refused until it is idle.
export def attach [
  session: string@complete-sessions # session id, or its start
  --title: string                   # window title, default the session's title
] {
  let target = on-beast $"open-session ($session | to json)"
  connect $target.id ($title | default $target.title)
}

# Starts a session in a repo on beast and attaches this terminal to it.
#
# The session runs in the daemon with Remote Control, so the app shows it too.
export def new [
  repo?: string@complete-repos # repo under ~/repos on beast, or a unique part of its name
  --name (-n): string          # session name, default the title Claude gives it
] {
  let repos = on-beast repos
  let target = if $repo == null {
    let choice = $repos.name | input list --fuzzy --index "new session in (esc cancels)"
    if $choice == null { return }
    $repos | get $choice
  } else {
    let exact = $repos | where name == $repo
    let found = if ($exact | is-empty) { $repos | where name =~ $repo } else { $exact }
    if ($found | length) != 1 {
      error make --unspanned { msg: $"($repo) matches ($found | length) of: ($repos.name | str join ' ')" }
    }
    $found | first
  }
  let created = on-beast $"new-session ($target.path | to json) ($target.name | to json) ($name | to json)"
  connect $created.id $created.title
}

# Puts a session in a group, or groups every session that has none.
#
# beast groups new sessions by itself every few minutes, and the picker sorts by group.
@example "Group the sessions that have none yet" { beast group }
@example "Move a session" { beast group 51ccc78f zurich-casino }
export def group [
  session?: string@complete-sessions # session id, or its start
  group?: string@complete-groups     # group to put it in
] {
  if $session == null {
    on-beast label-sessions
  } else if $group == null {
    error make --unspanned { msg: "give the group to put the session in, or no arguments" }
  } else {
    on-beast $"set-group ($session | to json) ($group | to json)"
  }
}

# Ends the sessions matching every given filter, after listing them and asking.
#
# Working sessions are always kept. Ending one keeps its transcript, so
# `claude --resume` still opens it.
@example "See what has been quiet for three days" { beast clean --quiet-for 3day --dry-run }
@example "Clean one repo" { beast clean --quiet-for 2day --repo blackjackeye }
export def clean [
  --quiet-for: duration           # no message for this long
  --repo: string@complete-repos   # in this repo
  --group: string@complete-groups # in this group
  --match: string                 # title matches this regex
  --stopped                       # only sessions that are already stopped
  --dry-run (-n)                  # list the matches without ending them
  --yes (-y)                      # end them without asking
] {
  if ([$quiet_for $repo $group $match] | compact | is-empty) and not $stopped {
    error make --unspanned { msg: "give at least one filter, see beast clean --help" }
  }
  let matches = on-beast sessions
  | where state != "working"
  | where {|s| $quiet_for == null or (date now) - ($s.last * 1_000_000 | into datetime) >= $quiet_for }
  | where {|s| $repo == null or $s.repo == $repo }
  | where {|s| $group == null or $s.group == $group }
  | where {|s| $match == null or $s.title =~ $match }
  | where {|s| not $stopped or $s.state == "stopped" }
  if ($matches | is-empty) {
    print "no sessions match"
    return
  }
  print ($matches | each {|s|
    { id: $s.id, title: ($s.title | str substring ..<50), repo: $s.repo, group: $s.group, state: $s.state, "last message": (ago $s.last) }
  })
  if $dry_run { return }
  if $yes or ((input $"end these ($matches | length) sessions? [y/N] ") in [y Y]) {
    on-beast $"stop-sessions ($matches.session | to json --raw)" | ignore
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

# Opens every running background session in a fullscreen window on its own workspace.
export def restore [
  --group: string@complete-groups # only this group
  --dry-run (-n)                  # list the windows without opening them
] {
  let windows = on-beast sessions
  | where host == "bg" and state != "stopped"
  | where {|s| $group == null or $s.group == $group }
  | each {|s| { id: $s.id, title: $s.title } }
  if $dry_run { return $windows }
  for window in $windows {
    let command = $"use ($self); beast attach ($window.id | to json) --title ($window.title | to json)"
    let known = niri-window-ids
    # A window lands on the workspace focused when it appears, so each one has
    # to appear before the next workspace gets focus.
    ^niri msg action focus-workspace (empty-workspace)
    ^setsid -f foot --fullscreen --title $window.title nu --no-config-file --commands $command o+e> /dev/null
    wait-for-new-window $known
  }
}
