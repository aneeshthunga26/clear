import QtQuick
import "Protocol.js" as Protocol

Image {
    property string mode: "master_stack"
    source: Protocol.modeIcon(mode)
    sourceSize.width: width
    sourceSize.height: height
    fillMode: Image.PreserveAspectFit
    smooth: true
}
