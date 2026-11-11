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

# Evaluates a nushell expression on beast with sessions.nu loaded. An error there
# comes back as data and is raised here, so this terminal renders it.
def on-beast [expression: string] {
  let script = $"(open --raw $sessions_nu | decode utf-8)\ntry { { value: \(($expression)\) } } catch {|e| { error: $e.msg, help: $e.details.help? } } | to json"
  let result = if (on-beast-itself) {
    ^nu --no-config-file --commands $script | from json
  } else {
    ^ssh ...$connect_options beast $script | from json
  }
  if "error" in $result {
    error make --unspanned { msg: $result.error, help: $result.help }
  }
  $result.value
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
# marked with what hosts it, opening it moves it into the daemon. Tabs separate
# group, title, age and state, repo, so the search can skip age and state.
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
    let status = [(ago $s.last | fill -w 14) ($s.state | fill -w 7) ($marker | fill -w 4)] | str join "  "
    let line = [
      ($s.title | str substring ..<$title_width | fill -w $title_width)
      $status
      $s.repo
      $copies
    ] | str join " \t" | str substring ..<$columns
    $"($label)\t($line)"
  }
}

# Picks lines in fzf and returns their indexes, none on esc. Lines may hold tabs,
# shown one column wide; `nth` limits the search to those tab-separated fields.
# Matches rank by score, ties and the empty query keep the given order.
def fzf-pick [header: string, --multi, --nth: string = ".."] {
  let lines = $in | enumerate | each {|line| $"($line.index)\t($line.item)" } | str join "\n"
  try {
    $lines
    | ^fzf ...(if $multi { [--multi] } else { [] }) --ansi --tiebreak index --layout reverse --height "~100%" --gutter " " --info inline-right --delimiter "\t" --tabstop 1 --with-nth "2.." --nth $nth --accept-nth 1 --header $header
    | lines
    | each { into int }
  } catch { [] }
}

# Ends sessions on beast, prints each one that stayed and why, and returns how
# many stayed.
def end-sessions [sessions: list<string>] {
  let failed = on-beast $"stop-sessions ($sessions | to json --raw)" | where error != null
  for f in $failed {
    print --stderr $"(ansi red)($f.title) \(($f.id)\) was not ended:(ansi reset) ($f.error)"
  }
  $failed | length
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

# Makes a session attachable, moving it into the daemon if it runs elsewhere, and
# returns its id and title. With --wait a working session is asked to pause and
# is taken once it is idle or waits on a prompt; ctrl-c gives up.
def take [session: string, --wait] {
  if $wait {
    let s = on-beast $"pause-session ($session | to json)"
    if $s.reply != null {
      let asked = if $s.reply == "sent" { "asked it to pause at the next safe point" } else { $"could not ask it to pause: ($s.reply)" }
      print $"(ansi attr_dimmed)($s.title) is working, ($asked). Waiting until it is idle, ctrl-c gives up.(ansi reset)"
      let started = date now
      loop {
        let status = on-beast $"session-status ($s.pid)"
        if $status in [idle waiting] { break }
        if $status == "gone" {
          print ""
          error make --unspanned { msg: $"($s.title) ended while waiting" }
        }
        print --no-newline $"\r(ansi attr_dimmed)waited (((date now) - $started) // 1sec * 1sec), it is ($status)(ansi reset)\e[K"
        sleep 3sec
      }
      print ""
    }
  }
  on-beast $"open-session ($session | to json)"
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
  let choice = $rows | append ["new session" "end sessions…"] | fzf-pick --nth "1,2,4" "type to filter, esc for a plain shell" | get -o 0
  if $choice == null {
    if not (on-beast-itself) {
      do --ignore-errors { ^ssh -t beast "hide-env SSH_CONNECTION; exec nu --login" }
      exit
    }
  } else if $choice < $count {
    let session = $sessions | get $choice
    connect (take $session.session --wait=($session.state == "working") | get id) $session.title
  } else if $choice == $count {
    new
  } else {
    let picked = $rows | fzf-pick --multi --nth "1,2,4" "tab marks, enter ends them, esc cancels"
    if ($picked | is-not-empty) {
      if (end-sessions ($picked | each {|i| $sessions | get $i | get session })) > 0 {
        input $"(ansi attr_dimmed)enter goes back to the list(ansi reset)" | ignore
      }
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
# A session that is working is refused, unless --wait: then a hidden Claude asks
# it to pause at its next safe point and it is taken once it is idle. The picker
# waits like that by itself. A session waiting on a prompt is taken right away,
# the prompt is dropped and the terminal answers it instead.
export def attach [
  session: string@complete-sessions # session id, or its start
  --title: string                   # window title, default the session's title
  --wait (-w)                       # ask a working session to pause, take it once idle
] {
  let target = take $session --wait=$wait
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
    let width = $repos.name | each { str length } | math max
    let rows = $repos | each {|r|
      let age = if $r.last == null { "" } else { $"(ansi attr_dimmed)(ago $r.last)(ansi reset)" }
      $"($r.name | fill -w $width) \t($age)"
    }
    let choice = $rows | fzf-pick --nth 1 "new session in, esc cancels" | get -o 0
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
  let args = [$target.path $target.name $name] | compact | each { to json } | str join " "
  let created = on-beast $"new-session ($args)"
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
    let stayed = end-sessions $matches.session
    print $"ended (($matches | length) - $stayed) of ($matches | length) sessions"
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
