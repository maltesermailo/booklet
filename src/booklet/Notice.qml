import QtQuick
import QtQuick.Controls
import booklet

// Failures the user needs to see. Nothing in this app writes to a console
// nobody is reading, so every `failed` signal lands here. It does not
// auto-dismiss: a note that would not save is not something to blink past.
Rectangle {
    id: notice

    property string message: ""
    // Positive confirmations (sign-in, publish) use the calmer brass accent;
    // failures keep the ember bar that means "this one matters".
    property bool positive: false

    visible: message !== ""
    height: visible ? 30 : 0
    color: Theme.page

    function show(text) {
        notice.positive = false
        notice.message = text
    }

    function inform(text) {
        notice.positive = true
        notice.message = text
    }

    Connections {
        target: Library
        function onFailed(message) { notice.show(message) }
    }
    Connections {
        target: NoteEditor
        function onFailed(message) { notice.show(message) }
    }
    Connections {
        target: Backlinks
        function onFailed(message) { notice.show(message) }
    }
    Connections {
        target: Sync
        function onFailed(message) { notice.show(message) }
        function onNotice(message) { notice.inform(message) }
    }

    Rectangle {
        anchors.top: parent.top
        width: parent.width
        height: 1
        color: Theme.pageLine
    }

    // Ember for failures, brass for confirmations.
    Rectangle {
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        width: 3
        height: parent.height - 8
        color: notice.positive ? Theme.brass : Theme.ember
    }

    Text {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 14
        anchors.right: dismiss.left
        anchors.rightMargin: 8
        text: notice.message
        color: Theme.text
        font.family: Theme.ui
        font.pixelSize: Theme.px(12)
        elide: Text.ElideRight
    }

    Rectangle {
        id: dismiss
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 10
        width: 18
        height: 18
        radius: 3
        color: dismissHover.hovered ? Theme.activePill : "transparent"

        HoverHandler { id: dismissHover }

        Text {
            anchors.centerIn: parent
            text: "×"
            color: dismissHover.hovered ? Theme.textBright : Theme.textDim
            font.pixelSize: Theme.px(13)
        }

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: notice.message = ""
        }
    }
}
