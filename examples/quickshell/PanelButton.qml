import QtQuick
import QtQuick.Controls

Button {
    id: control

    property bool selected: false
    property string description: text

    implicitWidth: Math.max(28, label.implicitWidth + leftPadding + rightPadding)
    implicitHeight: 26
    leftPadding: 7
    rightPadding: 7
    focusPolicy: Qt.NoFocus
    hoverEnabled: true
    Accessible.name: description
    ToolTip.visible: hovered
    ToolTip.text: description
    ToolTip.delay: 600

    contentItem: Text {
        id: label
        text: control.text
        textFormat: Text.PlainText
        font.pixelSize: 12
        color: control.enabled ? "#f1f5f9" : "#94a3b8"
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: 4
        color: control.down ? "#475569" : control.selected ? "#1d4ed8"
            : control.hovered ? "#334155" : "#1e293b"
    }
}
