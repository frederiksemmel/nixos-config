import QtQuick
import Quickshell

Item {
    id: root

    readonly property var actions: [
        {
            title: "Log in",
            subtitle: Config.sessionName,
            accessory: "Session",
            icon: "login"
        },
        {
            title: "Restart",
            accessory: "Power",
            icon: "restart",
            command: Config.reboot
        },
        {
            title: "Shut down",
            accessory: "Power",
            icon: "poweroff",
            command: Config.poweroff
        }
    ]

    function activate(index) {
        Auth.selected = index;
        if (index === 0)
            Auth.submit();
        else
            Auth.power(actions[index].command);
    }

    function move(delta) {
        Auth.selected = (Auth.selected + delta + actions.length) % actions.length;
    }

    opacity: 0
    Component.onCompleted: opacity = Qt.binding(() => Auth.leaving ? 0 : 1)

    Behavior on opacity {
        NumberAnimation {
            duration: 300
            easing.type: Easing.OutCubic
        }
    }

    Image {
        anchors.fill: parent
        source: `file://${Config.background}`
        fillMode: Image.PreserveAspectCrop
    }

    SystemClock {
        id: clock

        precision: SystemClock.Minutes
    }

    Column {
        id: content

        anchors.centerIn: parent
        anchors.verticalCenterOffset: -height * 0.08
        spacing: 36

        Column {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 4

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: Qt.formatTime(clock.date, "HH:mm")
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: 88
                font.weight: Font.Light
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: Qt.formatDate(clock.date, "dddd, d MMMM")
                color: Qt.rgba(1, 1, 1, 0.7)
                font.family: Theme.font
                font.pixelSize: 17
            }
        }

        Item {
            width: 600
            height: panel.height

            Rectangle {
                id: panel

                width: parent.width
                height: search.height + list.height + footer.height + 2
                radius: Theme.radius
                color: Theme.panel
                border.color: Theme.border
                border.width: 1

                SequentialAnimation {
                    id: shake

                    loops: 1
                    NumberAnimation { target: panel; property: "x"; to: -12; duration: 50 }
                    NumberAnimation { target: panel; property: "x"; to: 10; duration: 70 }
                    NumberAnimation { target: panel; property: "x"; to: -6; duration: 70 }
                    NumberAnimation { target: panel; property: "x"; to: 0; duration: 60 }
                }

                Connections {
                    target: Auth

                    function onRejected() {
                        shake.restart();
                    }
                }

                Item {
                    id: search

                    width: parent.width
                    height: 64

                    Rectangle {
                        id: avatar

                        x: 20
                        anchors.verticalCenter: parent.verticalCenter
                        width: 30
                        height: 30
                        radius: 15
                        color: Qt.rgba(1, 1, 1, 0.1)

                        Text {
                            anchors.centerIn: parent
                            text: Config.user.charAt(0).toUpperCase()
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 14
                            font.weight: Font.DemiBold
                        }
                    }

                    TextInput {
                        id: input

                        anchors.left: avatar.right
                        anchors.leftMargin: 14
                        anchors.right: layoutChip.left
                        anchors.rightMargin: 14
                        anchors.verticalCenter: parent.verticalCenter
                        text: Auth.password
                        echoMode: Auth.prompt !== "" && Auth.promptEcho ? TextInput.Normal : TextInput.Password
                        passwordCharacter: "•"
                        readOnly: Auth.busy || Auth.leaving
                        color: Theme.text
                        selectionColor: Qt.rgba(1, 1, 1, 0.2)
                        font.family: Theme.font
                        font.pixelSize: 17
                        focus: true
                        clip: true

                        onTextEdited: {
                            Auth.password = text;
                            Auth.selected = 0;
                            if (Auth.messageIsError)
                                Auth.show("", false);
                        }

                        Keys.onPressed: event => {
                            const ctrl = event.modifiers & Qt.ControlModifier;
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                                root.activate(Auth.selected);
                            } else if (event.key === Qt.Key_Escape) {
                                Auth.cancel();
                                Auth.selected = 0;
                            } else if (event.key === Qt.Key_Down || event.key === Qt.Key_Tab || (ctrl && (event.key === Qt.Key_J || event.key === Qt.Key_N))) {
                                root.move(1);
                            } else if (event.key === Qt.Key_Up || event.key === Qt.Key_Backtab || (ctrl && (event.key === Qt.Key_K || event.key === Qt.Key_P))) {
                                root.move(-1);
                            } else {
                                return;
                            }
                            event.accepted = true;
                        }

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            visible: input.text === ""
                            text: Auth.prompt || `Password for ${Config.user}`
                            color: Theme.faint
                            font: input.font
                        }
                    }

                    Rectangle {
                        id: layoutChip

                        anchors.right: parent.right
                        anchors.rightMargin: 20
                        anchors.verticalCenter: parent.verticalCenter
                        visible: KeyboardLayout.label !== ""
                        width: layoutText.implicitWidth + 16
                        height: 22
                        radius: 5
                        color: Qt.rgba(1, 1, 1, 0.06)

                        Text {
                            id: layoutText

                            anchors.centerIn: parent
                            text: KeyboardLayout.label
                            color: Theme.dim
                            font.family: Theme.mono
                            font.pixelSize: 11
                        }
                    }
                }

                Rectangle {
                    id: rule

                    anchors.top: search.bottom
                    width: parent.width
                    height: 1
                    color: Theme.border
                    clip: true

                    Rectangle {
                        id: progress

                        visible: Auth.busy
                        width: parent.width * 0.25
                        height: 1
                        color: Theme.accent

                        NumberAnimation on x {
                            running: Auth.busy
                            from: -progress.width
                            to: rule.width
                            duration: 900
                            loops: Animation.Infinite
                            easing.type: Easing.InOutQuad
                        }
                    }
                }

                Column {
                    id: list

                    anchors.top: rule.bottom
                    width: parent.width
                    topPadding: 6
                    bottomPadding: 8

                    Text {
                        x: 20
                        height: 28
                        verticalAlignment: Text.AlignVCenter
                        text: `${Config.user}@${Config.host}`
                        color: Theme.dim
                        font.family: Theme.font
                        font.pixelSize: 12
                        font.weight: Font.Medium
                    }

                    Repeater {
                        model: root.actions

                        ActionRow {
                            required property var modelData
                            required property int index

                            width: list.width
                            action: modelData
                            selected: Auth.selected === index
                            onHovered: Auth.selected = index
                            onActivated: root.activate(index)
                        }
                    }
                }

                Rectangle {
                    id: footer

                    anchors.top: list.bottom
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.margins: 1
                    height: 40
                    bottomLeftRadius: Theme.radius - 1
                    bottomRightRadius: Theme.radius - 1
                    color: Theme.footer

                    Rectangle {
                        width: parent.width
                        height: 1
                        color: Theme.border
                    }

                    Image {
                        id: lockIcon

                        x: 19
                        anchors.verticalCenter: parent.verticalCenter
                        source: Qt.resolvedUrl("icons/lock.svg")
                        sourceSize.width: 14
                        sourceSize.height: 14
                    }

                    Text {
                        anchors.left: lockIcon.right
                        anchors.leftMargin: 8
                        anchors.right: hint.left
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        text: Auth.message || (Auth.busy ? "Authenticating…" : "Locked")
                        color: Auth.messageIsError ? Theme.error : Theme.dim
                        elide: Text.ElideRight
                        font.family: Theme.font
                        font.pixelSize: 12
                    }

                    Row {
                        id: hint

                        anchors.right: parent.right
                        anchors.rightMargin: 14
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: root.actions[Auth.selected].title
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 12
                            font.weight: Font.Medium
                        }

                        Rectangle {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 22
                            height: 20
                            radius: 4
                            color: Qt.rgba(1, 1, 1, 0.08)

                            Text {
                                anchors.centerIn: parent
                                text: "↵"
                                color: Theme.dim
                                font.family: Theme.font
                                font.pixelSize: 12
                            }
                        }
                    }
                }
            }
        }
    }
}
