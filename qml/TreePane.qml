import QtQuick
import QtQuick.Controls
import booklet

// Obsidian-style file explorer over the flattened tree from Rust.
// Depth arrives per row; indentation is depth * 13. Toggling a folder is a
// slot call; Rust recomputes the visible rows and signals tree_changed.
Rectangle {
    id: pane
    color: Theme.sidebar

    property var rows: []
    property string currentId: ""

    function refresh() {
        rows = JSON.parse(Library.visible_rows())
    }
    Component.onCompleted: refresh()

    Connections {
        target: Library
        function onTree_changed() { pane.refresh() }
    }
    Connections {
        target: NoteEditor
        function onNote_opened(id, title) { pane.currentId = id }
    }

    ListView {
        anchors.fill: parent
        anchors.margins: 6
        model: pane.rows
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        delegate: Rectangle {
            required property var modelData
            width: ListView.view ? ListView.view.width : 0
            height: 24
            radius: 4
            color: modelData.id === pane.currentId ? Theme.activePill
                 : hover.hovered ? Qt.rgba(1, 1, 1, 0.03) : "transparent"

            HoverHandler { id: hover }

            Row {
                anchors.verticalCenter: parent.verticalCenter
                x: 8 + modelData.depth * 13
                spacing: 6

                Text { // chevron
                    visible: modelData.has_children
                    text: modelData.expanded ? "\u25BE" : "\u25B8"
                    color: Theme.textDim
                    font.pixelSize: 10
                    anchors.verticalCenter: parent.verticalCenter
                }
                Rectangle { // binding chip, books only
                    visible: modelData.kind === "book"
                    width: 3; height: 12; radius: 1
                    color: modelData.color !== "" ? modelData.color : Theme.textDim
                    anchors.verticalCenter: parent.verticalCenter
                }
                Text {
                    text: modelData.title
                    color: modelData.kind === "note"
                        ? (modelData.id === pane.currentId ? Theme.textBright : Theme.textSoft)
                        : Theme.text
                    font.family: Theme.ui
                    font.pixelSize: 13
                    font.weight: modelData.kind === "book" ? Font.Medium : Font.Normal
                    elide: Text.ElideRight
                }
            }

            MouseArea {
                anchors.fill: parent
                onClicked: modelData.kind === "note"
                    ? NoteEditor.open(modelData.id)
                    : Library.toggle(modelData.id)
            }
        }
    }
}
