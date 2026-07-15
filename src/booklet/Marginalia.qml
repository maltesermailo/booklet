import QtQuick
import QtQuick.Controls
import booklet

// Backlinks panel — notes written in other books' margins that reference
// this page.
Rectangle {
    id: panel
    color: Theme.panel

    property var backlinks: []
    property var outgoing: []
    property var tags: []
    property string noteId: ""
    property string noteTitle: ""

    // What the star map draws: who points here, and where this note points.
    //
    // Outgoing first, and a title is only placed once: link to a note that links
    // back and it is still one neighbour. Drawing it twice would put two dots
    // with the same name at different angles and claim a graph that isn't there.
    // The mutual case keeps the outgoing colour — that dot answers to a [[link]]
    // visible in the text you are reading.
    readonly property var stars: {
        var all = []
        var placed = []

        for (var j = 0; j < panel.outgoing.length; j++) {
            var link = panel.outgoing[j]
            all.push({ "title": link.title, "id": link.id,
                       "kind": link.id === "" ? "unresolved" : "out" })
            placed.push(link.title)
        }

        for (var i = 0; i < panel.backlinks.length; i++) {
            var back = panel.backlinks[i]
            if (placed.indexOf(back.source_title) === -1)
                all.push({ "title": back.source_title, "id": back.source_id, "kind": "in" })
        }

        return all
    }

    function reload() {
        if (panel.noteId === "") {
            panel.backlinks = []
            panel.outgoing = []
            panel.tags = []
            return
        }

        panel.backlinks = JSON.parse(Backlinks.for_note(panel.noteId, panel.noteTitle))
        panel.outgoing = JSON.parse(NoteEditor.outgoing_links())
        panel.tags = JSON.parse(NoteEditor.tags())
    }

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
            panel.reload()
        }
        // false means "just written", which is when links and tags may have
        // moved.
        function onSave_state_changed(unsaved) {
            if (!unsaved)
                panel.reload()
        }
    }

    Column {
        id: head
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: 14
        spacing: 9

        Row {
            spacing: 8
            Text {
                id: heading
                text: "Marginalia"
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: Theme.px(16)
            }
            Text {
                text: panel.backlinks.length
                color: Theme.textSoft
                font.family: Theme.mono
                font.pixelSize: Theme.px(11)
                anchors.baseline: heading.baseline
            }
        }

        StarMap {
            width: parent.width
            title: panel.noteTitle
            stars: panel.stars
        }
    }

    // Pinned to the bottom: tags describe the whole note, so they read as a
    // footer rather than something the backlink list pushes around.
    Column {
        id: foot
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: 14
        spacing: 6
        visible: panel.tags.length > 0

        Text {
            text: "TAGS"
            color: Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(11)
            font.letterSpacing: 2 * Theme.uiScale
        }

        Flow {
            width: parent.width
            spacing: 5

            Repeater {
                model: panel.tags

                delegate: Rectangle {
                    required property string modelData

                    width: pill.implicitWidth + 16
                    height: pill.implicitHeight + 2
                    radius: height / 2
                    color: Theme.page
                    border.color: Theme.pageLine
                    border.width: 1

                    Text {
                        id: pill
                        anchors.centerIn: parent
                        text: "#" + modelData
                        color: Theme.textSoft
                        font.family: Theme.mono
                        font.pixelSize: Theme.px(11)
                    }
                }
            }
        }
    }

    ListView {
        anchors.top: head.bottom
        anchors.bottom: foot.visible ? foot.top : parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.margins: 14
        model: panel.backlinks
        spacing: 8
        clip: true

        delegate: Rectangle {
            required property var modelData
            width: ListView.view ? ListView.view.width : 0
            height: card.implicitHeight + 18
            color: Theme.page
            border.color: Theme.pageLine
            radius: Theme.radiusCard

            Column {
                id: card
                anchors.fill: parent
                anchors.margins: 9
                spacing: 4
                Text {
                    text: modelData.source_title.toUpperCase()
                    color: Theme.textSoft
                    font.family: Theme.ui
                    font.pixelSize: Theme.px(10)
                    font.letterSpacing: 1 * Theme.uiScale
                    elide: Text.ElideRight
                    width: parent.width
                }
                Text {
                    text: panel.markLink(modelData.snippet, panel.noteTitle)
                    textFormat: Text.RichText
                    color: Theme.text
                    font.family: Theme.body
                    font.pixelSize: Theme.px(12)
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
