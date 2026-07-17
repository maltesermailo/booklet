import QtQuick
import QtQuick.Controls
import booklet

// Version history for a note: the list on the left, the selected version's text
// on the right, with restore. Same modal shell as Settings.
Popup {
    id: modal
    modal: true
    focus: true
    padding: 1
    width: Math.min(760, Overlay.overlay ? Overlay.overlay.width - 60 : 760)
    height: Math.min(520, Overlay.overlay ? Overlay.overlay.height - 60 : 520)
    anchors.centerIn: Overlay.overlay
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property string notePath: ""
    property var versions: []
    property int selected: -1

    function openFor(path) {
        modal.notePath = path
        modal.versions = []
        modal.selected = -1
        preview.text = ""
        Sync.request_history(path)
        modal.open()
    }

    function select(index) {
        modal.selected = index
        preview.text = ""
        Sync.request_version(JSON.stringify({ path: modal.notePath, version: modal.versions[index].version }))
    }

    function whenText(epochMs) {
        return new Date(epochMs).toLocaleString(Qt.locale(), "d MMM yyyy, h:mm")
    }

    // Insertions (added since this version) in green, deletions (that this
    // version still had) struck through in ember.
    readonly property string insertColor: "#6FA86F"

    function escapeHtml(text) {
        return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    }

    // A colored diff of the selected version against the note's current text.
    function buildDiff(versionContent) {
        var segments = JSON.parse(NoteEditor.diff_segments(versionContent))
        var html = "<span style='white-space:pre-wrap'>"
        for (var i = 0; i < segments.length; i++) {
            var text = modal.escapeHtml(segments[i].text).replace(/\n/g, "<br>")
            if (segments[i].op === "insert")
                html += "<span style='color:" + modal.insertColor + "'>" + text + "</span>"
            else if (segments[i].op === "delete")
                html += "<span style='color:" + Theme.ember + "; text-decoration:line-through'>" + text + "</span>"
            else
                html += "<span style='color:" + Theme.textSoft + "'>" + text + "</span>"
        }
        return html + "</span>"
    }

    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
            NumberAnimation { property: "scale"; from: 0.97; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
        }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Theme.quick; easing.type: Theme.easing }
    }

    background: Rectangle {
        color: Theme.bg
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    Connections {
        target: Sync
        function onHistory_ready(payload) {
            modal.versions = JSON.parse(payload)
            if (modal.versions.length > 0)
                modal.select(modal.versions.length - 1)
        }
        function onVersion_ready(payload) {
            preview.text = modal.buildDiff(JSON.parse(payload).content)
        }
    }

    // × close, mirroring Settings.
    Rectangle {
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 10
        z: 1
        width: 22
        height: 22
        radius: 4
        color: closeHover.hovered ? Theme.activePill : "transparent"
        HoverHandler { id: closeHover }
        Text {
            anchors.centerIn: parent
            text: "×"
            color: closeHover.hovered ? Theme.textBright : Theme.textDim
            font.pixelSize: Theme.px(16)
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: modal.close()
        }
    }

    Row {
        anchors.fill: parent
        spacing: 0

        // The version list.
        Rectangle {
            width: 220
            height: parent.height
            color: Theme.sidebar
            topLeftRadius: Theme.radiusCard - 1
            bottomLeftRadius: Theme.radiusCard - 1

            Rectangle {
                anchors.right: parent.right
                width: 1
                height: parent.height
                color: Theme.sidebarLine
            }

            Column {
                anchors.fill: parent
                anchors.margins: 16
                spacing: 8

                Text {
                    text: "Version history"
                    color: Theme.textBright
                    font.family: Theme.display
                    font.pixelSize: Theme.px(17)
                }

                ListView {
                    width: parent.width
                    height: parent.height - 40
                    clip: true
                    model: modal.versions
                    spacing: 2

                    delegate: Rectangle {
                        required property int index
                        required property var modelData
                        width: ListView.view.width
                        height: Theme.row(40)
                        radius: Theme.radiusSmall
                        color: index === modal.selected ? Theme.activePill
                               : (rowHover.hovered ? Theme.activePill : "transparent")
                        Behavior on color { ColorAnimation { duration: Theme.quick } }

                        HoverHandler { id: rowHover }

                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            x: 10
                            spacing: 2

                            Text {
                                text: "Version " + modelData.version + (modelData.deleted ? " (deleted)" : "")
                                color: Theme.textBright
                                font.family: Theme.ui
                                font.pixelSize: Theme.px(12)
                            }
                            Text {
                                text: modal.whenText(modelData.created_at)
                                      + (modelData.device !== "" ? " · " + modelData.device : "")
                                color: Theme.textDim
                                font.family: Theme.ui
                                font.pixelSize: Theme.px(10)
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: modal.select(index)
                        }
                    }
                }
            }
        }

        // The selected version's text, and restore.
        Item {
            width: parent.width - 220
            height: parent.height

            ScrollView {
                anchors.fill: parent
                anchors.margins: 16
                anchors.bottomMargin: 52
                clip: true

                TextArea {
                    id: preview
                    readOnly: true
                    textFormat: TextEdit.RichText
                    wrapMode: TextArea.Wrap
                    color: Theme.text
                    font.family: Theme.mono
                    font.pixelSize: Theme.px(12)
                    background: Rectangle { color: Theme.codeBg; radius: Theme.radiusSmall }
                }
            }

            TextButton {
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: 16
                label: "Restore this version"
                filled: true
                enabled: modal.selected >= 0
                onClicked: {
                    Sync.restore(JSON.stringify({
                        path: modal.notePath,
                        version: modal.versions[modal.selected].version
                    }))
                    modal.close()
                }
            }
        }
    }
}
