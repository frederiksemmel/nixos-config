import QtQuick
import Quickshell
import Quickshell.Services.UPower

Item {
    id: root

    // The power action waiting for its confirming second click, as in veila.
    property string armed: ""

    function power(action, command, confirm) {
        if (confirm && armed !== action) {
            armed = action;
            disarm.restart();
            return;
        }
        armed = "";
        if (action === "suspend")
            Quickshell.execDetached(command);
        else
            Auth.power(command);
    }

    opacity: 0
    Component.onCompleted: opacity = Qt.binding(() => Auth.leaving ? 0 : 1)

    Behavior on opacity {
        NumberAnimation {
            duration: 300
            easing.type: Easing.OutCubic
        }
    }

    Timer {
        id: disarm

        interval: 5000
        onTriggered: root.armed = ""
    }

    Image {
        anchors.fill: parent
        source: `file://${Config.background}`
        fillMode: Image.PreserveAspectCrop
    }

    Rectangle {
        anchors.fill: parent
        color: "black"
        opacity: Theme.dim
    }

    SystemClock {
        id: clock

        precision: SystemClock.Minutes
    }

    Text {
        anchors.centerIn: parent
        anchors.verticalCenterOffset: -60
        text: Qt.formatTime(clock.date, "HH:mm")
        color: Theme.clock
        font.family: Theme.font
        font.pixelSize: 88
    }

    Text {
        anchors.centerIn: parent
        text: Qt.formatDate(clock.date, "dddd, MMMM d")
        color: Theme.clock
        font.family: Theme.font
        font.pixelSize: 22
    }

    Rectangle {
        id: pill

        property real shift: 0

        anchors.centerIn: parent
        anchors.verticalCenterOffset: 100
        anchors.horizontalCenterOffset: shift
        width: 310
        height: 54
        radius: height / 2
        color: Theme.input
        border.color: Theme.inputBorder
        border.width: 3

        SequentialAnimation {
            id: shake

            NumberAnimation { target: pill; property: "shift"; to: -10; duration: 50 }
            NumberAnimation { target: pill; property: "shift"; to: 8; duration: 70 }
            NumberAnimation { target: pill; property: "shift"; to: -5; duration: 70 }
            NumberAnimation { target: pill; property: "shift"; to: 0; duration: 60 }
        }

        Connections {
            target: Auth

            function onRejected() {
                shake.restart();
            }
        }

        TextInput {
            id: input

            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 18
            anchors.rightMargin: 18
            anchors.verticalCenter: parent.verticalCenter
            readonly property bool masked: echoMode === TextInput.Password

            text: Auth.password
            echoMode: Auth.prompt !== "" && Auth.promptEcho ? TextInput.Normal : TextInput.Password
            readOnly: Auth.busy || Auth.leaving
            color: masked ? "transparent" : Theme.mask
            selectionColor: masked ? "transparent" : Qt.rgba(1, 1, 1, 0.25)
            cursorDelegate: Item {}
            font.family: Theme.font
            font.pixelSize: 16
            focus: true
            clip: true

            // veila's mask: 7px dots on a 16px pitch, 22px in from the pill's edge.
            Row {
                x: 4
                anchors.verticalCenter: parent.verticalCenter
                visible: input.masked
                spacing: 9

                Repeater {
                    model: Math.min(input.text.length, Math.floor((input.width - 4) / 16))

                    Rectangle {
                        width: 7
                        height: 7
                        radius: 3.5
                        color: Theme.mask
                    }
                }
            }

            onTextEdited: {
                Auth.password = text;
                if (Auth.messageIsError)
                    Auth.show("", false);
            }

            Keys.onPressed: event => {
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    Auth.submit();
                } else if (event.key === Qt.Key_Escape) {
                    Auth.cancel();
                    root.armed = "";
                } else {
                    return;
                }
                event.accepted = true;
            }

            // veila's inline status: placeholder, prompt, "Checking..." or the failure.
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width
                visible: input.text === ""
                elide: Text.ElideRight
                text: Auth.busy ? "Checking..." : Auth.message || Auth.prompt || "Password"
                color: Auth.busy ? Theme.pending : Auth.message ? (Auth.messageIsError ? Theme.rejected : Theme.pending) : Theme.placeholder
                font.family: Theme.font
                font.pixelSize: 16
            }
        }
    }

    // veila's top-right row, right edges at the offsets its normandy theme uses.
    Item {
        anchors.fill: parent
        anchors.topMargin: 21

        Chip {
            anchors.right: parent.right
            anchors.rightMargin: 24
            visible: KeyboardLayout.label !== ""
            label: KeyboardLayout.label
        }

        Chip {
            readonly property var battery: UPower.displayDevice
            readonly property int percent: Math.round(battery.percentage * 100)

            anchors.right: parent.right
            anchors.rightMargin: 78
            visible: battery.ready && battery.isLaptopBattery
            icon: battery.state === UPowerDeviceState.Charging ? "battery-charging" : percent >= 80 ? "battery-full" : percent >= 35 ? "battery-medium" : percent >= 15 ? "battery-low" : "battery-warning"
        }

        Chip {
            anchors.right: parent.right
            anchors.rightMargin: 132
            icon: "suspend"
            background: Theme.powerChip
            foreground: root.armed === "suspend" ? Theme.pending : Theme.chipIcon
            interactive: true
            onClicked: root.power("suspend", Config.suspend, false)
        }

        Chip {
            anchors.right: parent.right
            anchors.rightMargin: 186
            icon: "reboot"
            background: Theme.powerChip
            foreground: root.armed === "reboot" ? Theme.pending : Theme.chipIcon
            interactive: true
            onClicked: root.power("reboot", Config.reboot, true)
        }

        Chip {
            anchors.right: parent.right
            anchors.rightMargin: 240
            icon: "power-off"
            background: Theme.powerChip
            foreground: root.armed === "power-off" ? Theme.pending : Theme.chipIcon
            interactive: true
            onClicked: root.power("power-off", Config.poweroff, true)
        }
    }
}
