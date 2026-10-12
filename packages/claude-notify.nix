{
  lib,
  writeShellApplication,
  coreutils,
  gnugrep,
  jq,
}:

# Claude Code hook handler, presenting Claude as "Claudio". It never talks to
# the notification daemon itself: it answers with an OSC 99 (kitty desktop
# notification) escape in `terminalSequence`, which Claude Code writes to its
# own terminal. foot turns that into a notification through
# `desktop-notifications.command`, skips it while the window has focus, and
# focuses the right window when it is clicked. Because the escape travels down
# the pty, this works the same over SSH and inside shpool.
#
# Registered for `Notification`, `Stop`, `StopFailure` and `UserPromptSubmit`
# in `programs.claude-code.settings.hooks`. Urgency encodes what Claudio wants
# (mako colours the border by it), and the project and session topic are
# tinted with a colour derived from the session, so parallel sessions are easy
# to tell apart.
writeShellApplication {
  name = "claude-notify";

  runtimeInputs = [
    coreutils
    gnugrep
    jq
  ];

  # `set -u` is on, so every jq lookup has a `//` fallback: a payload missing an
  # optional field must never abort the hook.
  text = ''
    # One field per line, in the order read into `fields` below. Whitespace is
    # collapsed so no field can spill across lines.
    mapfile -t fields < <(
      jq -r '
        def claudio: gsub("Claude Code"; "Claudio") | gsub("\\bClaude\\b"; "Claudio");
        def oneline: gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ");
        def short: if length > 160 then .[0:159] + "…" else . end;
        .hook_event_name // "",
        .notification_type // "",
        .cwd // "",
        .session_id // "",
        .transcript_path // "",
        (.message // "" | claudio | oneline),
        (.last_assistant_message // .error // "" | gsub("[`*#]"; "") | claudio | oneline | short)
      '
    )

    event=''${fields[0]-}
    kind=''${fields[1]-}
    directory=''${fields[2]-}
    session=''${fields[3]-}
    transcript=''${fields[4]-}
    message=''${fields[5]-}
    reply=''${fields[6]-}

    # OSC 99 identifiers allow [A-Za-z0-9_.+-]. One per session, so each new
    # notification replaces the previous one instead of stacking up.
    id=''${session//[^A-Za-z0-9_.+-]/}
    id=claudio-''${id:-shared}
    # One kitty desktop notification escape: OSC 99 ; metadata ; payload ST.
    osc99() {
      printf '\e]99;%s;%s%s' "$1" "$2" $'\e\\'
    }

    close=$(osc99 "i=$id:p=close" "")

    emit() {
      jq -nc --arg seq "$1" '{terminalSequence: $seq}'
    }

    detail=
    expire=0 # stays up until clicked, dismissed or replaced
    case "$event" in
      UserPromptSubmit)
        # You are back at the prompt, so whatever was waiting has been seen.
        emit "$close"
        exit 0
        ;;
      Stop)
        summary="Claudio is done"
        urgency=0
        expire=6000
        detail=$reply
        ;;
      StopFailure)
        summary="Claudio hit an error"
        urgency=2
        detail=$reply
        ;;
      *)
        detail=$message
        case "$kind" in
          permission_prompt)
            summary="Claudio needs permission"
            urgency=2
            ;;
          elicitation_dialog | elicitation_url_dialog)
            summary="Claudio has a question"
            urgency=2
            ;;
          agent_needs_input)
            summary="A Claudio subagent needs you"
            urgency=1
            ;;
          idle_prompt)
            summary="Claudio is waiting for you"
            urgency=1
            detail=
            ;;
          *)
            summary="Claudio wants your attention"
            urgency=1
            ;;
        esac
        ;;
    esac

    project=$(basename -- "''${directory:-claudio}")

    # The session's name: a /rename title wins over the generated one. Read
    # from the end of the transcript, where the latest title entry lives.
    topic=
    if [[ -r $transcript ]]; then
      topic=$(
        tac -- "$transcript" \
          | grep -m1 -E '"type":"(agent-name|ai-title)"' \
          | jq -r '.agentName // .aiTitle // empty | gsub("Claude Code"; "Claudio") | gsub("\\bClaude\\b"; "Claudio") | gsub("\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")'
      ) || true
    fi

    # Readable on mako's dark Claudio background; one per session.
    palette=(e06c75 e5c07b 98c379 56b6c2 61afef c678dd d19a66 ff79c6)
    hash=$(cksum <<<"''${session:-$project}")
    accent=''${palette[''${hash%% *} % ''${#palette[@]}]}

    # The body is Pango markup in mako; everything interpolated is escaped.
    # Replacements are quoted: bash 5.2's patsub_replacement reads a bare `&`
    # as the matched text.
    markup() {
      local s=''${1//&/"&amp;"}
      s=''${s//</"&lt;"}
      printf '%s' "''${s//>/"&gt;"}"
    }

    heading=$(markup "$project")
    [[ -n $topic ]] && heading+=" · $(markup "$topic")"
    body="<span foreground=\"#$accent\" weight=\"bold\">$heading</span>"
    [[ -n $detail ]] && body+=$'\n'$(markup "$detail")

    b64() {
      printf '%s' "$1" | base64 -w0
    }

    # Payloads are base64 (e=1) so no message text can terminate the escape.
    # f= becomes the app name foot hands to the notification daemon.
    title=$(osc99 "i=$id:d=0:e=1:u=$urgency:w=$expire:f=$(b64 Claudio):p=title" "$(b64 "$summary")")
    text=$(osc99 "i=$id:d=1:e=1:p=body" "$(b64 "$body")")

    # Closing first matters when the window is focused: foot then suppresses
    # the new notification, and the stale one must not linger.
    emit "$close$title$text"
  '';

  meta = {
    description = "Claude Code hook that has foot raise a click-to-focus notification when a session wants attention";
    license = lib.licenses.mit;
    mainProgram = "claude-notify";
    platforms = lib.platforms.linux;
  };
}
