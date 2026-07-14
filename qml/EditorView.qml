import QtQuick
import QtQuick.Controls
import booklet

// The block editor. Each block renders its markdown natively
// (TextEdit.MarkdownText). Clicking a block — including the title, which is
// simply block 0 — swaps it to a raw TextArea with exactly that block's
// source. Leaving the block (focus loss or Escape) commits it to Rust,
// which re-parses and saves.
Rectangle {
    id: view
    color: Theme.bg

    property var blocks: []
    property int editing: -1

    Connections {
        target: NoteEditor
        function onBlocks_changed() {
            view.blocks = JSON.parse(NoteEditor.blocks())
            view.editing = -1
        }
    }

    Rectangle { // the page
        anchors.fill: parent
        anchors.margins: 18
        color: Theme.page
        border.color: Theme.pageLine
        border.width: 1
        radius: 4

        // Stitched inner margin — the one bookish flourish on the page.
        Rectangle {
            x: 22; width: 1
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            anchors.topMargin: 10
            anchors.bottomMargin: 10
            color: "transparent"
            Column {
                spacing: 6
                Repeater {
                    model: Math.floor(parent.parent.height / 12)
                    Rectangle { width: 1; height: 6; color: Theme.pageLine }
                }
            }
        }

        ListView {
            id: blockList
            anchors.fill: parent
            anchors.margins: 34
            anchors.leftMargin: 44
            model: view.blocks
            spacing: 8
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            delegate: Item {
                id: blockItem
                required property var modelData
                required property int index
                width: blockList.width
                implicitHeight: view.editing === index
                    ? srcEdit.implicitHeight
                    : rendered.implicitHeight
                height: implicitHeight

                // --- rendered mode ---
                TextEdit {
                    id: rendered
                    visible: view.editing !== blockItem.index
                    width: parent.width
                    readOnly: true
                    textFormat: TextEdit.MarkdownText
                    text: blockItem.modelData.display
                    color: Theme.text
                    selectionColor: Theme.brassDeep
                    font.family: blockItem.modelData.kind === "code" ? Theme.mono : Theme.body
                    font.pixelSize: blockItem.modelData.kind === "heading" ? 24 : 16
                    wrapMode: Text.Wrap

                    onLinkActivated: (link) => {
                        if (link.startsWith("folio://"))
                            NoteEditor.open_by_title(decodeURIComponent(link.slice(8)))
                        else
                            Qt.openUrlExternally(link)
                    }

                    MouseArea { // the Obsidian move: click -> source
                        anchors.fill: parent
                        cursorShape: rendered.hoveredLink !== ""
                            ? Qt.PointingHandCursor : Qt.IBeamCursor
                        onClicked: (mouse) => {
                            // Let links win; edit on plain clicks.
                            if (rendered.hoveredLink !== "") {
                                rendered.linkActivated(rendered.hoveredLink)
                                return
                            }
                            view.editing = blockItem.index
                            srcEdit.forceActiveFocus()
                        }
                    }
                }

                // --- source mode ---
                TextArea {
                    id: srcEdit
                    visible: view.editing === blockItem.index
                    width: parent.width
                    text: blockItem.modelData.source
                    color: Theme.textBright
                    font.family: Theme.mono
                    font.pixelSize: 13.5
                    wrapMode: Text.Wrap
                    background: Rectangle { color: Theme.editBg; radius: 4 }

                    onActiveFocusChanged: {
                        if (!activeFocus && view.editing === blockItem.index)
                            NoteEditor.commit_block(blockItem.index, text)
                    }
                    Keys.onEscapePressed:
                        NoteEditor.commit_block(blockItem.index, text)
                }
            }
        }

        Text {
            visible: view.blocks.length === 0
            anchors.centerIn: parent
            text: "Choose a note from the index"
            color: Theme.textDim
            font.family: Theme.display
            font.pixelSize: 18
        }
    }
}
