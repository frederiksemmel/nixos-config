pragma Singleton

import QtQuick
import Quickshell

// veila's normandy theme, so login and lock screen look the same.
Singleton {
    readonly property real dim: 0.34

    readonly property color clock: Qt.rgba(1, 1, 1, 0.4)
    readonly property color input: Qt.rgba(1, 1, 1, 0.08)
    readonly property color inputBorder: Qt.rgba(1, 1, 1, 0.16)
    readonly property color mask: "#ffffff"
    readonly property color placeholder: Qt.rgba(1, 1, 1, 0.6)
    readonly property color pending: "#ececec"
    readonly property color rejected: "#ffd5d5"

    readonly property color chip: Qt.rgba(1, 1, 1, 0.05)
    readonly property color powerChip: Qt.rgba(1, 1, 1, 10 / 255)
    readonly property color chipHover: Qt.rgba(1, 1, 1, 34 / 255)
    readonly property color chipIcon: Qt.rgba(1, 1, 1, 0.68)

    readonly property string font: "Google Sans Flex 72pt"
    readonly property string badgeFont: "Geom"
}
