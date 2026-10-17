import QtQuick

Item {
    id: preview

    property string source: ""
    readonly property bool hasFrame: front >= 0
    property bool ready: false
    property int front: -1
    property int loading: -1
    property string displayedSource: ""

    function refresh() {
        if (!ready || !source || source === displayedSource || loading >= 0)
            return;
        const index = front === 0 ? 1 : 0;
        loading = index;
        const image = index === 0 ? first : second;
        image.source = source;
        settle(index);
    }

    function settle(index) {
        if (loading !== index)
            return;
        const image = index === 0 ? first : second;
        if (image.status === Image.Ready) {
            // Swap only decoded frames; the front image stays visible during IO.
            displayedSource = image.source.toString();
            front = index;
            loading = -1;
            Qt.callLater(refresh);
        } else if (image.status === Image.Error) {
            loading = -1;
            if (source !== image.source.toString())
                Qt.callLater(refresh);
        }
    }

    onSourceChanged: refresh()
    Component.onCompleted: { ready = true; refresh(); }

    Image {
        id: first
        anchors.fill: parent
        visible: preview.front === 0
        asynchronous: true
        cache: false
        fillMode: Image.PreserveAspectFit
        onStatusChanged: preview.settle(0)
    }
    Image {
        id: second
        anchors.fill: parent
        visible: preview.front === 1
        asynchronous: true
        cache: false
        fillMode: Image.PreserveAspectFit
        onStatusChanged: preview.settle(1)
    }
}
