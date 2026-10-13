import QtQuick

Rectangle {
    id: chip

    property string label: ""
    property string icon: ""

    implicitWidth: Math.max(height, (icon ? 12 : labelText.implicitWidth) + 12)
    implicitHeight: 22
    radius: 5
    color: Theme.chip
    border.color: Theme.border
    border.width: 1

    Text {
        id: labelText

        anchors.centerIn: parent
        visible: !chip.icon
        text: chip.label
        color: Theme.muted
        font.family: Theme.font
        font.pixelSize: 12
        font.weight: Font.Medium
    }

    Image {
        anchors.centerIn: parent
        visible: !!chip.icon
        source: chip.icon ? Qt.resolvedUrl(`icons/${chip.icon}.svg`) : ""
        sourceSize.width: 12
        sourceSize.height: 12
    }
}
