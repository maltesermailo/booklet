import QtQuick
import QtQuick.Controls
import booklet

// The library as shelves of books. Each book is a spine whose size grows with
// its note count and whose color is its binding (from booklet.json); books are
// grouped by their shelf label. Picking a spine reveals that book in the tree.
Rectangle {
    id: shelf
    color: Theme.bg

    signal bookPicked(string id)

    property var books: []

    // Spines stay legible at the small end and stop growing at the large end,
    // so one huge book cannot dwarf the shelf.
    readonly property int countCap: 30
    readonly property int tallestSpine: spineHeight(countCap)

    function reload() {
        shelf.books = JSON.parse(Library.books())
    }

    function spineWidth(count) {
        return 28 + Math.min(count, countCap) * 1.1
    }
    function spineHeight(count) {
        return 148 + Math.min(count, countCap) * 2.4
    }

    /// Books grouped by shelf label, each group sorted by title.
    readonly property var shelves: {
        var groups = {}
        for (var i = 0; i < books.length; i++) {
            var book = books[i]
            if (groups[book.shelf] === undefined)
                groups[book.shelf] = []
            groups[book.shelf].push(book)
        }

        var out = []
        for (var name in groups) {
            groups[name].sort(function (a, b) { return a.title.localeCompare(b.title) })
            out.push({ name: name, books: groups[name] })
        }
        out.sort(function (a, b) { return a.name.localeCompare(b.name) })
        return out
    }

    onVisibleChanged: if (visible) reload()

    ScrollView {
        anchors.fill: parent
        anchors.margins: 30
        clip: true

        Column {
            id: content
            width: shelf.width - 60
            spacing: 30

            Text {
                text: "Library"
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: 32
            }

            Text {
                visible: shelf.books.length === 0
                text: "No books yet — add a vault with folders in it."
                color: Theme.textDim
                font.family: Theme.ui
                font.pixelSize: 13
            }

            Repeater {
                model: shelf.shelves

                delegate: Column {
                    id: shelfGroup
                    required property var modelData

                    width: content.width
                    spacing: 8

                    Text {
                        text: shelfGroup.modelData.name.toUpperCase()
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: 11
                        font.letterSpacing: 1.5
                    }

                    Row {
                        height: shelf.tallestSpine
                        spacing: 5

                        Repeater {
                            model: shelfGroup.modelData.books

                            // Full-height slot so every spine stands on the plank.
                            delegate: Item {
                                id: slot
                                required property var modelData

                                width: shelf.spineWidth(slot.modelData.note_count)
                                height: shelf.tallestSpine

                                Rectangle {
                                    id: spine
                                    anchors.bottom: parent.bottom
                                    anchors.bottomMargin: hover.hovered ? 7 : 0
                                    width: parent.width
                                    height: shelf.spineHeight(slot.modelData.note_count)
                                    radius: 2
                                    color: slot.modelData.color

                                    Behavior on anchors.bottomMargin {
                                        NumberAnimation { duration: 110 }
                                    }

                                    HoverHandler { id: hover }

                                    // Brass bands, the way a bound spine is tooled.
                                    Rectangle {
                                        anchors.top: parent.top
                                        anchors.topMargin: 14
                                        width: parent.width
                                        height: 1
                                        color: Theme.brass
                                        opacity: 0.55
                                    }
                                    Rectangle {
                                        anchors.bottom: parent.bottom
                                        anchors.bottomMargin: 22
                                        width: parent.width
                                        height: 1
                                        color: Theme.brass
                                        opacity: 0.55
                                    }

                                    Text {
                                        anchors.centerIn: parent
                                        rotation: -90
                                        // Rotated, so the spine's height is the
                                        // text's length to play with.
                                        width: spine.height - 52
                                        text: slot.modelData.title
                                        color: "#EDE6D6"
                                        font.family: Theme.display
                                        font.pixelSize: 13
                                        horizontalAlignment: Text.AlignHCenter
                                        elide: Text.ElideRight
                                    }

                                    Text {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        anchors.bottom: parent.bottom
                                        anchors.bottomMargin: 7
                                        text: slot.modelData.note_count
                                        color: "#EDE6D6"
                                        opacity: 0.7
                                        font.family: Theme.mono
                                        font.pixelSize: 9
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: shelf.bookPicked(slot.modelData.id)
                                    }
                                }
                            }
                        }
                    }

                    // The plank the books stand on.
                    Rectangle {
                        width: content.width
                        height: 2
                        color: Theme.pageLine
                    }
                }
            }
        }
    }
}
