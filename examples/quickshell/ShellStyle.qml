import QtQuick

QtObject {
    property bool glass: false

    readonly property int panelHeight: glass ? 44 : 36
    readonly property int panelTopMargin: glass ? 10 : 0
    readonly property int panelSideMargin: glass ? 12 : 0
    readonly property int panelPadding: glass ? 24 : 7
    readonly property int overlayRadius: glass ? 24 : 0
    readonly property int outlineWidth: glass ? 1 : 0
    readonly property color outlineColor: Qt.rgba(1, 1, 1, 0.25)

    // Alpha belongs to fills, so text and icons stay sharp over the glass.
    readonly property color panelColor: glass ? Qt.rgba(35 / 255, 40 / 255, 52 / 255, 0.18) : "#e60f172a"
    readonly property color surfaceColor: glass ? Qt.rgba(35 / 255, 40 / 255, 52 / 255, 0.24) : "#e6111827"
    readonly property color controlColor: glass ? Qt.rgba(1, 1, 1, 0.06) : "#1e293b"
    readonly property color hoverColor: glass ? Qt.rgba(1, 1, 1, 0.16) : "#334155"
    readonly property color pressedColor: glass ? Qt.rgba(1, 1, 1, 0.24) : "#475569"
    readonly property color selectedColor: glass ? Qt.rgba(0.4, 0.65, 1, 0.3) : "#1d4ed8"
    readonly property color cardSelectedColor: glass ? selectedColor : "#1e40af"
}
