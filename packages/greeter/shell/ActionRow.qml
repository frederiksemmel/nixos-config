import QtQuick

Item {
    id: row

    required property var action
    property bool selected: false

    signal activated
    signal hovered

    height: 38

    Rectangle {
        anchors.fill: parent
        anchors.leftMargin: 6
        anchors.rightMargin: 6
        radius: 6
        color: row.selected ? Theme.selection : "transparent"
    }

    Rectangle {
        id: icon

        x: 16
        anchors.verticalCenter: parent.verticalCenter
        width: 20
        height: 20
        radius: 5
        gradient: Gradient {
            GradientStop {
                position: 0
                color: Qt.lighter(row.action.color, 1.25)
            }
            GradientStop {
                position: 1
                color: row.action.color
            }
        }

        Image {
            anchors.centerIn: parent
            source: Qt.resolvedUrl(`icons/${row.action.icon}.svg`)
            sourceSize.width: 12
            sourceSize.height: 12
        }
    }

    Text {
        id: title

        x: 48
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
        color: Theme.muted
        font.family: Theme.font
        font.pixelSize: 14
    }

    Text {
        anchors.right: parent.right
        anchors.rightMargin: 14
        anchors.verticalCenter: parent.verticalCenter
        text: row.action.accessory ?? ""
        color: Theme.muted
        font.family: Theme.font
        font.pixelSize: 13
    }

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onEntered: row.hovered()
        onClicked: row.activated()
    }
}
