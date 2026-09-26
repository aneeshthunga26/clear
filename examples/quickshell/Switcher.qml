import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland

PanelWindow {
    id: switcher
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
    color: activeSelection ? "#111827" : "transparent"
    mask: Region { width: switcher.activeSelection ? switcher.width : 0; height: switcher.activeSelection ? switcher.height : 0 }
    WlrLayershell.namespace: "clear-switcher"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    ListView {
        visible: switcher.activeSelection
        anchors.centerIn: parent
        width: parent.width - 20
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
            color: ListView.isCurrentItem ? "#1d4ed8" : "#1e293b"
            readonly property var window: shellBridge.state.windows.find(function(item) { return item.id === modelData; })
            Column {
                anchors.fill: parent
                anchors.margins: 7
                spacing: 6
                Text {
                    text: window ? window.app_id : ""
                    color: "#93c5fd"
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    width: parent.width
                }
                Text {
                    text: window ? (window.title || "Untitled") : ""
                    color: "#f8fafc"
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    width: parent.width
                }
            }
        }
    }
}
