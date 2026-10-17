import QtQuick
import QtQuick.Controls

Item {
    id: control

    property ShellStyle appearance

    appearance: ShellStyle {
    }

    property url icon
    property string description: ""
    property bool dimmed: false

    signal clicked()
    signal scrolled(real delta)

    width: 32
    height: 32
    Accessible.name: description
    Accessible.role: Accessible.Button
    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: pointer.pressed ? control.appearance.pressedColor : pointer.containsMouse ? control.appearance.hoverColor : "transparent"

        Behavior on color {
            ColorAnimation {
                duration: control.appearance.motionDuration
            }

        }

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
        onWheel: (event) => {
            return control.scrolled(event.angleDelta.y);
        }
    }

    ShellToolTip {
        target: control
        appearance: control.appearance
        hovered: pointer.containsMouse
        text: control.description
        belowPanel: true
    }

}
