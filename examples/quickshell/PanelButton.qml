import QtQuick
import QtQuick.Controls

Button {
    id: control

    property ShellStyle appearance

    appearance: ShellStyle {
    }

    property bool selected: false
    property string description: text
    property url iconSource: ""
    property bool quiet: false
    property bool tooltipBelowPanel: false

    implicitWidth: Math.max(32, contents.implicitWidth + leftPadding + rightPadding)
    implicitHeight: 32
    leftPadding: 10
    rightPadding: 10
    focusPolicy: Qt.NoFocus
    hoverEnabled: true
    Accessible.name: description
    ShellToolTip {
        target: control
        appearance: control.appearance
        hovered: control.hovered && control.enabled
        text: control.description
        belowPanel: control.tooltipBelowPanel
    }

    contentItem: Item {
        implicitWidth: contents.implicitWidth
        implicitHeight: contents.implicitHeight

        Row {
            id: contents

            anchors.centerIn: parent
            spacing: 6

            Image {
                visible: control.iconSource.toString() !== ""
                source: control.iconSource
                width: visible ? 18 : 0
                height: 18
                anchors.verticalCenter: parent.verticalCenter
                fillMode: Image.PreserveAspectFit
            }

            Text {
                visible: text !== ""
                text: control.text
                textFormat: Text.PlainText
                font.pixelSize: 12
                font.weight: control.selected ? Font.DemiBold : Font.Medium
                color: !control.enabled ? control.appearance.mutedColor : control.selected && !control.appearance.glass && control.iconSource.toString() === "" ? control.appearance.accentTextColor : control.appearance.textColor
                anchors.verticalCenter: parent.verticalCenter
            }

        }

    }

    background: Rectangle {
        radius: control.appearance.glass ? height / 2 : control.appearance.controlRadius
        color: control.down ? control.appearance.pressedColor : control.selected ? (control.iconSource.toString() === "" ? control.appearance.selectedColor : control.appearance.cardSelectedColor) : control.hovered ? control.appearance.hoverColor : control.quiet ? "transparent" : control.appearance.controlColor

        Behavior on color {
            ColorAnimation {
                duration: control.appearance.motionDuration
            }

        }

    }

}
