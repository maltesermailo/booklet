import QtQuick
import QtQuick.Controls
import booklet

// Cmd/Ctrl+K note finder. Navigation spans every configured vault (unlike
// wiki-links, which stay inside one vault), so each row shows a breadcrumb to
// tell same-titled notes apart.
Popup {
    id: switcher

    modal: true
    focus: true
    padding: 0
    width: 520
    height: 380
    anchors.centerIn: Overlay.overlay

    property var notes: []
    property string query: ""

    readonly property var matches: {
        var needle = query.trim().toLowerCase()
        if (needle === "")
            return notes
        return notes.filter(function (note) {
            return note.title.toLowerCase().indexOf(needle) !== -1
                || note.context.toLowerCase().indexOf(needle) !== -1
        })
    }

    // Named to avoid clashing with Popup's own open().
    function openSwitcher() {
        switcher.notes = JSON.parse(Library.notes())
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
        radius: 8
    }

    Column {
        anchors.fill: parent
        spacing: 0

        TextField {
            id: field
            width: parent.width
            padding: 14
            placeholderText: "Find a note…"
            color: Theme.textBright
            font.family: Theme.ui
            font.pixelSize: 15
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

                width: ListView.view.width
                height: 46
                color: index === list.currentIndex ? Theme.activePill : "transparent"

                Column {
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14
                    spacing: 2

                    Text {
                        text: modelData.title
                        color: Theme.textBright
                        font.family: Theme.display
                        font.pixelSize: 15
                        elide: Text.ElideRight
                        width: parent.width
                    }
                    Text {
                        text: modelData.context
                        color: Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: 11
                        elide: Text.ElideMiddle
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
            font.pixelSize: 13
        }
    }
}
