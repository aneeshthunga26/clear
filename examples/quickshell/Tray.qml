import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Services.SystemTray

Row {
    id: tray
    spacing: 3

    Repeater {
        model: SystemTray.items
        delegate: Item {
            required property var modelData
            width: 28
            height: 28

            Image {
                anchors.centerIn: parent
                width: 19
                height: 19
                source: modelData.icon
                fillMode: Image.PreserveAspectFit
            }
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.LeftButton | Qt.RightButton | Qt.MiddleButton
                hoverEnabled: true
                onClicked: mouse => {
                    if (mouse.button === Qt.MiddleButton) modelData.secondaryActivate();
                    else if (mouse.button === Qt.RightButton || modelData.onlyMenu) {
                        if (modelData.hasMenu)
                            modelData.display(QSWindow.window, parent.x, parent.y + parent.height);
                    } else modelData.activate();
                }
                ToolTip.visible: containsMouse
                ToolTip.text: modelData.tooltipTitle || modelData.title || modelData.id
            }
        }
    }
}
