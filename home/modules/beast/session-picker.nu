# Orders sessions by group, the group with the newest activity first, and gives
# each group a colour. Sessions without a group come last.
export def by-group [] {
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

# Picker lines for by-group sessions. Every line carries its group name so typing
# it filters to the group: bold on the group's first line, dimmed on the rest,
# with a bar in the group's colour joining them.
export def session-rows [sessions: list] {
  if ($sessions | is-empty) { return [] }
  let group_width = $sessions.group | compact | each { str length } | append 0 | math max
  let title_width = [($sessions.title | str length | math max) 40] | math min
  let ids_width = $sessions.ids | str length | math max
  let columns = (term size).columns - $group_width - 8
  $sessions | enumerate | each {|row|
    let s = $row.item
    let group = $s.group | default "" | fill -w $group_width
    let leads = $row.index == 0 or ($sessions | get ($row.index - 1) | get group) != $s.group
    let label = if $s.colour == null { $"($group)  " } else {
      let style = if $leads { ansi $"($s.colour)_bold" } else { ansi attr_dimmed }
      $"($style)($group)(ansi reset) (ansi $s.colour)▌(ansi reset)"
    }
    let line = [
      ($s.title | str substring ..<$title_width | fill -w $title_width)
      ($s.ids | fill -w $ids_width)
      ($s.ago | str replace " ago" "" | fill -w 14)
      ($s.status | str replace "disconnected" "detached" | fill -w 8)
      ($s.dir | path basename)
    ] | str join "  " | str substring ..<$columns
    $"($label) ($line)"
  }
}

# Picks lines in fzf and returns their indexes, none on esc.
export def fzf-pick [header: string, --multi] {
  let lines = $in | enumerate | each {|line| $"($line.index)\t($line.item)" } | str join "\n"
  try {
    $lines
    | ^fzf ...(if $multi { [--multi] } else { [] }) --ansi --no-sort --layout reverse --height "~100%" --gutter " " --info inline-right --delimiter "\t" --with-nth "2.." --accept-nth 1 --header $header
    | lines
    | each { into int }
  } catch { [] }
}
