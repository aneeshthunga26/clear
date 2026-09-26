import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "Protocol.js" as Protocol

PanelWindow {
    id: launcher
    required property var pinStore
    required property var targetScreen
    property bool opened: false
    readonly property var matches: DesktopEntries.applications.values
        .filter(function(entry) { return Protocol.matchesApp(entry, search.text); })
        .sort(function(a, b) { return a.name.localeCompare(b.name); })
        .slice(0, 100)

    screen: targetScreen
    // Keep the layer role alive after first opening. Destroying a visible layer
    // surface on hide disconnects Quickshell from this nested compositor.
    visible: targetScreen !== null
    anchors { top: true }
    margins.top: 64
    implicitWidth: opened ? Math.min(540, targetScreen ? targetScreen.width - 24 : 540) : 1
    implicitHeight: opened ? 480 : 1
    exclusiveZone: 0
    color: opened ? "#111827" : "transparent"
    mask: Region { width: launcher.opened ? launcher.width : 0; height: launcher.opened ? launcher.height : 0 }
    WlrLayershell.namespace: "clear-app-launcher"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: opened ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    function open() {
        opened = true;
        search.text = "";
        Qt.callLater(function() { search.forceActiveFocus(); });
    }
    function dismiss() { opened = false; }
    function launch(entry) {
        if (!entry) return;
        entry.execute();
        dismiss();
    }

    ColumnLayout {
        visible: launcher.opened
        anchors.fill: parent
        anchors.margins: 14
        spacing: 10

        RowLayout {
            Layout.fillWidth: true
            Text {
                text: "Applications"
                color: "#f8fafc"
                font.pixelSize: 17
                font.bold: true
            }
            Item { Layout.fillWidth: true }
            PanelButton { text: "×"; description: "Close launcher"; onClicked: launcher.dismiss() }
        }

        TextField {
            id: search
            Layout.fillWidth: true
            placeholderText: "Search applications"
            selectByMouse: true
            onAccepted: launcher.launch(results.currentItem ? results.currentItem.entry : launcher.matches[0])
            Keys.onEscapePressed: launcher.dismiss()
            Keys.onDownPressed: results.currentIndex = Math.min(results.count - 1, results.currentIndex + 1)
            Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex - 1)
        }

        ListView {
            id: results
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 3
            model: launcher.matches
            currentIndex: 0
            delegate: Rectangle {
                required property var modelData
                readonly property var entry: modelData
                width: ListView.view.width
                height: 44
                radius: 5
                color: ListView.isCurrentItem || hit.containsMouse ? "#334155" : "#1e293b"
                Row {
                    anchors.fill: parent
                    anchors.margins: 7
                    spacing: 10
                    Image {
                        width: 28; height: 28
                        source: Quickshell.iconPath(entry.icon || "application-x-executable")
                        fillMode: Image.PreserveAspectFit
                    }
                    Column {
                        width: parent.width - 72
                        Text {
                            width: parent.width
                            text: entry.name
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            color: "#f8fafc"
                        }
                        Text {
                            width: parent.width
                            text: entry.genericName || entry.id
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            font.pixelSize: 11
                            color: "#94a3b8"
                        }
                    }
                    PanelButton {
                        text: launcher.pinStore.ids.indexOf(entry.startupClass || entry.id) === -1 ? "☆" : "★"
                        description: "Pin or unpin " + entry.name
                        onClicked: launcher.pinStore.toggle(entry.startupClass || entry.id)
                    }
                }
                MouseArea {
                    id: hit
                    anchors.fill: parent
                    hoverEnabled: true
                    z: -1
                    onClicked: launcher.launch(entry)
                }
            }
        }
        Text {
            Layout.fillWidth: true
            visible: results.count === 0
            text: "No applications found"
            color: "#94a3b8"
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
