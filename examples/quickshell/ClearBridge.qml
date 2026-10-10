import QtQuick
import Quickshell
import Quickshell.Io
import "Protocol.js" as Protocol

Scope {
    id: bridge

    readonly property string socketPath: Quickshell.env("CLEAR_SOCKET") || ""
    readonly property bool online: socket.connected && state !== null
    readonly property string status: socketPath === "" ? "Offline: CLEAR_SOCKET is unset"
        : lastError !== "" ? lastError
        : online ? "Connected" : socket.connected ? "Waiting for Clear state" : "Offline: retrying Clear IPC"
    property var state: null
    property var animationPanels: []
    property string lastError: ""
    // QML int is signed; double represents every u32 request ID exactly.
    property double nextId: 1
    property bool ready: false

    function send(request) {
        if (!socket.connected)
            return;
        lastError = "";
        socket.write(Protocol.encodeRequest(nextId, request));
        socket.flush();
        nextId = nextId === 4294967295 ? 1 : nextId + 1;
    }

    function command(request) {
        // Never queue/replay clicks across disconnection or before a snapshot.
        if (online)
            send(request);
    }

    function receive(line) {
        try {
            var message = Protocol.decodeMessage(line);
            if (message.type === "snapshot" || message.type === "state") {
                if (state === null)
                    console.info("Clear shell: subscribed to protocol v1");
                var first = state === null;
                state = message.state;
                if (first && Protocol.supportsAnimationTargets(state))
                    send({type: "animation_panels"});
            } else if (message.type === "animation_panels")
                animationPanels = message.panels;
            else if (message.type === "error")
                lastError = "Clear: " + message.message;
        } catch (error) {
            state = null;
            animationPanels = [];
            lastError = "Offline: " + error.message;
            socket.connected = false;
        }
    }

    Component.onCompleted: ready = true

    Socket {
        id: socket
        path: bridge.socketPath
        connected: false
        parser: SplitParser {
            splitMarker: "\n"
            onRead: data => bridge.receive(data)
        }
        onConnectedChanged: {
            bridge.state = null;
            bridge.animationPanels = [];
            if (connected)
                bridge.send({type: "subscribe"});
        }
        onError: {
            bridge.state = null;
            bridge.animationPanels = [];
            bridge.lastError = "Offline: Clear IPC unavailable; retrying";
            connected = false;
        }
    }

    Timer {
        interval: 1000
        repeat: true
        running: bridge.online && Protocol.supportsAnimationTargets(bridge.state)
        onTriggered: bridge.send({type: "animation_panels"})
    }

    Timer {
        interval: 2000
        repeat: true
        // Do not attempt an empty path or connect during object construction.
        running: bridge.ready && bridge.socketPath !== "" && !socket.connected
        onTriggered: socket.connected = true
    }
}
