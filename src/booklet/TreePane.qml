import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import booklet

// Obsidian-style file explorer over the flattened tree from Rust.
// Depth arrives per row; indentation is depth * 13. Toggling a folder is a
// slot call; Rust recomputes the visible rows and signals tree_changed.
Rectangle {
    id: pane
    color: Theme.sidebar

    property var rows: []
    property string currentId: ""   // the open note — the active row
    property string selectedId: ""  // the last row clicked — where new things land

    // Inline editing, either creating a child of `editParent` or renaming
    // `editId`. Empty `editMode` means nothing is being edited.
    property string editMode: ""    // "" | "note" | "section" | "rename"
    property string editParent: ""
    property string editId: ""

    signal hideRequested()
    signal searchRequested()

    // Icon paths from design/reference.html, on its 24×24 grid. Its <circle>
    // and <rect> shapes are written out as path data here.
    readonly property string newNoteIcon: "M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z M14 3v6h6 M12 12v5 M9.5 14.5h5"
    readonly property string newSectionIcon: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M12 11v5 M9.5 13.5h5"
    readonly property string searchIcon: "M18 11 A7 7 0 1 1 4 11 A7 7 0 1 1 18 11 M21 21l-4.35-4.35"
    readonly property string collapseIcon: "M7 20l5-5 5 5 M7 4l5 5 5-5"
    readonly property string hideSidebarIcon: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M9 4v16"

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

    function rowById(id) {
        for (var i = 0; i < pane.rows.length; i++)
            if (pane.rows[i].id === id)
                return pane.rows[i]
        return null
    }
    function dirname(path) {
        var cut = path.lastIndexOf("/")
        return cut > 0 ? path.substring(0, cut) : ""
    }
    function basename(path) {
        return path.substring(path.lastIndexOf("/") + 1)
    }

    // New things land in the selected section, or the selected note's section,
    // or — with nothing selected — at the vault root.
    function createParent() {
        var row = pane.selectedId === "" ? null : pane.rowById(pane.selectedId)
        if (row === null)
            return Library.active_vault()
        if (row.kind === "note") {
            var parent = pane.dirname(row.id)
            return parent === "" ? Library.active_vault() : parent
        }
        return row.id
    }

    function startCreate(kind) {
        pane.editId = ""
        pane.editParent = pane.createParent()
        pane.editMode = kind
        // Open the parent so the new row is somewhere you can see it. This
        // round-trips through tree_changed, and `visibleRows` re-inserts the
        // field once the rows come back.
        if (pane.editParent !== Library.active_vault())
            Library.reveal(pane.editParent)
    }

    function startRename(id) {
        pane.editParent = ""
        pane.editId = id
        pane.editMode = "rename"
    }

    function cancelEdit() {
        pane.editMode = ""
        pane.editParent = ""
        pane.editId = ""
    }

    function commitEdit(text) {
        var name = text.trim()
        var mode = pane.editMode
        var parent = pane.editParent
        var target = pane.editId
        pane.cancelEdit()

        if (name === "")
            return
        if (mode === "rename") {
            Library.rename(target, name)
        } else if (mode === "note") {
            var created = Library.create_note(parent, name)
            if (created !== "")
                NoteEditor.open(created)
        } else if (mode === "section") {
            Library.create_section(parent, name)
        }
    }

    // The rows plus, while creating, a placeholder row carrying the name field.
    readonly property var visibleRows: {
        if (pane.editMode !== "note" && pane.editMode !== "section")
            return pane.rows

        var out = pane.rows.slice()
        var at = 0
        var depth = 0
        for (var i = 0; i < out.length; i++) {
            if (out[i].id === pane.editParent) {
                at = i + 1
                depth = out[i].depth + 1
                break
            }
        }
        out.splice(at, 0, {
            id: "", title: "", depth: depth, kind: pane.editMode,
            color: "", expanded: false, has_children: false, placeholder: true
        })
        return out
    }

    // Action toolbar: new note, new section, search | collapse all, hide sidebar.
    Item {
        id: iconBar
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: 36

        Row {
            anchors.left: parent.left
            anchors.leftMargin: 4
            anchors.top: parent.top
            anchors.topMargin: 2
            spacing: 2

            IconButton {
                path: pane.newNoteIcon
                tip: "New note (⌘N)"
                onClicked: pane.startCreate("note")
            }
            IconButton {
                path: pane.newSectionIcon
                tip: "New section"
                onClicked: pane.startCreate("section")
            }
            IconButton {
                path: pane.searchIcon
                tip: "Search / quick switcher (⌘K)"
                onClicked: pane.searchRequested()
            }
        }

        Row {
            anchors.right: parent.right
            anchors.rightMargin: 4
            anchors.top: parent.top
            anchors.topMargin: 2
            spacing: 2

            IconButton {
                path: pane.collapseIcon
                tip: "Collapse all"
                onClicked: Library.collapse_all()
            }
            IconButton {
                path: pane.hideSidebarIcon
                tip: "Hide sidebar (⌘⌥S)"
                onClicked: pane.hideRequested()
            }
        }

        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width
            height: 1
            color: Theme.sidebarLine
        }
    }

    ListView {
        anchors.top: iconBar.bottom
        anchors.topMargin: 6
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 6
        model: pane.visibleRows
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        delegate: Rectangle {
            id: rowItem
            required property var modelData

            readonly property bool placeholder: modelData.placeholder === true
            readonly property bool renaming: pane.editMode === "rename" && modelData.id === pane.editId
            readonly property bool editing: placeholder || renaming

            width: ListView.view ? ListView.view.width : 0
            height: 24
            radius: 4
            color: modelData.id === pane.currentId && !rowItem.editing ? Theme.activePill
                 : hover.hovered ? Qt.rgba(1, 1, 1, 0.03) : "transparent"

            HoverHandler { id: hover }

            // Indent guides: the reference nests each level in a container with
            // a 1px left rule, so a row at depth d sits behind d of them. Each
            // row draws its own segment; stacked, they read as continuous lines.
            Repeater {
                model: modelData.depth
                delegate: Rectangle {
                    required property int index
                    x: 8 + index * 13
                    width: 1
                    height: parent.height
                    color: Theme.sidebarLine
                }
            }

            Row {
                visible: !rowItem.editing
                anchors.verticalCenter: parent.verticalCenter
                x: 8 + modelData.depth * 13
                spacing: 6

                Text { // chevron
                    visible: modelData.has_children
                    text: modelData.expanded ? "▾" : "▸"
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
                    // Per the reference: books bright, sections plain ink, notes
                    // secondary — and whatever is open is bright.
                    text: modelData.title
                    color: modelData.id === pane.currentId ? Theme.textBright
                         : modelData.kind === "book" ? Theme.textBright
                         : modelData.kind === "note" ? Theme.textSoft
                         : Theme.text
                    font.family: Theme.ui
                    font.pixelSize: 13
                    font.weight: modelData.kind === "book" ? Font.Medium : Font.Normal
                    elide: Text.ElideRight
                }
            }

            // Naming a new note/section, or renaming an existing one.
            TextField {
                id: nameField
                visible: rowItem.editing
                anchors.verticalCenter: parent.verticalCenter
                x: 8 + modelData.depth * 13 + 14
                width: Math.max(40, parent.width - x - 6)
                height: 21
                padding: 3
                text: rowItem.placeholder ? "" : modelData.title
                color: Theme.textBright
                font.family: Theme.ui
                font.pixelSize: 13
                selectByMouse: true

                background: Rectangle {
                    color: Theme.editBg
                    border.color: Theme.brass
                    border.width: 1
                    radius: 3
                }

                onVisibleChanged: {
                    if (visible) {
                        forceActiveFocus()
                        selectAll()
                    }
                }
                Keys.onReturnPressed: pane.commitEdit(nameField.text)
                Keys.onEnterPressed: pane.commitEdit(nameField.text)
                Keys.onEscapePressed: pane.cancelEdit()
                onActiveFocusChanged: {
                    if (!activeFocus && rowItem.editing)
                        pane.cancelEdit()
                }
            }

            MouseArea {
                anchors.fill: parent
                enabled: !rowItem.editing
                acceptedButtons: Qt.LeftButton | Qt.RightButton

                onClicked: (mouse) => {
                    pane.selectedId = modelData.id
                    if (mouse.button === Qt.RightButton) {
                        rowMenu.targetId = modelData.id
                        rowMenu.targetTitle = modelData.title
                        rowMenu.popup()
                        return
                    }
                    if (modelData.kind === "note")
                        NoteEditor.open(modelData.id)
                    else
                        Library.toggle(modelData.id)
                }
            }
        }
    }

    Menu {
        id: rowMenu
        property string targetId: ""
        property string targetTitle: ""

        MenuItem { text: "New note"; onTriggered: pane.startCreate("note") }
        MenuItem { text: "New section"; onTriggered: pane.startCreate("section") }
        MenuSeparator {}
        MenuItem { text: "Rename…"; onTriggered: pane.startRename(rowMenu.targetId) }
        MenuItem {
            text: "Delete…"
            onTriggered: {
                deleteDialog.targetId = rowMenu.targetId
                deleteDialog.targetTitle = rowMenu.targetTitle
                deleteDialog.open()
            }
        }
    }

    MessageDialog {
        id: deleteDialog
        property string targetId: ""
        property string targetTitle: ""

        title: "Delete"
        text: "Move “" + deleteDialog.targetTitle + "” to the Trash?"
        informativeText: "You can put it back from the Trash."
        buttons: MessageDialog.Ok | MessageDialog.Cancel
        onAccepted: Library.delete_entry(deleteDialog.targetId)
    }
}
