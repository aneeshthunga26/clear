import QtQuick
import QtQuick.Controls

Button {
    id: action
    property ShellStyle appearance: ShellStyle {}
    property bool chosen: false
    implicitHeight: 34
    leftPadding: 12
    rightPadding: 12
    hoverEnabled: true
    Accessible.name: text
    contentItem: Text {
        text: action.text
        textFormat: Text.PlainText
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
        font.pixelSize: 13
        color: action.enabled ? action.appearance.textColor : action.appearance.mutedColor
    }
    background: Rectangle {
        radius: 9
        color: action.down ? action.appearance.pressedColor
            : action.hovered || action.activeFocus ? action.appearance.hoverColor
            : action.chosen ? action.appearance.cardSelectedColor : "transparent"
    }
    Keys.onDownPressed: nextItemInFocusChain().forceActiveFocus()
    Keys.onUpPressed: nextItemInFocusChain(false).forceActiveFocus()
}
