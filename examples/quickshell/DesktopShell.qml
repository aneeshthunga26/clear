import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "Protocol.js" as Protocol

Scope {
    id: root

    property ShellStyle appearance

    ClearBridge {
        id: bridge
    }

    PinnedApps {
        id: pins
    }

    NotificationStore {
        id: notifications
    }

    Variants {
        model: Quickshell.screens

        PanelWindow {
            id: panel

            required property var modelData
            readonly property var output: Protocol.outputForScreen(bridge.state, screen ? screen.name : "")
            readonly property bool active: bridge.online && output !== null
            readonly property bool switcherHere: !!(bridge.state && bridge.state.switcher && output && bridge.state.switcher.output === output.id)
            readonly property string animationNamespace: root.appearance.glass ? "clear-glass-pill-panel-rounded-popups" : "clear-example-panel"
            property bool switcherLoaded: false

            onSwitcherHereChanged: {
                if (switcherHere)
                    switcherLoaded = true;

            }
            screen: modelData
            implicitHeight: root.appearance.panelHeight
            exclusiveZone: root.appearance.panelHeight
            color: "transparent"
            WlrLayershell.namespace: panel.animationNamespace
            WlrLayershell.layer: WlrLayer.Top
            WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

            LazyLoader {
                id: launcherLoader

                AppLauncher {
                    appearance: root.appearance
                    pinStore: pins
                    targetScreen: panel.screen
                }

            }

            LazyLoader {
                id: notificationsLoader

                NotificationCenter {
                    appearance: root.appearance
                    store: notifications
                    targetScreen: panel.screen
                }

            }

            LazyLoader {
                id: switcherLoader

                active: panel.switcherLoaded

                Switcher {
                    appearance: root.appearance
                    shellBridge: bridge
                    targetScreen: panel.screen
                }

            }

            anchors {
                top: true
                left: true
                right: true
            }

            margins {
                top: root.appearance.panelTopMargin
                left: root.appearance.panelSideMargin
                right: root.appearance.panelSideMargin
            }

            Rectangle {
                id: panelBackground

                anchors.fill: parent
                radius: root.appearance.panelRadius
                color: root.appearance.panelColor
                border.width: root.appearance.outlineWidth
                border.color: root.appearance.outlineColor
                antialiasing: true
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: root.appearance.panelPadding
                anchors.rightMargin: root.appearance.panelPadding
                spacing: panel.screen && panel.screen.width < 600 ? 4 : 8

                PanelButton {
                    appearance: root.appearance
                    iconSource: "icons/launch.svg"
                    quiet: true
                    tooltipBelowPanel: true
                    description: "Open application launcher"
                    onClicked: {
                        dock.dismissPreview();
                        if (notificationsLoader.item)
                            notificationsLoader.item.opened = false;

                        launcherLoader.active = true;
                        launcherLoader.item.open();
                    }
                }

                Rectangle {
                    implicitWidth: Math.min(workspaceRow.implicitWidth + 6, panel.screen && panel.screen.width < 600 ? 76 : panel.width / 4)
                    implicitHeight: 34
                    radius: 12
                    color: root.appearance.controlColor

                    Flickable {
                        id: workspaceViewport

                        anchors.fill: parent
                        anchors.margins: 3
                        contentWidth: workspaceRow.implicitWidth
                        contentHeight: 28
                        flickableDirection: Flickable.HorizontalFlick
                        boundsBehavior: Flickable.StopAtBounds
                        clip: true

                        Row {
                            id: workspaceRow

                            spacing: 2

                            Repeater {
                                model: bridge.state ? bridge.state.workspaces : []

                                delegate: PanelButton {
                                    required property var modelData

                                    appearance: root.appearance
                                    text: modelData.id
                                    implicitWidth: 30
                                    implicitHeight: 28
                                    quiet: true
                                    tooltipBelowPanel: true
                                    description: "Workspace " + modelData.name
                                    enabled: panel.active
                                    selected: panel.active && panel.output.workspace === modelData.id
                                    onSelectedChanged: {
                                        if (selected) {
                                            Qt.callLater(function() {
                                            workspaceViewport.contentX = Math.max(0, Math.min(x, workspaceViewport.contentWidth - workspaceViewport.width));
                                        });
                                        }
                                    }
                                    onClicked: bridge.command({
                                        "type": "switch_workspace",
                                        "output": panel.output.id,
                                        "workspace": modelData.id
                                    })
                                }

                            }

                        }

                    }

                }

                Rectangle {
                    width: 1
                    height: 18
                    color: root.appearance.separatorColor
                }

                Flickable {
                    id: dockViewport

                    Layout.fillWidth: true
                    Layout.minimumWidth: 48
                    Layout.preferredHeight: 34
                    contentWidth: dock.implicitWidth
                    contentHeight: 34
                    flickableDirection: Flickable.HorizontalFlick
                    boundsBehavior: Flickable.StopAtBounds
                    clip: true
                    onMovingChanged: {
                        if (moving)
                            dock.dismissPreview();

                    }

                    AppDock {
                        id: dock

                        appearance: root.appearance
                        shellBridge: bridge
                        currentOutput: panel.output
                        panelWindow: panel
                        panelNamespace: panel.animationNamespace
                        viewport: dockViewport
                        pinStore: pins
                    }

                }

                Row {
                    spacing: 6
                    visible: panel.screen && panel.screen.width >= 900

                    ModeIcon {
                        width: 22
                        height: 22
                        anchors.verticalCenter: parent.verticalCenter
                        mode: panel.output ? panel.output.effective_mode : "master_stack"
                    }

                    PanelButton {
                        appearance: root.appearance
                        text: panel.output ? panel.output.effective_mode.replace(/_/g, " ") : "Mode"
                        quiet: true
                        tooltipBelowPanel: true
                        description: "Current mode: " + text + ". Click to cycle."
                        enabled: panel.active
                        selected: panel.active && panel.output.mode_override !== null
                        onClicked: bridge.command({
                            "type": "set_mode",
                            "output": panel.output.id,
                            "mode": Protocol.nextMode(panel.output.effective_mode)
                        })
                    }

                    PanelButton {
                        appearance: root.appearance
                        visible: panel.active && panel.output.mode_override !== null
                        iconSource: "icons/reset.svg"
                        quiet: true
                        tooltipBelowPanel: true
                        description: "Reset this workspace's output mode"
                        onClicked: bridge.command({
                            "type": "clear_mode",
                            "output": panel.output.id
                        })
                    }

                }

                PanelButton {
                    appearance: root.appearance
                    iconSource: "icons/overview.svg"
                    quiet: true
                    tooltipBelowPanel: true
                    description: "Desktop overview"
                    visible: panel.screen && panel.screen.width >= 600
                    enabled: bridge.online
                    selected: bridge.state !== null && bridge.state.overview_open === true
                    onClicked: bridge.command({
                        "type": "toggle_overview"
                    })
                }

                Tray {
                    id: tray

                    appearance: root.appearance
                }

                PanelButton {
                    appearance: root.appearance
                    iconSource: "icons/bell.svg"
                    text: notifications.count > 0 ? String(notifications.count) : ""
                    quiet: true
                    tooltipBelowPanel: true
                    selected: notificationsLoader.item ? notificationsLoader.item.opened : false
                    description: "Open notification tray"
                    onClicked: {
                        notificationsLoader.active = true;
                        dock.dismissPreview();
                        if (launcherLoader.item)
                            launcherLoader.item.dismiss();

                        notificationsLoader.item.toggle();
                    }
                }

                SystemStatus {
                    appearance: root.appearance
                    shellBridge: bridge
                    currentOutput: panel.output
                    compact: !panel.screen || panel.screen.width < 900
                }

            }

            mask: Region {
                item: panelBackground
                radius: panelBackground.radius
            }

        }

    }

    appearance: ShellStyle {
    }

}
