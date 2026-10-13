pragma Singleton

import QtQuick
import Quickshell
import Quickshell.Io

// The active xkb layout, followed through niri's event stream. Typing the
// password in the wrong layout otherwise just fails as a wrong password.
Singleton {
    id: layout

    property var names: []
    property int index: 0
    // "English (US, intl., with dead keys)" -> "US intl"
    readonly property string label: {
        const name = names[index] ?? "";
        const inner = name.match(/\(([^)]*)\)/);
        if (!inner)
            return name;
        return inner[1].split(",").slice(0, 2).map(s => s.trim().replace(/\.$/, "")).join(" ");
    }

    Process {
        command: [Config.niri, "msg", "--json", "event-stream"]
        running: true

        stdout: SplitParser {
            onRead: line => {
                let event;
                try {
                    event = JSON.parse(line);
                } catch (e) {
                    return;
                }
                if (event.KeyboardLayoutsChanged) {
                    layout.names = event.KeyboardLayoutsChanged.keyboard_layouts.names;
                    layout.index = event.KeyboardLayoutsChanged.keyboard_layouts.current_idx;
                } else if (event.KeyboardLayoutSwitched) {
                    layout.index = event.KeyboardLayoutSwitched.idx;
                }
            }
        }
    }
}
