import QtQuick

QtObject {
    property bool glass: false
    readonly property int panelHeight: 44
    readonly property int panelTopMargin: glass ? 10 : 8
    readonly property int panelSideMargin: 12
    readonly property int panelPadding: glass ? 24 : 10
    readonly property int panelRadius: glass ? panelHeight / 2 : 16
    readonly property int overlayRadius: glass ? 24 : 20
    readonly property int controlRadius: 10
    readonly property int popupGap: 8
    readonly property int hoverDelay: 1000
    readonly property int tooltipDelay: 1000
    readonly property int motionDuration: 140
    readonly property int outlineWidth: 1
    readonly property color outlineColor: Qt.rgba(1, 1, 1, glass ? 0.25 : 0.1)
    readonly property color textColor: "#edf2fa"
    readonly property color mutedColor: "#a5b1c6"
    readonly property color accentColor: "#b8caff"
    readonly property color accentTextColor: "#182443"
    readonly property color separatorColor: Qt.rgba(1, 1, 1, 0.1)
    // Alpha belongs to fills, so text and icons stay sharp over the glass.
    readonly property color panelColor: glass ? Qt.rgba(35 / 255, 40 / 255, 52 / 255, 0.18) : "#ed171c29"
    readonly property color surfaceColor: glass ? Qt.rgba(35 / 255, 40 / 255, 52 / 255, 0.24) : "#f51c2232"
    readonly property color controlColor: glass ? Qt.rgba(1, 1, 1, 0.06) : "#252d40"
    readonly property color hoverColor: glass ? Qt.rgba(1, 1, 1, 0.16) : "#35415a"
    readonly property color pressedColor: glass ? Qt.rgba(1, 1, 1, 0.24) : "#465574"
    readonly property color selectedColor: glass ? Qt.rgba(0.4, 0.65, 1, 0.3) : accentColor
    readonly property color cardSelectedColor: glass ? selectedColor : "#303f60"
}
