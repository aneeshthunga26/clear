import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "Protocol.js" as Protocol

Scope {
    id: root

    ClearBridge { id: bridge }
    PinnedApps { id: pins }
    NotificationStore { id: notifications }
    Variants {
        model: Quickshell.screens
        PanelWindow {
            id: panel
            required property var modelData
            readonly property var output: Protocol.outputForScreen(bridge.state, screen ? screen.name : "")
            readonly property bool active: bridge.online && output !== null
            readonly property bool switcherHere: !!(bridge.state && bridge.state.switcher
                && output && bridge.state.switcher.output === output.id)
            property bool switcherLoaded: false
            onSwitcherHereChanged: if (switcherHere) switcherLoaded = true

            LazyLoader {
                id: launcherLoader
                AppLauncher { pinStore: pins; targetScreen: panel.screen }
            }
            LazyLoader {
                id: notificationsLoader
                NotificationCenter { store: notifications; targetScreen: panel.screen }
            }
            LazyLoader {
                id: switcherLoader
                active: panel.switcherLoaded
                Switcher { shellBridge: bridge; targetScreen: panel.screen }
            }

            screen: modelData
            anchors { top: true; left: true; right: true }
            implicitHeight: 36
            exclusiveZone: 36
            color: "transparent"
            WlrLayershell.namespace: "clear-example-panel"
            WlrLayershell.layer: WlrLayer.Top
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

            Rectangle {
                anchors.fill: parent
                color: "#e60f172a"
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 7
                anchors.rightMargin: 7
                spacing: 6

                PanelButton {
                    text: "◈"
                    description: "Open application launcher"
                    onClicked: {
                        launcherLoader.active = true;
                        launcherLoader.item.open();
                    }
                }

                Row {
                    spacing: 2
                    Repeater {
                        model: bridge.state ? bridge.state.workspaces : []
                        delegate: PanelButton {
                            required property var modelData
                            text: modelData.id
                            description: "Workspace " + modelData.name
                            enabled: panel.active
                            selected: panel.active && panel.output.workspace === modelData.id
                            onClicked: bridge.command({type: "switch_workspace",
                                output: panel.output.id, workspace: modelData.id})
                        }
                    }
                }

                Rectangle { width: 1; height: 22; color: "#475569" }

                Flickable {
                    Layout.fillWidth: true
                    Layout.minimumWidth: 48
                    Layout.preferredHeight: 30
                    contentWidth: dock.implicitWidth
                    contentHeight: 30
                    flickableDirection: Flickable.HorizontalFlick
                    boundsBehavior: Flickable.StopAtBounds
                    clip: true
                    AppDock {
                        id: dock
                        shellBridge: bridge
                        currentOutput: panel.output
                        panelWindow: panel
                        pinStore: pins
                    }
                }

                Row {
                    spacing: 4
                    ModeIcon {
                        width: 22; height: 22
                        anchors.verticalCenter: parent.verticalCenter
                        mode: panel.output ? panel.output.effective_mode : "master_stack"
                    }
                    PanelButton {
                        text: panel.output ? panel.output.effective_mode.replace("_", " ") : "Mode"
                        description: "Current mode: " + text + ". Click to cycle."
                        enabled: panel.active
                        selected: panel.active && panel.output.mode_override !== null
                        onClicked: bridge.command({type: "set_mode",
                            output: panel.output.id,
                            mode: Protocol.nextMode(panel.output.effective_mode)})
                    }
                    PanelButton {
                        visible: panel.active && panel.output.mode_override !== null
                        text: "↺"
                        description: "Reset this workspace's output mode"
                        onClicked: bridge.command({type: "clear_mode", output: panel.output.id})
                    }
                }

                Tray { id: tray }
                PanelButton {
                    text: "♢ " + notifications.count
                    description: "Open notification tray"
                    onClicked: {
                        notificationsLoader.active = true;
                        notificationsLoader.item.toggle();
                    }
                }
            }
        }
    }
}
