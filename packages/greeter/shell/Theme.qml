pragma Singleton

import QtQuick
import Quickshell

// vicinae-dark, as vicinae renders it over a blurred backdrop.
Singleton {
    readonly property color panel: Qt.rgba(30 / 255, 31 / 255, 36 / 255, 0.88)
    readonly property color footer: Qt.rgba(20 / 255, 21 / 255, 26 / 255, 0.92)
    readonly property color border: "#33343c"
    readonly property color selection: Qt.rgba(1, 1, 1, 0.08)
    readonly property color chip: Qt.rgba(1, 1, 1, 0.05)
    readonly property color text: "#e7e5e4"
    readonly property color muted: "#a8a29e"
    readonly property color placeholder: "#78716c"
    readonly property color danger: "#d0654b"
    readonly property color accent: "#b8944e"
    readonly property color green: "#3a9c61"
    readonly property color yellow: "#c9a76e"
    readonly property color red: "#b9543b"

    readonly property string font: "Outfit"
    readonly property int radius: 10
}
