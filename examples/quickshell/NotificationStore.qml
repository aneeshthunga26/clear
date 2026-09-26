import QtQuick
import Quickshell
import Quickshell.Services.Notifications

Scope {
    id: store

    readonly property var items: server.trackedNotifications.values
    readonly property int count: items.length

    function clear() {
        // Copy before dismissing: dismiss mutates the ObjectModel.
        items.slice().forEach(function(item) { item.dismiss(); });
    }

    NotificationServer {
        id: server
        actionsSupported: true
        bodySupported: true
        persistenceSupported: true
        onNotification: notification => notification.tracked = true
    }
}
