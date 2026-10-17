import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Wayland
import "Protocol.js" as Protocol

PanelWindow {
    id: launcher

    property ShellStyle appearance
    required property var pinStore
    required property var targetScreen
    property bool opened: false
    readonly property var matches: DesktopEntries.applications.values.filter(function(entry) {
        return Protocol.matchesApp(entry, search.text);
    }).sort(function(a, b) {
        return a.name.localeCompare(b.name);
    }).slice(0, 100)

    function open() {
        opened = true;
        search.text = "";
        Qt.callLater(function() {
            search.forceActiveFocus();
        });
    }

    function dismiss() {
        opened = false;
    }

    function launch(entry) {
        if (!entry)
            return ;

        entry.execute();
        dismiss();
    }

    screen: targetScreen
    // Keep the layer role alive after first opening. Destroying a visible layer
    // surface on hide disconnects Quickshell from this nested compositor.
    visible: targetScreen !== null
    // Nonexclusive layer surfaces are placed inside the panel-reduced area.
    margins.top: appearance.popupGap + 8
    implicitWidth: opened ? Math.max(1, Math.min(560, targetScreen ? targetScreen.width - 24 : 560)) : 1
    implicitHeight: opened ? Math.max(1, Math.min(520, targetScreen ? targetScreen.height - appearance.panelTopMargin - appearance.panelHeight - margins.top - 16 : 520)) : 1
    exclusiveZone: 0
    color: "transparent"
    WlrLayershell.namespace: appearance.glass ? "clear-glass-rounded-launcher" : "clear-app-launcher"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: opened ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None

    anchors {
        top: true
    }

    Rectangle {
        visible: launcher.opened
        anchors.fill: parent
        color: launcher.appearance.surfaceColor
        radius: launcher.appearance.overlayRadius
        border.width: launcher.appearance.outlineWidth
        border.color: launcher.appearance.outlineColor
        antialiasing: true
    }

    ColumnLayout {
        visible: launcher.opened
        anchors.fill: parent
        anchors.margins: 20
        spacing: 14

        RowLayout {
            Layout.fillWidth: true

            Text {
                text: "Applications"
                color: launcher.appearance.textColor
                font.pixelSize: 20
                font.weight: Font.DemiBold
            }

            Item {
                Layout.fillWidth: true
            }

            PanelButton {
                appearance: launcher.appearance
                iconSource: "icons/close.svg"
                quiet: true
                description: "Close launcher"
                onClicked: launcher.dismiss()
            }

        }

        TextField {
            id: search

            Layout.fillWidth: true
            implicitHeight: 48
            leftPadding: 42
            rightPadding: 14
            placeholderText: "Search apps…"
            placeholderTextColor: launcher.appearance.mutedColor
            color: launcher.appearance.textColor
            selectionColor: launcher.appearance.cardSelectedColor
            selectedTextColor: launcher.appearance.textColor
            font.pixelSize: 15
            selectByMouse: true
            onTextChanged: {
                results.currentIndex = 0;
                results.positionViewAtBeginning();
            }
            onAccepted: launcher.launch(results.currentItem ? results.currentItem.entry : launcher.matches[0])
            Keys.onEscapePressed: launcher.dismiss()
            Keys.onDownPressed: results.currentIndex = Math.min(results.count - 1, results.currentIndex + 1)
            Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex - 1)

            Image {
                x: 14
                anchors.verticalCenter: parent.verticalCenter
                width: 18
                height: 18
                source: "icons/search.svg"
            }

            background: Rectangle {
                radius: 14
                color: launcher.appearance.controlColor
                border.width: 1
                border.color: search.activeFocus ? launcher.appearance.accentColor : launcher.appearance.outlineColor
            }

        }

        Text {
            text: search.text.trim() ? "SEARCH RESULTS · " + results.count : "ALL APPLICATIONS"
            color: launcher.appearance.mutedColor
            font.pixelSize: 10
            font.weight: Font.DemiBold
            font.letterSpacing: 1.2
        }

        ListView {
            id: results

            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            spacing: 4
            model: launcher.matches
            currentIndex: 0
            onCurrentIndexChanged: {
                if (currentIndex >= 0)
                    positionViewAtIndex(currentIndex, ListView.Contain);

            }

            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
            }

            delegate: Rectangle {
                id: appResult

                required property int index
                required property var modelData
                readonly property var entry: modelData

                width: ListView.view.width
                height: 60
                radius: 12
                color: ListView.isCurrentItem ? launcher.appearance.cardSelectedColor : hit.containsMouse ? launcher.appearance.hoverColor : "transparent"

                MouseArea {
                    id: hit

                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: launcher.launch(appResult.entry)
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.margins: 10
                    spacing: 12

                    AppIcon {
                        Layout.preferredWidth: 32
                        Layout.preferredHeight: 32
                        icon: entry.icon
                    }

                    Column {
                        Layout.fillWidth: true
                        spacing: 3

                        Text {
                            width: parent.width
                            text: entry.name
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            font.pixelSize: 14
                            font.weight: Font.Medium
                            color: launcher.appearance.textColor
                        }

                        Text {
                            width: parent.width
                            text: entry.genericName || entry.id
                            textFormat: Text.PlainText
                            elide: Text.ElideRight
                            font.pixelSize: 11
                            color: launcher.appearance.mutedColor
                        }

                    }

                    PanelButton {
                        appearance: launcher.appearance
                        quiet: true
                        text: launcher.pinStore.ids.indexOf(entry.startupClass || entry.id) === -1 ? "☆" : "★"
                        description: "Pin or unpin " + entry.name
                        onClicked: launcher.pinStore.toggle(entry.startupClass || entry.id)
                    }

                }

                Behavior on color {
                    ColorAnimation {
                        duration: launcher.appearance.motionDuration
                    }

                }

            }

        }

        Text {
            Layout.fillWidth: true
            visible: results.count === 0
            text: "No apps match “" + search.text + "”"
            textFormat: Text.PlainText
            elide: Text.ElideRight
            color: launcher.appearance.mutedColor
            horizontalAlignment: Text.AlignHCenter
        }

        Rectangle {
            Layout.fillWidth: true
            height: 1
            color: launcher.appearance.separatorColor
        }

        Text {
            text: "↑ ↓  Navigate     Enter  Launch     Esc  Close"
            color: launcher.appearance.mutedColor
            font.pixelSize: 11
        }

    }

    appearance: ShellStyle {
    }

    mask: Region {
        radius: launcher.appearance.overlayRadius
        width: launcher.opened ? launcher.width : 0
        height: launcher.opened ? launcher.height : 0
    }

}
