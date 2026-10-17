import QtQuick
import QtQuick.Controls
import Quickshell
import "Protocol.js" as Protocol

Item {
    id: dock

    property ShellStyle appearance
    required property var shellBridge
    required property var currentOutput
    required property var panelWindow
    required property var pinStore
    property string panelNamespace: ""
    property var viewport: null
    readonly property string outputIdentity: currentOutput ? currentOutput.id : ""
    property string hoveredApp: ""
    property string pendingApp: ""
    property bool previewLoaded: false
    property int previewStartIndex: 0
    readonly property int previewWidth: Math.min(350, Math.max(1, panelWindow.width - 16))
    readonly property int previewHeight: Math.min(560, Math.max(1, (panelWindow.screen ? panelWindow.screen.height : 480) - appearance.panelTopMargin - panelWindow.height - appearance.popupGap - 12), 66 + (hoveredGroup ? hoveredGroup.windows.length : 0) * 210)
    readonly property var groups: Protocol.appGroups(shellBridge ? shellBridge.state : null, currentOutput, pinStore ? pinStore.ids : [])
    readonly property var hoveredGroup: (groups || []).find(function(group) {
        return group.id === hoveredApp;
    }) || null

    function syncApps() {
        if (!dockApps) return;
        for (let i = 0; i < groups.length; ++i) {
            let existing = i;
            while (existing < dockApps.count && dockApps.get(existing).appId !== groups[i].id)
                ++existing;
            if (existing === dockApps.count)
                dockApps.insert(i, {appId: groups[i].id});
            else if (existing !== i)
                dockApps.move(existing, i, 1);
        }
        if (dockApps.count > groups.length)
            dockApps.remove(groups.length, dockApps.count - groups.length);
    }

    ListModel { id: dockApps }

    function scheduleTargets() {
        Qt.callLater(reportTargets);
    }

    function reportTargets() {
        if (!shellBridge || !shellBridge.online || !viewport || !Protocol.supportsAnimationTargets(shellBridge.state))
            return ;

        var panel = Protocol.animationPanelFor(shellBridge.animationPanels, currentOutput, panelNamespace, panelWindow.width, panelWindow.height);
        if (!panel)
            return ;

        var icons = [];
        for (var i = 0; i < appRepeater.count; i++) {
            var button = appRepeater.itemAt(i);
            if (!visible || !viewport.visible || !button || !button.visible)
                continue;

            var point = panelWindow.mapFromItem(button.iconItem, 0, 0);
            icons.push({
                "app_id": button.modelData.id,
                "x": point.x,
                "y": point.y,
                "width": button.iconItem.width,
                "height": button.iconItem.height
            });
        }
        var origin = panelWindow.mapFromItem(viewport, 0, 0);
        var targets = Protocol.animationAppTargets(shellBridge.state, currentOutput, icons, {
            "x": origin.x,
            "y": origin.y,
            "width": viewport.width,
            "height": viewport.height
        }, panelWindow.width, panelWindow.height);
        shellBridge.command({
            "type": "set_animation_targets",
            "panel": panel.panel,
            "output": currentOutput.id,
            "targets": targets
        });
    }

    function showPreview(id, button) {
        hoveredApp = id;
        previewStartIndex = 0;
        previewLoaded = true;
        closeDelay.stop();
        Qt.callLater(function() {
            if (previewLoader.item)
                previewLoader.item.anchor.updateAnchor();

        });
    }

    function showContextMenu(id, button) {
        dismissPreview();
        contextLoader.active = true;
        const point = panelWindow.mapFromItem(button, 0, 0);
        contextLoader.item.open(id, appearance.panelSideMargin + point.x + button.width / 2,
            appearance.panelTopMargin + panelWindow.height + appearance.popupGap);
    }

    function queuePreview(id, button) {
        if (contextLoader.item && contextLoader.item.opened) return;
        closeDelay.stop();
        if (hoveredApp !== id)
            hoveredApp = "";

        pendingApp = id;
        openDelay.restart();
    }

    function buttonForApp(id) {
        for (let i = 0; i < appRepeater.count; ++i) {
            const button = appRepeater.itemAt(i);
            if (button && button.modelData.id === id) return button;
        }
        return null;
    }

    function leavePreview() {
        openDelay.stop();
        pendingApp = "";
        closeDelay.restart();
    }

    function dismissPreview() {
        openDelay.stop();
        closeDelay.stop();
        hoveredApp = "";
        pendingApp = "";
    }

    onGroupsChanged: { syncApps(); scheduleTargets(); }
    onXChanged: scheduleTargets()
    onYChanged: scheduleTargets()
    onVisibleChanged: scheduleTargets()
    onCurrentOutputChanged: scheduleTargets()
    onOutputIdentityChanged: dismissPreview()
    Component.onCompleted: { syncApps(); scheduleTargets(); }
    implicitWidth: appRow.implicitWidth
    implicitHeight: 34

    Timer {
        interval: 1000
        repeat: true
        running: dock.shellBridge && dock.shellBridge.online && Protocol.supportsAnimationTargets(dock.shellBridge.state)
        onTriggered: dock.reportTargets()
    }

    Connections {
        function onContentXChanged() {
            dock.scheduleTargets();
        }

        function onContentYChanged() {
            dock.scheduleTargets();
        }

        function onWidthChanged() {
            dock.scheduleTargets();
        }

        function onHeightChanged() {
            dock.scheduleTargets();
        }

        function onXChanged() {
            dock.scheduleTargets();
        }

        function onYChanged() {
            dock.scheduleTargets();
        }

        target: dock.viewport
    }

    Connections {
        function onWidthChanged() {
            dock.scheduleTargets();
            dock.dismissPreview();
        }

        function onHeightChanged() {
            dock.scheduleTargets();
            dock.dismissPreview();
        }

        target: dock.panelWindow
    }

    Timer {
        id: openDelay

        interval: dock.appearance.hoverDelay
        onTriggered: {
            const button = dock.buttonForApp(dock.pendingApp);
            if (button) dock.showPreview(dock.pendingApp, button);

        }
    }

    Timer {
        id: closeDelay

        interval: 300
        onTriggered: dock.dismissPreview()
    }

    Row {
        id: appRow

        spacing: 4

        Repeater {
            id: appRepeater

            onItemAdded: dock.scheduleTargets()
            onItemRemoved: dock.scheduleTargets()
            model: dockApps

            delegate: Rectangle {
                id: appButton

                required property string appId
                readonly property var modelData: (dock.groups || []).find(function(group) { return group.id === appId; }) || {id: appId, windows: [], pinned: false}
                readonly property var iconItem: appIcon
                readonly property var entry: (DesktopEntries.applications.values.length > 0 ? DesktopEntries.heuristicLookup(modelData.id) : null)
                readonly property bool focused: modelData.windows.some(function(window) {
                    return window.focused;
                })

                width: 40
                height: 34
                radius: dock.appearance.controlRadius
                color: mouseArea.pressed ? dock.appearance.pressedColor : mouseArea.containsMouse ? dock.appearance.hoverColor : focused ? dock.appearance.cardSelectedColor : "transparent"
                Accessible.name: entry ? entry.name : modelData.id
                Accessible.role: Accessible.Button

                AppIcon {
                    id: appIcon

                    anchors.centerIn: parent
                    anchors.verticalCenterOffset: -1
                    width: 22
                    height: 22
                    icon: entry ? entry.icon : ""
                }

                Rectangle {
                    visible: modelData.windows.length > 0
                    anchors.bottom: parent.bottom
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(18, 4 + modelData.windows.length * 4)
                    height: 2
                    radius: 1
                    color: appButton.focused ? dock.appearance.accentColor : dock.appearance.mutedColor
                }

                MouseArea {
                    id: mouseArea

                    anchors.fill: parent
                    hoverEnabled: true
                    acceptedButtons: Qt.LeftButton | Qt.RightButton
                    onEntered: dock.queuePreview(appButton.modelData.id, appButton)
                    onExited: dock.leavePreview()
                    onClicked: (mouse) => {
                        dock.dismissPreview();
                        if (mouse.button === Qt.RightButton) {
                            dock.showContextMenu(appButton.modelData.id, appButton);
                            return ;
                        }
                        var windows = appButton.modelData.windows;
                        if (windows.length)
                            dock.shellBridge.command({
                            "type": "focus_window",
                            "window": windows[0].id
                        });
                        else if (appButton.entry)
                            appButton.entry.execute();
                    }
                }

                ShellToolTip {
                    target: appButton
                    appearance: dock.appearance
                    hovered: mouseArea.containsMouse && appButton.modelData.windows.length === 0
                    belowPanel: true
                    text: (appButton.entry ? appButton.entry.name : appButton.modelData.id) + (appButton.modelData.pinned ? " · pinned" : "") + " · right click for options"
                }

                Behavior on color {
                    ColorAnimation {
                        duration: dock.appearance.motionDuration
                    }

                }

            }

        }

    }

    LazyLoader {
        id: contextLoader
        // Incubate the closed menu at startup instead of compiling on right-click.
        activeAsync: true
        AppContextMenu {
            appearance: dock.appearance
            shellBridge: dock.shellBridge
            pinStore: dock.pinStore
            targetScreen: dock.panelWindow.screen
        }
    }
    Timer {
        interval: 220
        repeat: true
        running: dock.hoveredGroup !== null && dock.hoveredGroup.windows.length > 0
            && dock.hoveredApp !== "" && dock.shellBridge.online
        triggeredOnStart: true
        onTriggered: dock.shellBridge.requestPreviews(dock.hoveredGroup.windows
            .slice(dock.previewStartIndex, dock.previewStartIndex + 4).map(function(w) { return w.id; }))
    }

    LazyLoader {
        id: previewLoader

        active: dock.previewLoaded

        PopupWindow {
            id: preview

            visible: !!dock.hoveredGroup && dock.hoveredGroup.windows.length > 0
            color: "transparent"
            implicitWidth: dock.previewWidth
            implicitHeight: dock.previewHeight
            anchor.window: dock.panelWindow
            anchor.edges: Edges.Top | Edges.Left
            anchor.gravity: Edges.Bottom | Edges.Right
            // Slide horizontally at output edges; never flip up over the bar.
            anchor.adjustment: PopupAdjustment.SlideX
            anchor.rect.y: dock.panelWindow.height + dock.appearance.popupGap
            anchor.onAnchoring: {
                const button = dock.buttonForApp(dock.hoveredApp) || dock;
                const point = dock.panelWindow.mapFromItem(button, 0, 0);
                anchor.rect.x = Math.max(8, Math.min(dock.panelWindow.width - preview.width - 8, point.x + button.width / 2 - preview.width / 2));
            }
            grabFocus: false

            Rectangle {
                anchors.fill: parent
                color: dock.appearance.surfaceColor
                radius: dock.appearance.overlayRadius
                border.width: dock.appearance.outlineWidth
                border.color: dock.appearance.outlineColor
                antialiasing: true
            }

            Column {
                anchors.fill: parent
                anchors.margins: 12
                spacing: 10

                Row {
                    width: parent.width
                    height: 30
                    spacing: 10

                    AppIcon {
                        readonly property var entry: dock.hoveredGroup ? (DesktopEntries.applications.values.length > 0 ? DesktopEntries.heuristicLookup(dock.hoveredGroup.id) : null) : null

                        width: 26
                        height: 26
                        anchors.verticalCenter: parent.verticalCenter
                        icon: entry ? entry.icon : ""
                    }

                    Text {
                        readonly property var entry: dock.hoveredGroup ? (DesktopEntries.applications.values.length > 0 ? DesktopEntries.heuristicLookup(dock.hoveredGroup.id) : null) : null

                        width: parent.width - 76
                        anchors.verticalCenter: parent.verticalCenter
                        text: entry ? entry.name : dock.hoveredGroup ? dock.hoveredGroup.id : ""
                        color: dock.appearance.textColor
                        font.pixelSize: 14
                        font.weight: Font.DemiBold
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: dock.hoveredGroup ? dock.hoveredGroup.windows.length : ""
                        color: dock.appearance.mutedColor
                        font.pixelSize: 12
                    }

                }

                ListView {
                    id: previewList

                    // Preserve delegates (and their displayed frame) on metadata snapshots.
                    function syncWindows() {
                        const windows = dock.hoveredGroup ? dock.hoveredGroup.windows : [];
                        for (let i = 0; i < windows.length; ++i) {
                            let existing = i;
                            while (existing < previewWindows.count && previewWindows.get(existing).windowData.id !== windows[i].id)
                                ++existing;
                            if (existing === previewWindows.count)
                                previewWindows.insert(i, {windowData: windows[i]});
                            else {
                                if (existing !== i) previewWindows.move(existing, i, 1);
                                previewWindows.setProperty(i, "windowData", windows[i]);
                            }
                        }
                        if (previewWindows.count > windows.length)
                            previewWindows.remove(windows.length, previewWindows.count - windows.length);
                    }

                    Component.onCompleted: syncWindows()
                    Connections {
                        target: dock
                        function onHoveredGroupChanged() { previewList.syncWindows(); }
                        function onHoveredAppChanged() { previewList.positionViewAtBeginning(); }
                    }
                    ListModel { id: previewWindows; dynamicRoles: true }

                    width: parent.width
                    height: Math.max(0, parent.height - 40)
                    clip: true
                    spacing: 6
                    model: previewWindows
                    onContentYChanged: dock.previewStartIndex = Math.max(0, Math.floor(contentY / 210))

                    ScrollBar.vertical: ScrollBar {
                        policy: ScrollBar.AsNeeded
                    }

                    delegate: Rectangle {
                        id: windowCard

                        required property var windowData
                        readonly property var modelData: windowData

                        width: ListView.view.width
                        height: 204
                        radius: 12
                        color: modelData.focused ? dock.appearance.cardSelectedColor : hover.containsMouse ? dock.appearance.hoverColor : dock.appearance.controlColor

                        LivePreview {
                            id: liveImage
                            anchors.top: parent.top
                            anchors.topMargin: 10
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: parent.width - 24
                            height: 144
                            source: dock.shellBridge.windowPreviews[windowCard.modelData.id] || ""
                        }
                        AppIcon {
                            anchors.centerIn: liveImage
                            width: 36; height: 36
                            visible: !liveImage.hasFrame
                            readonly property var entry: (DesktopEntries.applications.values.length > 0 ? DesktopEntries.heuristicLookup(windowCard.modelData.app_id) : null)
                            icon: entry ? entry.icon : ""
                        }
                        Column {
                            anchors.fill: parent
                            anchors.margins: 12
                            anchors.topMargin: 160
                            anchors.rightMargin: 88
                            spacing: 5

                            Text {
                                width: parent.width
                                text: modelData.title || modelData.app_id || "Untitled"
                                textFormat: Text.PlainText
                                font.pixelSize: 13
                                font.weight: Font.Medium
                                color: dock.appearance.textColor
                                elide: Text.ElideRight
                            }

                            Text {
                                text: "Workspace " + modelData.workspace + (modelData.minimized ? " · minimized" : modelData.maximized ? " · maximized" : "")
                                color: dock.appearance.mutedColor
                                font.pixelSize: 11
                                width: parent.width
                                elide: Text.ElideRight
                            }

                        }

                        MouseArea {
                            id: hover

                            anchors.fill: parent
                            hoverEnabled: true
                            onClicked: {
                                dock.shellBridge.command({
                                    "type": "focus_window",
                                    "window": modelData.id
                                });
                                dock.hoveredApp = "";
                            }
                        }

                        Row {
                            anchors.right: parent.right
                            anchors.rightMargin: 6
                            anchors.bottom: parent.bottom
                            anchors.bottomMargin: 8
                            spacing: 3

                            PanelButton {
                                appearance: dock.appearance
                                iconSource: windowCard.modelData.minimized ? "icons/restore.svg" : "icons/minimize.svg"
                                quiet: true
                                description: windowCard.modelData.minimized ? "Restore window" : "Minimize window"
                                onClicked: {
                                    if (windowCard.modelData.minimized)
                                        dock.shellBridge.command({
                                        "type": "focus_window",
                                        "window": windowCard.modelData.id
                                    });
                                    else
                                        dock.shellBridge.command({
                                        "type": "set_minimized",
                                        "window": windowCard.modelData.id,
                                        "minimized": true
                                    });
                                    dock.hoveredApp = "";
                                }
                            }

                            PanelButton {
                                appearance: dock.appearance
                                iconSource: windowCard.modelData.maximized ? "icons/restore.svg" : "icons/maximize.svg"
                                quiet: true
                                description: windowCard.modelData.maximized ? "Unmaximize window" : "Maximize window"
                                onClicked: {
                                    dock.shellBridge.command({
                                        "type": "set_maximized",
                                        "window": windowCard.modelData.id,
                                        "maximized": !windowCard.modelData.maximized
                                    });
                                    dock.shellBridge.command({
                                        "type": "focus_window",
                                        "window": windowCard.modelData.id
                                    });
                                    dock.hoveredApp = "";
                                }
                            }

                        }

                    }

                }

            }

            HoverHandler {
                onHoveredChanged: {
                    if (hovered)
                        closeDelay.stop();
                    else
                        closeDelay.restart();
                }
            }

            mask: Region {
                width: preview.width
                height: preview.height
                radius: dock.appearance.overlayRadius
            }

        }

    }

    appearance: ShellStyle {
    }

}
