import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland

PanelWindow {
    id: switcher
    property ShellStyle appearance: ShellStyle {}
    required property var shellBridge
    required property var targetScreen
    readonly property var selection: shellBridge && shellBridge.state ? shellBridge.state.switcher : null
    readonly property var selectedOutput: selection && shellBridge.state
        ? shellBridge.state.outputs.find(function(output) { return output.id === selection.output; }) : null
    readonly property bool activeSelection: !!selectedOutput
        && !!targetScreen && selectedOutput.name === targetScreen.name

    screen: targetScreen
    visible: targetScreen !== null
    anchors { top: true }
    margins.top: 110
    implicitWidth: activeSelection ? Math.min(600, targetScreen ? targetScreen.width - 24 : 600) : 1
    implicitHeight: activeSelection ? 110 : 1
    exclusiveZone: 0
    color: "transparent"
    mask: Region { radius: switcher.appearance.glass ? switcher.height / 2 : 0; width: switcher.activeSelection ? switcher.width : 0; height: switcher.activeSelection ? switcher.height : 0 }
    WlrLayershell.namespace: appearance.glass ? "clear-glass-pill-switcher" : "clear-switcher"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    Rectangle {
        visible: switcher.activeSelection
        anchors.fill: parent
        color: switcher.appearance.surfaceColor
        radius: switcher.appearance.glass ? height / 2 : 0
        border.width: switcher.appearance.outlineWidth
        border.color: switcher.appearance.outlineColor
        antialiasing: true
    }

    ListView {
        visible: switcher.activeSelection
        anchors.centerIn: parent
        width: parent.width - (switcher.appearance.glass ? 110 : 20)
        height: 86
        orientation: ListView.Horizontal
        clip: true
        spacing: 6
        model: switcher.activeSelection ? switcher.selection.windows : []
        currentIndex: switcher.activeSelection
            ? switcher.selection.windows.indexOf(switcher.selection.selected) : -1
        onCurrentIndexChanged: if (currentIndex >= 0)
            positionViewAtIndex(currentIndex, ListView.Center)
        delegate: Rectangle {
            required property var modelData
            width: 110
            height: 82
            radius: 6
            color: ListView.isCurrentItem ? switcher.appearance.selectedColor : switcher.appearance.controlColor
            readonly property var window: shellBridge.state.windows.find(function(item) { return item.id === modelData; })
            readonly property var entry: window && window.app_id
                ? DesktopEntries.heuristicLookup(window.app_id) : null
            Column {
                anchors.fill: parent
                anchors.margins: 6
                spacing: 3
                Item {
                    x: (parent.width - width) / 2
                    width: 32
                    height: 32
                    Image {
                        id: desktopIcon
                        anchors.fill: parent
                        source: entry && entry.icon ? Quickshell.iconPath(entry.icon) : ""
                        fillMode: Image.PreserveAspectFit
                    }
                    Image {
                        anchors.fill: parent
                        visible: desktopIcon.status !== Image.Ready
                        source: "icons/application.svg"
                        fillMode: Image.PreserveAspectFit
                    }
                }
                Text {
                    text: window ? (window.title || window.app_id || "Untitled") : ""
                    color: "#f8fafc"
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    font.pixelSize: 12
                }
                Text {
                    text: entry && entry.name ? entry.name : window ? window.app_id : ""
                    color: "#93c5fd"
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    font.pixelSize: 11
                }
            }
        }
    }
}
