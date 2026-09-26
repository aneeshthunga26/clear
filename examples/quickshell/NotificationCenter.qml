import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland

PanelWindow {
    id: center
    required property var store
    required property var targetScreen
    property bool opened: false

    screen: targetScreen
    // Retain the role after first opening; hiding the Wayland layer disconnects
    // the Quickshell client on this nested backend.
    visible: targetScreen !== null
    anchors { top: true; right: true }
    margins { top: 40; right: 8 }
    implicitWidth: opened ? Math.min(390, targetScreen ? targetScreen.width - 16 : 390) : 1
    implicitHeight: opened ? Math.min(530, targetScreen ? targetScreen.height - 54 : 530) : 1
    exclusiveZone: 0
    color: opened ? "#111827" : "transparent"
    mask: Region { width: center.opened ? center.width : 0; height: center.opened ? center.height : 0 }
    WlrLayershell.namespace: "clear-notifications"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    function toggle() {
        opened = !opened;
    }

    ColumnLayout {
        visible: center.opened
        anchors.fill: parent
        anchors.margins: 10
        spacing: 8
        RowLayout {
            Text {
                text: "Notifications (" + center.store.count + ")"
                color: "#f8fafc"
                font.bold: true
                font.pixelSize: 15
            }
            Item { Layout.fillWidth: true }
            PanelButton {
                text: "Clear all"
                enabled: center.store.count > 0
                onClicked: center.store.clear()
            }
            PanelButton { text: "×"; description: "Close notifications"; onClicked: center.opened = false }
        }
        Text {
            visible: center.store.count === 0
            Layout.fillWidth: true
            text: "No notifications"
            color: "#94a3b8"
            horizontalAlignment: Text.AlignHCenter
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 7
            model: center.store.items
            delegate: Rectangle {
                id: card
                required property var modelData
                width: ListView.view.width
                height: details.implicitHeight + 16
                radius: 6
                color: "#1e293b"
                Column {
                    id: details
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.margins: 8
                    spacing: 5
                    RowLayout {
                        width: parent.width
                        Text {
                            Layout.fillWidth: true
                            text: card.modelData.appName || "Application"
                            textFormat: Text.PlainText
                            color: "#93c5fd"
                            font.pixelSize: 11
                            elide: Text.ElideRight
                        }
                        PanelButton {
                            text: "×"
                            description: "Dismiss notification"
                            onClicked: card.modelData.dismiss()
                        }
                    }
                    Text {
                        width: parent.width
                        text: card.modelData.summary
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        font.bold: true
                        color: "#f8fafc"
                    }
                    Text {
                        width: parent.width
                        visible: text !== ""
                        text: card.modelData.body
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        color: "#cbd5e1"
                    }
                    Flow {
                        width: parent.width
                        spacing: 5
                        Repeater {
                            model: card.modelData.actions
                            delegate: PanelButton {
                                required property var modelData
                                text: modelData.text
                                onClicked: modelData.invoke()
                            }
                        }
                    }
                }
            }
        }
    }
}
