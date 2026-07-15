import QtQuick
import QtQuick.Controls
import booklet

// Cmd/Ctrl+K note finder, over the active vault. Titles match as you type,
// straight from the list the tree already has; a word you remember from inside
// a note matches too, which costs a scan of the vault and so waits for a pause.
// Each row shows a breadcrumb to tell same-titled notes apart.
Popup {
    id: switcher

    modal: true
    focus: true
    padding: 0
    width: 520
    height: 380
    anchors.centerIn: Overlay.overlay

    // Modals arrive rather than appear; same motion as the menus.
    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1
                          duration: Theme.gentle; easing.type: Theme.easing }
        NumberAnimation { property: "scale"; from: 0.97; to: 1
                          duration: Theme.gentle; easing.type: Theme.easing }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0
                          duration: Theme.quick; easing.type: Theme.easing }
    }

    property var notes: []
    // Notes whose text holds the query. Filled by the scan, not by typing.
    property var textHits: []
    property string query: ""

    // One letter matches most of the vault, which is a scan for nothing.
    readonly property int searchFloor: 2

    readonly property var titleMatches: {
        var needle = query.trim().toLowerCase()
        if (needle === "")
            return notes
        return notes.filter(function (note) {
            return note.title.toLowerCase().indexOf(needle) !== -1
                || note.context.toLowerCase().indexOf(needle) !== -1
        })
    }

    // Titles first — a note you can name is the one you meant. Then the notes
    // that merely say the word, each shown with the line it says it on. A note
    // matched both ways is listed once, under its title.
    readonly property var matches: {
        var rows = []
        var named = []

        for (var i = 0; i < titleMatches.length; i++) {
            var note = titleMatches[i]
            rows.push({ "id": note.id, "title": note.title, "detail": note.context,
                        "inText": false })
            named.push(note.id)
        }

        for (var j = 0; j < textHits.length; j++) {
            var hit = textHits[j]
            if (named.indexOf(hit.id) === -1)
                rows.push({ "id": hit.id, "title": hit.title, "detail": hit.snippet,
                            "inText": true })
        }

        return rows
    }

    // The scan reads every note in the vault, so it waits for you to stop typing
    // rather than running on each keystroke.
    Timer {
        id: scan
        interval: 180
        onTriggered: switcher.textHits = JSON.parse(Library.search(switcher.query))
    }

    // Named to avoid clashing with Popup's own open().
    function openSwitcher() {
        switcher.notes = JSON.parse(Library.notes())
        switcher.textHits = []
        switcher.query = ""
        field.text = ""
        switcher.open()
        field.forceActiveFocus()
    }

    function accept() {
        if (list.currentIndex < 0 || list.currentIndex >= matches.length)
            return
        NoteEditor.open(matches[list.currentIndex].id)
        switcher.close()
    }

    background: Rectangle {
        color: Theme.panel
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    Column {
        anchors.fill: parent
        spacing: 0

        TextField {
            id: field
            width: parent.width
            padding: 14
            placeholderText: "Find a note, or a word in one…"
            color: Theme.textBright
            font.family: Theme.ui
            font.pixelSize: Theme.px(15)
            background: Rectangle {
                color: "transparent"
                // Only the bottom edge, as a divider under the field.
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: 1
                    color: Theme.pageLine
                }
            }

            onTextChanged: {
                switcher.query = text
                list.currentIndex = 0

                if (text.trim().length >= switcher.searchFloor) {
                    scan.restart()
                } else {
                    scan.stop()
                    switcher.textHits = []
                }
            }
            Keys.onDownPressed: list.incrementCurrentIndex()
            Keys.onUpPressed: list.decrementCurrentIndex()
            Keys.onReturnPressed: switcher.accept()
            Keys.onEnterPressed: switcher.accept()
            Keys.onEscapePressed: switcher.close()
        }

        ListView {
            id: list
            width: parent.width
            height: parent.height - field.height
            model: switcher.matches
            clip: true
            currentIndex: 0
            boundsBehavior: Flickable.StopAtBounds

            delegate: Rectangle {
                required property var modelData
                required property int index

                width: ListView.view.width - Theme.gap(8)
                x: Theme.gap(4)
                height: Theme.row(46)
                radius: Theme.radiusSmall
                color: index === list.currentIndex ? Theme.activePill : "transparent"

                Behavior on color {
                    ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
                }

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    spacing: 2

                    Row {
                        spacing: 6
                        width: parent.width

                        Text {
                            text: modelData.title
                            color: Theme.textBright
                            font.family: Theme.display
                            font.pixelSize: Theme.px(15)
                            elide: Text.ElideRight
                        }
                        // Says why this row is here: the title does not match,
                        // the writing does.
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            visible: modelData.inText
                            text: "IN TEXT"
                            color: Theme.brass
                            font.family: Theme.ui
                            font.pixelSize: Theme.px(9)
                            font.letterSpacing: 1 * Theme.uiScale
                        }
                    }
                    Text {
                        text: modelData.detail
                        color: Theme.textSoft
                        font.family: modelData.inText ? Theme.body : Theme.ui
                        font.pixelSize: Theme.px(11)
                        elide: modelData.inText ? Text.ElideRight : Text.ElideMiddle
                        width: parent.width
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: {
                        list.currentIndex = index
                        switcher.accept()
                    }
                }
            }
        }

        Text {
            visible: switcher.matches.length === 0
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            topPadding: 40
            text: "No notes match"
            color: Theme.textDim
            font.family: Theme.ui
            font.pixelSize: Theme.px(13)
        }
    }
}
