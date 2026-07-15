import QtQuick
import QtQuick.Controls
import booklet

// Backlinks panel — notes written in other books' margins that reference
// this page.
Rectangle {
    id: panel
    color: Theme.panel

    property var backlinks: []
    property string noteId: ""
    property string noteTitle: ""

    // The reference marks the link at 24% accent. Qt's rich text wants a solid
    // colour, so blend it against the card once here.
    readonly property color markBg:
        Qt.tint(Theme.page, Qt.rgba(Theme.brass.r, Theme.brass.g, Theme.brass.b, 0.24))

    // Show the snippet as it reads — the link rendered as its text, highlighted —
    // rather than leaking the [[...]] source into the panel.
    function markLink(snippet, title) {
        var escaped = snippet.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
        var quoted = title.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
        var link = new RegExp("\\[\\[" + quoted + "(?:\\|([^\\]]+))?\\]\\]", "g")

        return escaped.replace(link, function (match, alias) {
            return "<span style=\"background-color:" + panel.markBg + "\">"
                 + (alias ? alias : title) + "</span>"
        })
    }

    Connections {
        target: NoteEditor
        // The id (absolute path) tells Rust which vault to scan: backlinks
        // never cross a vault boundary.
        function onNote_opened(id, title) {
            panel.noteId = id
            panel.noteTitle = title
            panel.backlinks = JSON.parse(Backlinks.for_note(id, title))
        }
        function onBlocks_changed() {
            if (panel.noteId !== "")
                panel.backlinks = JSON.parse(Backlinks.for_note(panel.noteId, panel.noteTitle))
        }
    }

    Column {
        anchors.fill: parent
        anchors.margins: 14
        spacing: 10

        Row {
            spacing: 8
            Text {
                text: "Marginalia"
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: 16
            }
            Text {
                text: panel.backlinks.length
                color: Theme.textSoft
                font.family: Theme.mono
                font.pixelSize: 11
                anchors.baseline: parent.children[0].baseline
            }
        }

        ListView {
            width: parent.width
            height: parent.height - 40
            model: panel.backlinks
            spacing: 8
            clip: true

            delegate: Rectangle {
                required property var modelData
                width: ListView.view ? ListView.view.width : 0
                height: card.implicitHeight + 18
                color: Theme.page
                border.color: Theme.pageLine
                radius: 6

                Column {
                    id: card
                    anchors.fill: parent
                    anchors.margins: 9
                    spacing: 4
                    Text {
                        text: modelData.source_title.toUpperCase()
                        color: Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: 10
                        font.letterSpacing: 1
                        elide: Text.ElideRight
                        width: parent.width
                    }
                    Text {
                        text: panel.markLink(modelData.snippet, panel.noteTitle)
                        textFormat: Text.RichText
                        color: Theme.text
                        font.family: Theme.body
                        font.pixelSize: 12
                        wrapMode: Text.Wrap
                        width: parent.width
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    onClicked: NoteEditor.open(modelData.source_id)
                }
            }
        }
    }
}
