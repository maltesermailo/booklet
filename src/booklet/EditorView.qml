import QtQuick
import QtQuick.Controls
import booklet

// The note editor: one surface over the whole note, always editable — no mode
// to enter, no block to click. What you edit is the markdown itself; the C++
// highlighter (src/cpp/) styles it live, showing the syntax markers only on the
// line holding the caret and collapsing them to nothing everywhere else.
//
// The sheet grows with the note and the whole thing scrolls, so a note can run
// as long as it likes.
Rectangle {
    id: view
    color: Theme.bg

    property bool hasNote: false
    // Loading a note assigns `text`, which fires onTextChanged; without this
    // the editor would schedule a save of what it just read back.
    property bool loading: false
    // Live preview off = the markdown as written, unstyled.
    property bool livePreview: true

    // Which [[links]] resolve. Kept in step with the vault so a link broken by
    // a rename shows as unresolved.
    property var knownTitles: []
    property string section: ""
    property real modified: -1

    // Reading size, persisted. Headings keep the reference's ratio to it
    // (24 against a 15px body).
    property int fontSize: 18
    readonly property int headingSize: Math.round(fontSize * 1.6)

    function reloadTitles() {
        var notes = JSON.parse(Library.notes())
        var titles = []
        for (var i = 0; i < notes.length; i++)
            titles.push(notes[i].title)
        view.knownTitles = titles
    }

    // "EDITED TODAY", as the reference words it.
    function editedWhen(epochSeconds) {
        if (epochSeconds < 0)
            return ""

        var then = new Date(epochSeconds * 1000)
        var days = Math.floor((new Date().setHours(0, 0, 0, 0) - new Date(epochSeconds * 1000).setHours(0, 0, 0, 0))
                              / 86400000)
        if (days <= 0)
            return "EDITED TODAY"
        if (days === 1)
            return "EDITED YESTERDAY"
        if (days < 7)
            return "EDITED " + days + " DAYS AGO"
        return "EDITED " + then.toLocaleDateString(Qt.locale(), "d MMM yyyy").toUpperCase()
    }

    function reloadFontSize() {
        view.fontSize = Library.editor_font_size()
    }

    Component.onCompleted: {
        reloadTitles()
        reloadFontSize()
    }

    Connections {
        target: Library
        function onTree_changed() { view.reloadTitles() }
        function onFont_size_changed() { view.reloadFontSize() }
    }

    // Plain clicks belong to the editor, so links are followed with ⌘+click —
    // Qt maps ⌘ to ControlModifier on macOS.
    function linkAt(position) {
        var text = editor.text
        var open = text.lastIndexOf("[[", position)
        if (open < 0)
            return ""

        var close = text.indexOf("]]", open)
        if (close < 0 || close + 2 < position)
            return ""

        var inner = text.substring(open + 2, close)
        if (inner.indexOf("\n") !== -1) // not a link, just stray brackets
            return ""

        var alias = inner.indexOf("|")
        return alias >= 0 ? inner.substring(0, alias) : inner
    }

    Connections {
        target: NoteEditor
        function onNote_opened(id, title) {
            view.hasNote = id !== ""
            saveTimer.stop()

            // Set, never bind: re-reading the source while typing would move
            // the caret. `open` has already flushed the note we came from.
            view.loading = true
            editor.text = NoteEditor.source()
            view.loading = false

            var meta = JSON.parse(NoteEditor.meta())
            view.section = meta.section !== undefined ? meta.section : ""
            view.modified = meta.modified !== undefined ? meta.modified : -1
        }
        // Writing the note moves its timestamp.
        function onSave_state_changed(unsaved) {
            if (!unsaved) {
                var meta = JSON.parse(NoteEditor.meta())
                view.modified = meta.modified !== undefined ? meta.modified : -1
            }
        }
    }

    // The text reaches Rust on every keystroke, but only reaches the disk on a
    // pause: every write wakes the file watcher, which re-reads the vault.
    Timer {
        id: saveTimer
        interval: 400
        onTriggered: NoteEditor.flush()
    }

    ScrollView {
        id: scroll
        anchors.fill: parent
        clip: true
        contentWidth: availableWidth // never scroll sideways
        ScrollBar.vertical.policy: ScrollBar.AsNeeded

        Item {
            width: scroll.availableWidth
            height: page.height + 32

            // The sheet fills the editor's width. The reference caps it at
            // 560px and centres it; using the whole field is a deliberate
            // override.
            Rectangle {
                id: page
                x: 14
                y: 16
                width: Math.max(280, parent.width - 28)
                // At least a screen of paper, taller once the note outgrows it:
                // 22 above the text, 28 below.
                height: Math.max(editor.implicitHeight + 50, view.height - 32)
                color: Theme.page
                border.color: Theme.pageLine
                border.width: 1
                radius: 4

                // Stitched inner margin — the one bookish flourish on the page.
                // 1px, 16px in from the left edge, 5px dashes with 6px gaps.
                Column {
                    x: 16
                    y: 10
                    spacing: 6
                    clip: true
                    Repeater {
                        // Clamp at 0: briefly negative during initial layout.
                        model: Math.max(0, Math.floor((page.height - 20) / 11))
                        Rectangle { width: 1; height: 5; color: Theme.pageLine }
                    }
                }

                // "● PIXEL 7 · EDITED TODAY" — where this note sits and when it
                // last changed.
                Text {
                    id: meta
                    x: 34
                    y: 22
                    visible: view.hasNote && text !== ""
                    text: {
                        var when = view.editedWhen(view.modified)
                        if (view.section === "")
                            return when
                        return "● " + view.section.toUpperCase() + (when === "" ? "" : " · " + when)
                    }
                    color: Theme.textSoft
                    font.family: Theme.ui
                    font.pixelSize: 11
                    font.letterSpacing: 1.5
                }

                // Preview | Source. Source turns the highlighter off, leaving the
                // markdown exactly as written.
                Row {
                    anchors.top: parent.top
                    anchors.right: parent.right
                    anchors.topMargin: 12
                    anchors.rightMargin: 12
                    visible: view.hasNote
                    spacing: 0

                    Repeater {
                        model: ["Preview", "Source"]

                        delegate: Rectangle {
                            required property string modelData
                            required property int index

                            readonly property bool on: (index === 0) === view.livePreview

                            width: label.implicitWidth + 18
                            height: 18
                            color: on ? Theme.brass : "transparent"
                            border.color: Theme.pageLine
                            border.width: 1

                            Text {
                                id: label
                                anchors.centerIn: parent
                                text: modelData
                                color: parent.on ? Theme.page : Theme.textSoft
                                font.family: Theme.ui
                                font.pixelSize: 11
                            }

                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: view.livePreview = (index === 0)
                            }
                        }
                    }
                }

                TextArea {
                    id: editor
                    x: 34
                    y: meta.visible ? meta.y + meta.height + 8 : 22
                    width: page.width - 60 // 34 left, 26 right
                    // Fill the sheet even when the note is short, so clicking
                    // the blank paper below the text lands in the editor and
                    // puts the caret on the last line, as Obsidian does.
                    height: Math.max(implicitHeight, page.height - 50)
                    visible: view.hasNote
                    padding: 0
                    wrapMode: TextArea.Wrap
                    selectByMouse: true
                    color: Theme.text
                    selectionColor: Theme.brassDeep
                    font.family: view.livePreview ? Theme.body : Theme.mono
                    font.pixelSize: view.livePreview ? view.fontSize : Math.round(view.fontSize * 0.87)
                    background: null // the page is the background

                    onTextChanged: {
                        if (view.loading)
                            return
                        NoteEditor.set_source(editor.text)
                        saveTimer.restart()
                    }
                    onActiveFocusChanged: {
                        // Don't sit on unsaved text when focus leaves.
                        if (!activeFocus) {
                            saveTimer.stop()
                            NoteEditor.flush()
                        }
                    }

                    MarkdownHighlighter {
                        // Detaching from the document is what "Source" means:
                        // no styling, no hidden markers, just the markdown.
                        document: view.livePreview ? editor.textDocument : null
                        cursorPosition: editor.cursorPosition
                        knownTitles: view.knownTitles
                        markerColor: Theme.textDim
                        textColor: Theme.textBright
                        linkColor: Theme.ember
                        unresolvedColor: Theme.textSoft
                        headingFamily: Theme.display
                        headingPixelSize: view.headingSize
                    }

                    TapHandler {
                        // Only fires with ⌘ held, so ordinary clicks still place
                        // the caret.
                        acceptedModifiers: Qt.ControlModifier
                        onTapped: (point) => {
                            var at = editor.positionAt(point.position.x, point.position.y)
                            var title = view.linkAt(at)
                            if (title !== "")
                                NoteEditor.open_by_title(title)
                        }
                    }
                }
            }
        }
    }

    Text {
        visible: !view.hasNote
        anchors.centerIn: parent
        text: "Choose a note from the index"
        color: Theme.textDim
        font.family: Theme.display
        font.pixelSize: 18
    }
}
