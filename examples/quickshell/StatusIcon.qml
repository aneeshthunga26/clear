import QtQuick
import QtQuick.Controls

Item {
    id: control
    property url icon
    property string description: ""
    property bool dimmed: false
    signal clicked()
    signal scrolled(real delta)
    width: 26
    height: 28

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: pointer.containsMouse ? "#334155" : "transparent"
    }
    Image {
        anchors.centerIn: parent
        width: 19
        height: 19
        source: control.icon
        opacity: control.dimmed ? 0.45 : 1
        fillMode: Image.PreserveAspectFit
    }
    MouseArea {
        id: pointer
        anchors.fill: parent
        hoverEnabled: true
        onClicked: control.clicked()
        onWheel: event => control.scrolled(event.angleDelta.y)
        ToolTip.visible: containsMouse
        ToolTip.text: control.description
        ToolTip.delay: 600
    }
}
