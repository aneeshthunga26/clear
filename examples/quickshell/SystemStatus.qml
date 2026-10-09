import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Bluetooth
import Quickshell.Services.Pipewire
import Quickshell.Services.UPower
import "Protocol.js" as Protocol

Row {
    id: status
    required property var shellBridge
    required property var currentOutput
    property bool compact: false
    property var minimizedByScope: ({})
    readonly property string scopeKey: currentOutput
        ? currentOutput.id + "/" + currentOutput.workspace : ""
    readonly property var minimizedIds: minimizedByScope[scopeKey] || []
    readonly property var sink: Pipewire.defaultAudioSink
    readonly property var adapter: Bluetooth.defaultAdapter
    readonly property var battery: UPower.displayDevice
    spacing: 2

    PwObjectTracker { objects: [status.sink] }
    SystemClock { id: clock; precision: SystemClock.Minutes }

    StatusIcon {
        visible: !!(status.sink && status.sink.audio)
        icon: status.sink && status.sink.audio && status.sink.audio.muted
            ? "icons/volume-muted.svg" : "icons/volume.svg"
        description: status.sink && status.sink.audio
            ? (status.sink.audio.muted ? "Unmute" : "Mute") + " audio ("
                + Math.round(status.sink.audio.volume * 100) + "%)"
            : "Audio unavailable"
        onClicked: status.sink.audio.muted = !status.sink.audio.muted
        onScrolled: delta => {
            if (!status.sink || !status.sink.audio) return;
            status.sink.audio.volume = Math.max(0, Math.min(1,
                status.sink.audio.volume + (delta > 0 ? 0.05 : -0.05)));
        }
    }

    StatusIcon {
        visible: !status.compact && !!status.adapter
        icon: "icons/bluetooth.svg"
        dimmed: !!status.adapter && !status.adapter.enabled
        description: status.adapter && status.adapter.enabled ? "Disable Bluetooth" : "Enable Bluetooth"
        onClicked: status.adapter.enabled = !status.adapter.enabled
    }

    StatusIcon {
        visible: !status.compact && !!(status.battery && status.battery.ready
            && status.battery.isLaptopBattery && status.battery.isPresent)
        icon: "icons/battery.svg"
        description: status.battery ? "Battery " + Math.round(status.battery.percentage * 100) + "%" : ""
    }

    Item {
        width: 76
        height: 32
        anchors.verticalCenter: parent.verticalCenter
        Column {
            anchors.centerIn: parent
            spacing: -3
            Text {
                text: Qt.formatDateTime(clock.date, "h:mm AP")
                color: "#7dd3fc"
                font.pixelSize: 14
                horizontalAlignment: Text.AlignHCenter
                width: 76
            }
            Text {
                text: Qt.formatDateTime(clock.date, "MM/dd/yyyy")
                color: "#7dd3fc"
                font.pixelSize: 10
                horizontalAlignment: Text.AlignHCenter
                width: 76
            }
        }
    }

    StatusIcon {
        visible: !status.compact
        icon: "icons/show-desktop.svg"
        description: status.minimizedIds.length ? "Restore windows" : "Show desktop"
        onClicked: {
            if (!status.currentOutput || !status.shellBridge.online) return;
            const updated = Object.assign({}, status.minimizedByScope);
            if (status.minimizedIds.length) {
                for (const id of status.minimizedIds)
                    status.shellBridge.command({type: "set_minimized", window: id, minimized: false});
                delete updated[status.scopeKey];
            } else {
                const windows = Protocol.visibleWindows(status.shellBridge.state, status.currentOutput);
                updated[status.scopeKey] = windows.map(window => window.id);
                for (const window of windows)
                    status.shellBridge.command({type: "set_minimized", window: window.id, minimized: true});
            }
            status.minimizedByScope = updated;
        }
    }
}
