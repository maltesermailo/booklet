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
    property string pulseId: ""     // a just-revealed row, briefly glowing

    // Scrolls the tree to `id` and pulses its row, so clicking a breadcrumb
    // segment shows where it lives. Deferred with callLater so it runs after the
    // Library.reveal that expanded the ancestors has refreshed the rows.
    function revealTo(id) {
        pane.pendingReveal = id
        Qt.callLater(pane.applyReveal)
    }
    property string pendingReveal: ""
    function applyReveal() {
        var id = pane.pendingReveal
        var index = -1
        for (var i = 0; i < pane.visibleRows.length; i++) {
            if (pane.visibleRows[i].id === id) {
                index = i
                break
            }
        }
        if (index < 0)
            return

        treeList.positionViewAtIndex(index, ListView.Center)
        pane.pulseId = id
        pulseTimer.restart()
    }

    Timer {
        id: pulseTimer
        interval: 850 // long enough to notice, then the row eases back
        onTriggered: pane.pulseId = ""
    }

    // Inline editing, either creating a child of `editParent` or renaming
    // `editId`. Empty `editMode` means nothing is being edited.
    property string editMode: ""    // "" | "note" | "section" | "rename"
    property string editParent: ""
    property string editId: ""

    // Exposed so a window-level overlay can close the inline field on a press
    // anywhere but the field itself. `editField` is the live name field while
    // one is up (see `beginEditing`), null otherwise.
    readonly property bool isEditing: editMode !== ""
    property Item editField: null

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
        pane.editField = null
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
        height: Theme.row(36)

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
        id: treeList
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

            readonly property bool pulsing: !placeholder && modelData.id === pane.pulseId

            width: ListView.view ? ListView.view.width : 0
            height: Theme.row(24)
            radius: Theme.radiusSmall
            // The hover lift is the theme's own ink at 3%, not white: the
            // reference hardcodes rgba(255,255,255,.03) here, which is invisible
            // on vellum's paper. Deriving it from the ink darkens on a light
            // theme and lightens on a dark one, which is what the 3% meant.
            // A just-revealed row glows in brass, then eases back via the Behavior.
            color: rowItem.pulsing ? Qt.rgba(Theme.brass.r, Theme.brass.g, Theme.brass.b, 0.45)
                 : modelData.id === pane.currentId && !rowItem.editing ? Theme.activePill
                 : hover.hovered ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03)
                 : "transparent"

            Behavior on color {
                ColorAnimation { duration: Theme.gentle; easing.type: Theme.easing }
            }

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
                    font.pixelSize: Theme.px(10)
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
                    font.pixelSize: Theme.px(13)
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
                height: Theme.row(21)
                padding: 3
                text: rowItem.placeholder ? "" : modelData.title
                color: Theme.textBright
                font.family: Theme.ui
                font.pixelSize: Theme.px(13)
                selectByMouse: true

                background: Rectangle {
                    color: Theme.editBg
                    border.color: Theme.brass
                    border.width: 1
                    radius: 3
                }

                function beginEditing() {
                    pane.editField = nameField
                    nameField.forceActiveFocus()
                    nameField.selectAll()
                }

                // Two triggers, because the two edits arrive by different routes.
                // Rename reuses the row's own delegate, so the field really does
                // flip from hidden to shown and onVisibleChanged catches it. A
                // new note or section splices in a placeholder row whose delegate
                // is *born* editing: visible starts true and never changes, so
                // Component.onCompleted is the only hook it has.
                //
                // Deferred there, and only there: at Component.onCompleted the
                // delegate is not in the view yet, so focus does not stick. It is
                // handed straight back, the focus-out guard below reads that as
                // "clicked away" and calls cancelEdit, and since editMode is what
                // visibleRows is built from, the binding loops and the field is
                // gone before you can type. callLater waits for the view to take
                // the delegate. Rename needs no such wait — its delegate is
                // already in there.
                Component.onCompleted: if (visible) Qt.callLater(nameField.beginEditing)
                onVisibleChanged: if (visible) nameField.beginEditing()
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
                        rowMenu.targetKind = modelData.kind
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

    AppMenu {
        id: rowMenu
        property string targetId: ""
        property string targetTitle: ""
        property string targetKind: ""

        // The three actions below open the inline name field, and the field takes
        // the caret. A Popup restores focus to whatever held it when it opened,
        // and that restore lands *after* onTriggered — once the exit transition
        // has run, ~110ms later. So the field appeared, took focus, and had it
        // pulled straight back out, which the field's focus-out guard reads as
        // "clicked away" and cancels: the name field vanished the instant it
        // appeared. Nothing shorter than the animation can win that race, so the
        // action waits for the menu to actually be gone.
        //
        // Binding… and Delete… open their own dialogs and are left alone: they do
        // not depend on holding the caret, and this only bites what does.
        property var pendingAction: null
        onClosed: {
            var action = rowMenu.pendingAction
            rowMenu.pendingAction = null
            if (action)
                action()
        }

        AppMenuItem {
            text: "New note"
            onTriggered: rowMenu.pendingAction = () => pane.startCreate("note")
        }
        AppMenuItem {
            text: "New section"
            onTriggered: rowMenu.pendingAction = () => pane.startCreate("section")
        }
        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.pageLine
            }
        }
        AppMenuItem {
            text: "Rename…"
            // targetId is read when the menu closes, not now; nothing clears it
            // in between.
            onTriggered: rowMenu.pendingAction = () => pane.startRename(rowMenu.targetId)
        }
        // Only a book has a binding: sections and notes are not bound.
        AppMenuItem {
            text: "Binding…"
            height: visible ? implicitHeight : 0
            visible: rowMenu.targetKind === "book"
            onTriggered: bindingDialog.openFor(rowMenu.targetId)
        }
        AppMenuItem {
            text: "Delete…"
            onTriggered: {
                deleteDialog.targetId = rowMenu.targetId
                deleteDialog.targetTitle = rowMenu.targetTitle
                deleteDialog.open()
            }
        }
    }

    BindingDialog { id: bindingDialog }

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
