import QtQuick
import Quickshell

Item {
    id: tip

    required property Item target
    property ShellStyle appearance

    appearance: ShellStyle {
    }

    property string text: ""
    property bool hovered: false
    property bool belowPanel: false
    property bool ready: false
    readonly property var targetWindow: target.QsWindow.window

    onHoveredChanged: {
        ready = false;
        if (hovered && text)
            delay.restart();
        else
            delay.stop();
    }
    TextMetrics {
        id: metrics

        text: tip.text
        font.pixelSize: 12
    }

    Timer {
        id: delay

        interval: tip.appearance.tooltipDelay
        onTriggered: tip.ready = true
    }

    LazyLoader {
        active: tip.ready

        PopupWindow {
            id: bubble

            visible: tip.ready && tip.hovered && tip.text !== "" && !!tip.targetWindow
            color: "transparent"
            grabFocus: false
            implicitWidth: Math.min(300, Math.max(1, tip.targetWindow ? tip.targetWindow.width - 16 : 300), Math.ceil(metrics.advanceWidth) + 26)
            implicitHeight: label.implicitHeight + 18
            anchor.window: tip.targetWindow
            anchor.edges: Edges.Top | Edges.Left
            anchor.gravity: Edges.Bottom | Edges.Right
            anchor.adjustment: PopupAdjustment.SlideX
            anchor.onAnchoring: {
                const p = tip.targetWindow.mapFromItem(tip.target, 0, 0);
                anchor.rect.x = Math.max(8, Math.min(tip.targetWindow.width - bubble.width - 8, p.x + tip.target.width / 2 - bubble.width / 2));
                anchor.rect.y = (tip.belowPanel ? tip.targetWindow.height : p.y + tip.target.height) + tip.appearance.popupGap;
            }

            Rectangle {
                anchors.fill: parent
                radius: 10
                color: tip.appearance.surfaceColor
                border.width: 1
                border.color: tip.appearance.outlineColor
            }

            Text {
                id: label

                anchors.centerIn: parent
                width: bubble.width - 24
                text: tip.text
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: tip.appearance.textColor
            }

            // Tooltips never intercept the pointer or cover the panel control.
            mask: Region {
                width: 0
                height: 0
            }

        }

    }

}
