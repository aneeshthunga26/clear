import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Wayland

PanelWindow {
    id: menu

    required property var shellBridge
    required property var pinStore
    required property var targetScreen
    property ShellStyle appearance
    property string appId: ""
    property string selectedWindow: ""
    property bool opened: false
    property bool showWorkspaces: false
    property real anchorX: 0
    property real anchorY: 0
    // Requested geometry never depends on a previously configured 1×1 surface.
    readonly property real menuWidth: Math.max(1, Math.min(320, (targetScreen ? targetScreen.width : 1280) - 16))
    readonly property real menuX: Math.max(8, Math.min((targetScreen ? targetScreen.width : 1280) - menuWidth - 8, anchorX - menuWidth / 2))
    readonly property real menuHeight: Math.min(actions.implicitHeight + 24, Math.max(1, (targetScreen ? targetScreen.height : 720) - anchorY - 12))
    readonly property rect bodyGeometry: Qt.rect(body.margins.left, body.margins.top, body.width, body.height)
    readonly property var entry: appId ? (DesktopEntries.applications.values.length > 0 ? DesktopEntries.heuristicLookup(appId) : null) : null
    readonly property var windows: shellBridge.state ? shellBridge.state.windows.filter(function(w) {
        return w.role === "normal" && w.app_id === appId;
    }) : []
    readonly property var window: windows.find(function(w) {
        return w.id === selectedWindow;
    }) || windows[0] || null

    function open(id, x, y) {
        appId = id;
        selectedWindow = (windows.find(function(w) {
            return w.focused;
        }) || windows[0] || {
        }).id || "";
        anchorX = x;
        anchorY = y;
        showWorkspaces = false;
        actions.forceLayout();
        opened = true;
        Qt.callLater(function() {
            content.forceActiveFocus();
        });
    }

    function dismiss() {
        opened = false;
    }

    function command(request) {
        dismiss();
        shellBridge.command(request);
    }

    screen: targetScreen
    visible: targetScreen !== null
    implicitWidth: 1
    implicitHeight: 1
    exclusiveZone: -1
    color: "transparent"
    WlrLayershell.namespace: "clear-app-context-dismissal"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    anchors {
        top: true
        left: true
        right: opened
        bottom: opened
    }

    MouseArea {
        visible: menu.opened
        anchors.fill: parent
        acceptedButtons: Qt.AllButtons
        onClicked: menu.dismiss()
    }

    PanelWindow {
        id: body

        screen: menu.targetScreen
        visible: menu.targetScreen !== null
        exclusiveZone: -1
        implicitWidth: menu.opened ? menu.menuWidth : 1
        implicitHeight: menu.opened ? menu.menuHeight : 1
        margins.left: menu.menuX
        margins.top: menu.anchorY
        color: "transparent"
        WlrLayershell.namespace: menu.appearance.glass ? "clear-glass-rounded-context-menu" : "clear-app-context-menu"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: menu.opened ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

        anchors {
            top: true
            left: true
        }

        FocusScope {
            id: content

            // Keep the full layout prepared while the retained window is 1×1.
            width: menu.menuWidth
            height: menu.menuHeight
            opacity: menu.opened ? 1 : 0
            Keys.onEscapePressed: menu.dismiss()

            Rectangle {
                anchors.fill: parent
                radius: menu.appearance.overlayRadius
                color: menu.appearance.surfaceColor
                border.width: 1
                border.color: menu.appearance.outlineColor
            }

            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.AllButtons
            }

            ScrollView {
                anchors.fill: parent
                anchors.margins: 12
                contentWidth: availableWidth
                clip: true

                Column {
                    id: actions

                    width: parent.width
                    spacing: 3

                    Text {
                        width: parent.width
                        height: 30
                        text: menu.entry ? menu.entry.name : menu.appId
                        textFormat: Text.PlainText
                        font.pixelSize: 14
                        font.weight: Font.DemiBold
                        color: menu.appearance.textColor
                        elide: Text.ElideRight
                        leftPadding: 12
                        verticalAlignment: Text.AlignVCenter
                    }

                    Repeater {
                        model: menu.entry ? menu.entry.actions.slice(0, 32) : []

                        delegate: MenuAction {
                            required property var modelData

                            width: actions.width
                            appearance: menu.appearance
                            text: modelData.name
                            onClicked: {
                                menu.dismiss();
                                modelData.execute();
                            }
                        }

                    }

                    MenuAction {
                        width: actions.width
                        appearance: menu.appearance
                        text: "Open new window"
                        visible: !!menu.entry
                        onClicked: {
                            menu.dismiss();
                            menu.entry.execute();
                        }
                    }

                    MenuAction {
                        width: actions.width
                        appearance: menu.appearance
                        text: menu.pinStore.ids.indexOf(menu.appId) >= 0 ? "Unpin from panel" : "Pin to panel"
                        onClicked: {
                            menu.dismiss();
                            menu.pinStore.toggle(menu.appId);
                        }
                    }

                    Rectangle {
                        width: parent.width
                        height: 1
                        color: menu.appearance.separatorColor
                        visible: !!menu.window
                    }

                    Repeater {
                        model: menu.windows.length > 1 ? menu.windows : []

                        delegate: MenuAction {
                            required property var modelData

                            width: actions.width
                            appearance: menu.appearance
                            text: modelData.title || "Untitled window"
                            chosen: menu.window && menu.window.id === modelData.id
                            onClicked: {
                                menu.selectedWindow = modelData.id;
                                menu.showWorkspaces = false;
                            }
                        }

                    }

                    Text {
                        visible: !!menu.window
                        width: parent.width
                        height: 28
                        text: menu.window ? (menu.window.title || "Untitled window") : ""
                        textFormat: Text.PlainText
                        color: menu.appearance.mutedColor
                        font.pixelSize: 11
                        leftPadding: 12
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                    }

                    Repeater {
                        model: menu.window ? [{
                            "text": menu.window.minimized ? "Restore" : "Minimize",
                            "type": "set_minimized",
                            "field": "minimized",
                            "value": !menu.window.minimized
                        }, {
                            "text": menu.window.maximized ? "Unmaximize" : "Maximize",
                            "type": "set_maximized",
                            "field": "maximized",
                            "value": !menu.window.maximized
                        }, {
                            "text": menu.window.fullscreen ? "Leave fullscreen" : "Fullscreen",
                            "type": "set_fullscreen",
                            "field": "fullscreen",
                            "value": !menu.window.fullscreen
                        }] : []

                        delegate: MenuAction {
                            required property var modelData

                            width: actions.width
                            appearance: menu.appearance
                            text: modelData.text
                            onClicked: {
                                const request = {
                                    "type": modelData.type,
                                    "window": menu.window.id
                                };
                                request[modelData.field] = modelData.value;
                                menu.command(request);
                            }
                        }

                    }

                    MenuAction {
                        width: actions.width
                        appearance: menu.appearance
                        visible: !!menu.window
                        text: "Move to workspace  ›"
                        chosen: menu.showWorkspaces
                        onClicked: menu.showWorkspaces = !menu.showWorkspaces
                    }

                    Repeater {
                        model: menu.showWorkspaces && menu.window && menu.shellBridge.state ? menu.shellBridge.state.workspaces : []

                        delegate: MenuAction {
                            required property var modelData

                            width: actions.width
                            appearance: menu.appearance
                            text: "    " + modelData.id + " · " + modelData.name
                            enabled: menu.window && menu.window.workspace !== modelData.id
                            onClicked: menu.command({
                                "type": "move_window_to_workspace",
                                "window": menu.window.id,
                                "workspace": modelData.id
                            })
                        }

                    }

                    MenuAction {
                        width: actions.width
                        appearance: menu.appearance
                        visible: !!menu.window
                        text: "Close window"
                        onClicked: menu.command({
                            "type": "close_window",
                            "window": menu.window.id
                        })
                    }

                    MenuAction {
                        width: actions.width
                        appearance: menu.appearance
                        visible: menu.windows.length > 1
                        text: "Close all windows"
                        onClicked: {
                            const ids = menu.windows.map(function(w) {
                                return w.id;
                            });
                            menu.dismiss();
                            ids.forEach(function(id) {
                                menu.shellBridge.command({
                                    "type": "close_window",
                                    "window": id
                                });
                            });
                        }
                    }

                }

            }

        }

        mask: Region {
            width: menu.opened ? body.width : 0
            height: menu.opened ? body.height : 0
            radius: menu.appearance.overlayRadius
        }

    }

    appearance: ShellStyle {
    }

    // The transparent dismissal layer must never cover the menu's input.
    mask: Region {
        width: menu.opened ? menu.width : 0
        height: menu.opened ? menu.height : 0

        Region {
            intersection: Intersection.Subtract
            x: menu.bodyGeometry.x
            y: menu.bodyGeometry.y
            width: menu.opened ? menu.bodyGeometry.width : 0
            height: menu.opened ? menu.bodyGeometry.height : 0
            radius: menu.appearance.overlayRadius
        }

    }

}
