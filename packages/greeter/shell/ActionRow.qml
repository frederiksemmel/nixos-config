import QtQuick

Item {
    id: row

    required property var action
    property bool selected: false

    signal activated
    signal hovered

    height: 44

    Rectangle {
        anchors.fill: parent
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        radius: 8
        color: row.selected ? Theme.selection : "transparent"
    }

    Rectangle {
        id: iconBox

        x: 20
        anchors.verticalCenter: parent.verticalCenter
        width: 26
        height: 26
        radius: 6
        color: Qt.rgba(1, 1, 1, 0.06)

        Image {
            anchors.centerIn: parent
            source: Qt.resolvedUrl(`icons/${row.action.icon}.svg`)
            sourceSize.width: 16
            sourceSize.height: 16
        }
    }

    Text {
        id: title

        anchors.left: iconBox.right
        anchors.leftMargin: 12
        anchors.verticalCenter: parent.verticalCenter
        text: row.action.title
        color: Theme.text
        font.family: Theme.font
        font.pixelSize: 14
    }

    Text {
        anchors.left: title.right
        anchors.leftMargin: 8
        anchors.verticalCenter: parent.verticalCenter
        text: row.action.subtitle ?? ""
        color: Theme.dim
        font.family: Theme.font
        font.pixelSize: 14
    }

    Text {
        anchors.right: parent.right
        anchors.rightMargin: 20
        anchors.verticalCenter: parent.verticalCenter
        text: row.action.accessory ?? ""
        color: Theme.faint
        font.family: Theme.font
        font.pixelSize: 12
    }

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onEntered: row.hovered()
        onClicked: row.activated()
    }
}
