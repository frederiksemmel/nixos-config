pragma Singleton

import QtQuick
import Quickshell

Singleton {
    readonly property color panel: Qt.rgba(0.086, 0.086, 0.094, 0.9)
    readonly property color footer: Qt.rgba(0.063, 0.063, 0.071, 0.9)
    readonly property color border: Qt.rgba(1, 1, 1, 0.08)
    readonly property color selection: Qt.rgba(1, 1, 1, 0.07)
    readonly property color text: "#ececec"
    readonly property color dim: "#8a8a8a"
    readonly property color faint: "#5c5c5c"
    readonly property color error: "#e06c75"
    readonly property color accent: "#98c379"

    readonly property string font: "Inter"
    readonly property string mono: "JetBrains Mono"
    readonly property int radius: 12
}
