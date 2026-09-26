import QtQuick
import Quickshell
import Quickshell.Io

Scope {
    id: root

    readonly property var ids: data.ids

    function toggle(id) {
        if (!id || id === "unknown") return;
        var next = data.ids.slice();
        var index = next.indexOf(id);
        if (index === -1) next.push(id);
        else next.splice(index, 1);
        data.ids = next;
    }

    FileView {
        path: Quickshell.statePath("pins.json")
        printErrors: false
        onAdapterUpdated: writeAdapter()
        JsonAdapter {
            id: data
            property var ids: []
        }
    }
}
