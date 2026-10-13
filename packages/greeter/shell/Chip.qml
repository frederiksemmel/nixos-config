import QtQuick

// A round widget in veila's top-right row: an icon or a short label.
Rectangle {
    id: chip

    property string icon: ""
    property string label: ""
    property color background: Theme.chip
    property color foreground: Theme.chipIcon
    property bool interactive: false
    readonly property bool hovered: mouse.containsMouse

    signal clicked

    width: 46
    height: 46
    radius: 23
    color: interactive && (hovered || mouse.pressed) ? Theme.chipHover : background

    Image {
        anchors.centerIn: parent
        visible: chip.icon !== ""
        source: chip.icon ? Qt.resolvedUrl(`icons/${chip.icon}.svg`) : ""
        sourceSize.width: 20
        sourceSize.height: 20
        opacity: chip.foreground.a
    }

    Text {
        anchors.centerIn: parent
        visible: chip.label !== ""
        text: chip.label
        color: chip.foreground
        font.family: Theme.badgeFont
        font.weight: Font.DemiBold
        font.pixelSize: 16
    }

    MouseArea {
        id: mouse

        anchors.fill: parent
        enabled: chip.interactive
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: chip.clicked()
    }
}
