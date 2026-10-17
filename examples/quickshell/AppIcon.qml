import QtQuick
import Quickshell

Item {
    id: root

    property string icon: ""

    Image {
        id: themed

        anchors.fill: parent
        source: root.icon ? Quickshell.iconPath(root.icon, true) : ""
        fillMode: Image.PreserveAspectFit
    }

    Image {
        anchors.fill: parent
        visible: themed.status !== Image.Ready
        source: "icons/application.svg"
        fillMode: Image.PreserveAspectFit
    }

}
