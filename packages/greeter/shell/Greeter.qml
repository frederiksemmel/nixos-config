import QtQuick
import QtQuick.Effects
import Quickshell

Item {
    id: root

    readonly property var actions: [
        {
            title: "Log in",
            subtitle: Config.sessionName,
            accessory: "Session",
            icon: "login",
            color: Theme.green
        },
        {
            title: "Restart",
            accessory: "Power",
            icon: "restart",
            color: Theme.yellow,
            command: Config.reboot
        },
        {
            title: "Shut down",
            accessory: "Power",
            icon: "poweroff",
            color: Theme.red,
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
        anchors.centerIn: parent
        anchors.verticalCenterOffset: -height * 0.08
        spacing: 40

        Column {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 2

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: Qt.formatTime(clock.date, "HH:mm")
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: 96
                font.weight: Font.Light
            }

            Text {
                anchors.horizontalCenter: parent.horizontalCenter
                text: Qt.formatDate(clock.date, "dddd, d MMMM")
                color: Theme.text
                opacity: 0.75
                font.family: Theme.font
                font.pixelSize: 18
            }
        }

        Item {
            width: 770
            height: panel.height

            RectangularShadow {
                anchors.fill: panel
                radius: panel.radius
                blur: 24
                spread: 0
                offset.y: 6
                color: Qt.rgba(0, 0, 0, 0.45)
            }

            Rectangle {
                id: panel

                width: parent.width
                height: search.height + 1 + list.height + footer.height + 1
                radius: Theme.radius
                color: Theme.panel
                border.color: Theme.border
                border.width: 1

                SequentialAnimation {
                    id: shake

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
                    height: 60

                    TextInput {
                        id: input

                        x: 16
                        width: parent.width - 32
                        anchors.verticalCenter: parent.verticalCenter
                        text: Auth.password
                        echoMode: Auth.prompt !== "" && Auth.promptEcho ? TextInput.Normal : TextInput.Password
                        passwordCharacter: "•"
                        readOnly: Auth.busy || Auth.leaving
                        color: Theme.text
                        selectionColor: Theme.accent
                        selectedTextColor: "#0f1014"
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
                            text: Auth.prompt || `Password for ${Config.user}…`
                            color: Theme.placeholder
                            font: input.font
                        }
                    }
                }

                Rectangle {
                    id: rule

                    anchors.top: search.bottom
                    x: 1
                    width: parent.width - 2
                    height: 1
                    color: Theme.border
                    clip: true

                    Rectangle {
                        id: progress

                        visible: Auth.busy
                        width: parent.width * 0.3
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
                    topPadding: 4
                    bottomPadding: 6

                    Text {
                        x: 16
                        height: 30
                        verticalAlignment: Text.AlignVCenter
                        text: `${Config.user}@${Config.host}`
                        color: Theme.muted
                        font.family: Theme.font
                        font.pixelSize: 13
                        font.weight: Font.DemiBold
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

                        x: 15
                        anchors.verticalCenter: parent.verticalCenter
                        source: Qt.resolvedUrl("icons/lock.svg")
                        sourceSize.width: 14
                        sourceSize.height: 14
                    }

                    Text {
                        anchors.left: lockIcon.right
                        anchors.leftMargin: 9
                        anchors.right: hints.left
                        anchors.rightMargin: 16
                        anchors.verticalCenter: parent.verticalCenter
                        text: Auth.message || (Auth.busy ? "Authenticating…" : "Locked")
                        color: Auth.messageIsError ? Theme.danger : Theme.muted
                        elide: Text.ElideRight
                        font.family: Theme.font
                        font.pixelSize: 13
                    }

                    Row {
                        id: hints

                        anchors.right: parent.right
                        anchors.rightMargin: 14
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: root.actions[Auth.selected].title
                            color: Theme.text
                            font.family: Theme.font
                            font.pixelSize: 13
                            font.weight: Font.Medium
                        }

                        KeyChip {
                            anchors.verticalCenter: parent.verticalCenter
                            icon: "return"
                        }

                        Rectangle {
                            visible: KeyboardLayout.label !== ""
                            anchors.verticalCenter: parent.verticalCenter
                            width: 1
                            height: 14
                            color: Theme.border
                        }

                        Text {
                            visible: KeyboardLayout.label !== ""
                            anchors.verticalCenter: parent.verticalCenter
                            text: "Layout"
                            color: Theme.muted
                            font.family: Theme.font
                            font.pixelSize: 13
                        }

                        KeyChip {
                            visible: KeyboardLayout.label !== ""
                            anchors.verticalCenter: parent.verticalCenter
                            label: KeyboardLayout.label
                        }
                    }
                }
            }
        }
    }
}
