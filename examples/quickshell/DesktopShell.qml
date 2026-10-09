import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "Protocol.js" as Protocol

Scope {
    id: root
    property ShellStyle appearance: ShellStyle {}

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
                AppLauncher { appearance: root.appearance; pinStore: pins; targetScreen: panel.screen }
            }
            LazyLoader {
                id: notificationsLoader
                NotificationCenter { appearance: root.appearance; store: notifications; targetScreen: panel.screen }
            }
            LazyLoader {
                id: switcherLoader
                active: panel.switcherLoaded
                Switcher { appearance: root.appearance; shellBridge: bridge; targetScreen: panel.screen }
            }

            screen: modelData
            anchors { top: true; left: true; right: true }
            margins { top: root.appearance.panelTopMargin; left: root.appearance.panelSideMargin; right: root.appearance.panelSideMargin }
            implicitHeight: root.appearance.panelHeight
            exclusiveZone: root.appearance.panelHeight
            color: "transparent"
            mask: Region { item: panelBackground; radius: panelBackground.radius }
            WlrLayershell.namespace: root.appearance.glass ? "clear-glass-pill-panel" : "clear-example-panel"
            WlrLayershell.layer: WlrLayer.Top
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

            Rectangle {
                id: panelBackground
                anchors.fill: parent
                radius: root.appearance.glass ? height / 2 : 0
                color: root.appearance.panelColor
                border.width: root.appearance.outlineWidth
                border.color: root.appearance.outlineColor
                antialiasing: true
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: root.appearance.panelPadding
                anchors.rightMargin: root.appearance.panelPadding
                spacing: 6

                PanelButton {
                    appearance: root.appearance
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
                            appearance: root.appearance
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
                        appearance: root.appearance
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
                        appearance: root.appearance
                        text: panel.output ? panel.output.effective_mode.replace("_", " ") : "Mode"
                        description: "Current mode: " + text + ". Click to cycle."
                        enabled: panel.active
                        selected: panel.active && panel.output.mode_override !== null
                        onClicked: bridge.command({type: "set_mode",
                            output: panel.output.id,
                            mode: Protocol.nextMode(panel.output.effective_mode)})
                    }
                    PanelButton {
                        appearance: root.appearance
                        visible: panel.active && panel.output.mode_override !== null
                        text: "↺"
                        description: "Reset this workspace's output mode"
                        onClicked: bridge.command({type: "clear_mode", output: panel.output.id})
                    }
                }

                PanelButton {
                    appearance: root.appearance
                    text: "▦"
                    description: "Desktop overview"
                    enabled: bridge.online
                    selected: bridge.state !== null && bridge.state.overview_open === true
                    onClicked: bridge.command({type: "toggle_overview"})
                }
                Tray { id: tray }
                PanelButton {
                    appearance: root.appearance
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
