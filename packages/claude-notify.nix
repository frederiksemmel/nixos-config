{
  lib,
  writeShellApplication,
  coreutils,
  jq,
  libnotify,
}:

# Claude Code hook handler. Reads the hook payload on stdin and raises a mako
# notification. Registered for `Notification` and `Stop` in
# `programs.claude-code.settings.hooks`; the event drives urgency and wording.
writeShellApplication {
  name = "claude-notify";

  runtimeInputs = [
    coreutils
    jq
    libnotify
  ];

  # `set -u` is on, so every jq lookup has a `//` fallback: a payload missing an
  # optional field must never abort the hook.
  text = ''
    # No session bus means no notification daemon to talk to, e.g. a session
    # over SSH. Bail out quietly rather than writing notify-send errors into
    # the terminal on every turn.
    if [[ -z ''${DBUS_SESSION_BUS_ADDRESS-} ]]; then
      exit 0
    fi

    # One field per line, in the order read into `fields` below. Whitespace in
    # the message is collapsed so it cannot spill across lines.
    mapfile -t fields < <(
      jq -r '
        .hook_event_name // "Notification",
        .notification_type // "",
        .cwd // "",
        .session_id // "shared",
        ((.message // "") | gsub("\\s+"; " "))
      ' <<<"$(cat)"
    )

    event=''${fields[0]-Notification}
    kind=''${fields[1]-}
    directory=''${fields[2]-}
    session=''${fields[3]-shared}
    message=''${fields[4]-}

    project=$(basename -- "''${directory:-claude}")

    if [[ $event == Stop ]]; then
      # The turn ended, so the prompt is yours again. Kept quiet and brief
      # because this fires every turn, including ones you are watching.
      urgency=low
      timeout=4000
      body="Finished responding"
    else
      urgency=normal
      timeout=0 # stays up until dismissed or replaced
      case "$kind" in
        permission_prompt) body="Waiting for permission" ;;
        idle_prompt) body="Waiting for your input" ;;
        agent_needs_input) body="Subagent needs input" ;;
        elicitation_dialog) body="Waiting on an MCP form" ;;
        *) body="Needs your attention" ;;
      esac
      # The payload message is more specific than our label when present.
      [[ -n $message ]] && body=$message
    fi

    # x-canonical-private-synchronous makes a session's notifications replace
    # each other rather than stacking up one per turn. That also means the
    # short-lived Stop notification clears a lingering permission prompt.
    notify-send \
      --app-name=claude-code \
      --urgency="$urgency" \
      --expire-time="$timeout" \
      --hint="string:x-canonical-private-synchronous:claude-code-$session" \
      "Claude Code · $project" \
      "$body"
  '';

  meta = {
    description = "Claude Code hook that raises a desktop notification when a session wants attention";
    license = lib.licenses.mit;
    mainProgram = "claude-notify";
    platforms = lib.platforms.linux;
  };
}
