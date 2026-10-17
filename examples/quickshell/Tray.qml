import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Services.SystemTray

Row {
    id: tray

    property ShellStyle appearance

    appearance: ShellStyle {
    }

    spacing: 3
    Repeater {
        model: SystemTray.items

        delegate: Item {
            id: trayItem

            required property var modelData

            width: 32
            height: 32

            Rectangle {
                anchors.fill: parent
                radius: tray.appearance.controlRadius
                color: pointer.containsMouse ? tray.appearance.hoverColor : "transparent"

                Behavior on color {
                    ColorAnimation {
                        duration: tray.appearance.motionDuration
                    }

                }

            }

            Image {
                anchors.centerIn: parent
                width: 19
                height: 19
                source: modelData.icon
                fillMode: Image.PreserveAspectFit
            }

            MouseArea {
                id: pointer

                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
                hoverEnabled: true
                onClicked: (mouse) => {
                    if (mouse.button === Qt.MiddleButton) {
                        modelData.secondaryActivate();
                    } else if (mouse.button === Qt.RightButton || modelData.onlyMenu) {
                        if (modelData.hasMenu) {
                            const window = trayItem.QsWindow.window;
                            const point = window.mapFromItem(trayItem, 0, 0);
                            modelData.display(window, point.x, window.height + tray.appearance.popupGap);
                        }
                    } else {
                        modelData.activate();
                    }
                }
            }

            ShellToolTip {
                target: trayItem
                appearance: tray.appearance
                hovered: pointer.containsMouse
                belowPanel: true
                text: trayItem.modelData.tooltipTitle || trayItem.modelData.title || trayItem.modelData.id
            }

        }

    }

}
