import QtQuick
import QtQuick.Controls
import booklet

// A book's binding: the colour of its spine and the shelf it stands on. Both
// were hand-edited in the book's booklet.json until now; this writes that same
// file, and the shelf view re-reads it.
Popup {
    id: dialog

    modal: true
    focus: true
    padding: 0
    width: 320
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

    property string bookId: ""
    property string bookTitle: ""
    // Not named `color`/`shelf`: what is picked here is not what the dialog is.
    property string pickedColor: ""
    property string pickedShelf: ""

    // Reads the book's current binding out of the shelf data the library
    // already has, so the dialog opens showing what is true.
    function openFor(id) {
        var books = JSON.parse(Library.books())
        var book = books.find(function (candidate) { return candidate.id === id })
        if (book === undefined)
            return

        dialog.bookId = id
        dialog.bookTitle = book.title
        dialog.pickedColor = book.color
        dialog.pickedShelf = book.shelf
        shelfField.text = book.shelf
        dialog.open()
    }

    function save() {
        Library.set_binding(dialog.bookId, dialog.pickedColor, shelfField.text.trim())
        dialog.close()
    }

    background: Rectangle {
        color: Theme.panel
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    Column {
        width: parent.width
        padding: 16
        spacing: 14

        Text {
            text: "Binding"
            color: Theme.textBright
            font.family: Theme.display
            font.pixelSize: Theme.px(18)
        }
        Text {
            text: dialog.bookTitle
            color: Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(12)
        }

        Row {
            spacing: 7

            Repeater {
                // The binding palette is book data, not theme, so it is the
                // same list in either theme.
                model: Theme.bindings

                delegate: Rectangle {
                    id: swatch
                    required property string modelData

                    width: 30
                    height: 30
                    radius: 3
                    color: modelData
                    border.color: dialog.pickedColor === modelData ? Theme.brass : Theme.pageLine
                    border.width: dialog.pickedColor === modelData ? 2 : 1

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: dialog.pickedColor = swatch.modelData
                    }
                }
            }
        }

        Column {
            spacing: 5

            Text {
                text: "SHELF"
                color: Theme.textSoft
                font.family: Theme.ui
                font.pixelSize: Theme.px(10)
                font.letterSpacing: 1.5 * Theme.uiScale
            }

            TextField {
                id: shelfField
                width: dialog.width - 32
                padding: 8
                placeholderText: "Which shelf this book stands on"
                color: Theme.textBright
                font.family: Theme.ui
                font.pixelSize: Theme.px(13)
                background: Rectangle {
                    color: Theme.editBg
                    border.color: Theme.pageLine
                    border.width: 1
                    radius: 4
                }

                Keys.onReturnPressed: dialog.save()
                Keys.onEnterPressed: dialog.save()
                Keys.onEscapePressed: dialog.close()
            }
        }

        Row {
            spacing: 8

            TextButton {
                label: "Cancel"
                onClicked: dialog.close()
            }
            TextButton {
                label: "Save"
                filled: true
                onClicked: dialog.save()
            }
        }
    }
}
