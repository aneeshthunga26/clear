import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland

PanelWindow {
    id: center

    property ShellStyle appearance
    required property var store
    required property var targetScreen
    property bool opened: false

    function toggle() {
        opened = !opened;
    }

    screen: targetScreen
    // Retain the role after first opening; hiding the Wayland layer disconnects
    // the Quickshell client on this nested backend.
    visible: targetScreen !== null
    implicitWidth: opened ? Math.min(390, targetScreen ? targetScreen.width - 16 : 390) : 1
    implicitHeight: opened ? Math.max(1, Math.min(530, targetScreen ? targetScreen.height - appearance.panelTopMargin - appearance.panelHeight - margins.top - 12 : 530)) : 1
    exclusiveZone: 0
    color: "transparent"
    WlrLayershell.namespace: appearance.glass ? "clear-glass-rounded-notifications" : "clear-notifications"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.None

    anchors {
        top: true
        right: true
    }

    margins {
        top: center.appearance.popupGap
        right: center.appearance.panelSideMargin
    }

    Rectangle {
        visible: center.opened
        anchors.fill: parent
        color: center.appearance.surfaceColor
        radius: center.appearance.overlayRadius
        border.width: center.appearance.outlineWidth
        border.color: center.appearance.outlineColor
        antialiasing: true
    }

    ColumnLayout {
        visible: center.opened
        anchors.fill: parent
        anchors.margins: 18
        spacing: 14

        RowLayout {
            Text {
                text: "Notifications"
                color: center.appearance.textColor
                font.weight: Font.DemiBold
                font.pixelSize: 18
            }

            Item {
                Layout.fillWidth: true
            }

            PanelButton {
                appearance: center.appearance
                text: "Clear all"
                quiet: true
                enabled: center.store.count > 0
                onClicked: center.store.clear()
            }

            PanelButton {
                appearance: center.appearance
                iconSource: "icons/close.svg"
                quiet: true
                description: "Close notifications"
                onClicked: center.opened = false
            }

        }

        Item {
            visible: center.store.count === 0
            Layout.fillWidth: true
            Layout.fillHeight: true

            Column {
                anchors.centerIn: parent
                spacing: 12

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 64
                    height: 64
                    radius: 22
                    color: center.appearance.controlColor

                    Image {
                        anchors.centerIn: parent
                        width: 28
                        height: 28
                        source: "icons/bell.svg"
                    }

                }

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "You're all caught up"
                    color: center.appearance.textColor
                    font.pixelSize: 16
                    font.weight: Font.Medium
                }

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "New notifications will appear here."
                    color: center.appearance.mutedColor
                    font.pixelSize: 12
                }

            }

        }

        ListView {
            visible: center.store.count > 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 10
            model: center.store.items

            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
            }

            delegate: Rectangle {
                id: card

                required property var modelData

                width: ListView.view.width
                height: details.implicitHeight + 24
                radius: 14
                color: center.appearance.controlColor

                Column {
                    id: details

                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.margins: 12
                    spacing: 8

                    RowLayout {
                        width: parent.width

                        Text {
                            Layout.fillWidth: true
                            text: card.modelData.appName || "Application"
                            textFormat: Text.PlainText
                            color: center.appearance.accentColor
                            font.pixelSize: 11
                            elide: Text.ElideRight
                        }

                        PanelButton {
                            appearance: center.appearance
                            iconSource: "icons/close.svg"
                            quiet: true
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
                        font.pixelSize: 14
                        color: center.appearance.textColor
                    }

                    Text {
                        width: parent.width
                        visible: text !== ""
                        text: card.modelData.body
                        textFormat: Text.PlainText
                        wrapMode: Text.Wrap
                        font.pixelSize: 12
                        color: center.appearance.mutedColor
                    }

                    Flow {
                        width: parent.width
                        spacing: 5

                        Repeater {
                            model: card.modelData.actions

                            delegate: PanelButton {
                                required property var modelData

                                appearance: center.appearance
                                text: modelData.text
                                onClicked: modelData.invoke()
                            }

                        }

                    }

                }

            }

        }

    }

    appearance: ShellStyle {
    }

    mask: Region {
        radius: center.appearance.overlayRadius
        width: center.opened ? center.width : 0
        height: center.opened ? center.height : 0
    }

}
