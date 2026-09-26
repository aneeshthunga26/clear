import QtQuick
import QtQuick.Controls
import Quickshell
import "Protocol.js" as Protocol

Item {
    id: dock
    required property var shellBridge
    required property var currentOutput
    required property var panelWindow
    required property var pinStore

    property string hoveredApp: ""
    property int previewX: 0
    property bool previewLoaded: false
    readonly property var groups: Protocol.appGroups(shellBridge ? shellBridge.state : null,
        currentOutput, pinStore ? pinStore.ids : [])
    readonly property var hoveredGroup: (groups || []).find(function(group) {
        return group.id === hoveredApp;
    }) || null
    implicitWidth: appRow.implicitWidth
    implicitHeight: 30

    function showPreview(id, button) {
        previewLoaded = true;
        hoveredApp = id;
        previewX = Math.max(6, Math.min(panelWindow.width - 310,
            panelWindow.mapFromItem(button, 0, 0).x));
        closeDelay.stop();
    }

    Timer {
        id: closeDelay
        interval: 250
        onTriggered: dock.hoveredApp = ""
    }

    Row {
        id: appRow
        spacing: 3
        Repeater {
            model: dock.groups
            delegate: Rectangle {
                id: appButton
                required property var modelData
                readonly property var entry: DesktopEntries.heuristicLookup(modelData.id)
                width: 38
                height: 29
                radius: 5
                color: mouseArea.containsMouse ? "#334155"
                    : modelData.windows.some(function(window) { return window.focused; }) ? "#1d4ed8" : "#1e293b"

                Image {
                    anchors.centerIn: parent
                    width: 20
                    height: 20
                    source: Quickshell.iconPath(entry && entry.icon ? entry.icon : "application-x-executable")
                    fillMode: Image.PreserveAspectFit
                }
                Rectangle {
                    visible: modelData.windows.length > 0
                    anchors.bottom: parent.bottom
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(18, 4 + modelData.windows.length * 4)
                    height: 2
                    radius: 1
                    color: "#93c5fd"
                }
                MouseArea {
                    id: mouseArea
                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onEntered: dock.showPreview(appButton.modelData.id, appButton)
                    onExited: closeDelay.restart()
                    onClicked: mouse => {
                        if (mouse.button === Qt.RightButton) {
                            dock.pinStore.toggle(appButton.modelData.id);
                            return;
                        }
                        var windows = appButton.modelData.windows;
                        if (windows.length) dock.shellBridge.command({type: "focus_window", window: windows[0].id});
                        else if (appButton.entry) appButton.entry.execute();
                    }
                    ToolTip.visible: containsMouse && !(previewLoader.item && previewLoader.item.visible)
                    ToolTip.text: modelData.id + (modelData.pinned ? " · pinned" : "") + " · right click to pin/unpin"
                }
            }
        }
    }

    LazyLoader {
        id: previewLoader
        active: dock.previewLoaded
        PopupWindow {
            id: preview
            visible: !!dock.hoveredGroup && dock.hoveredGroup.windows.length > 0
            color: "#111827"
            implicitWidth: 310
            implicitHeight: Math.min(360, 42 + (dock.hoveredGroup ? dock.hoveredGroup.windows.length : 0) * 62)
            anchor.window: dock.panelWindow
            anchor.rect.x: dock.previewX
            anchor.rect.y: 36
            grabFocus: false

            Column {
                anchors.fill: parent
                anchors.margins: 8
                spacing: 5

                Text {
                    text: dock.hoveredGroup ? dock.hoveredGroup.id : ""
                    color: "#93c5fd"
                    font.bold: true
                    textFormat: Text.PlainText
                }
                ListView {
                    width: parent.width
                    height: parent.height - 26
                    clip: true
                    spacing: 4
                    model: dock.hoveredGroup ? dock.hoveredGroup.windows : []
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width
                        height: 56
                        radius: 5
                        color: modelData.focused ? "#1e40af" : hover.containsMouse ? "#334155" : "#1e293b"
                        Column {
                            anchors.fill: parent
                            anchors.margins: 7
                            spacing: 2
                            Text {
                                width: parent.width
                                text: modelData.title || modelData.app_id || "Untitled"
                                textFormat: Text.PlainText
                                color: "#f8fafc"
                                elide: Text.ElideRight
                            }
                            Text {
                                text: "Workspace " + modelData.workspace
                                color: "#94a3b8"
                                font.pixelSize: 11
                            }
                        }
                        MouseArea {
                            id: hover
                            anchors.fill: parent
                            hoverEnabled: true
                            onEntered: closeDelay.stop()
                            onExited: closeDelay.restart()
                            onClicked: {
                                dock.shellBridge.command({type: "focus_window", window: modelData.id});
                                dock.hoveredApp = "";
                            }
                        }
                    }
                }
            }
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.NoButton
                hoverEnabled: true
                onEntered: closeDelay.stop()
                onExited: closeDelay.restart()
            }
        }
    }
}
