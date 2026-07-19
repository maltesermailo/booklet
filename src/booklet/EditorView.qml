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

    // The open note was auto-merged and awaits review; drives the banner.
    property bool flagged: false
    // Asks Main to open the version-history modal for the open note.
    signal requestHistory(string path)

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

    function refreshFlag() {
        view.flagged = view.hasNote && Sync.is_flagged(NoteEditor.current_id())
    }

    // Sync merged the open note's file on disk; land the merge in the live editor
    // as minimal range edits so the undo stack and caret survive (no full
    // reassignment). The edits and the caret map use UTF-16 units, matching Qt.
    function applyMerge() {
        var hunks = JSON.parse(NoteEditor.reload_edits(editor.text))
        if (hunks.length > 0) {
            var caret = editor.cursorPosition

            view.loading = true
            for (var i = hunks.length - 1; i >= 0; i--) { // back-to-front keeps offsets valid
                var h = hunks[i]
                if (h.remove > 0)
                    editor.remove(h.pos, h.pos + h.remove)
                if (h.insert.length > 0)
                    editor.insert(h.pos, h.insert)
            }
            editor.cursorPosition = view.mapCaret(hunks, caret)
            view.loading = false
        }
        view.refreshFlag()
    }

    // Where a caret lands after the hunks apply — mirrors booklet_core::merge::map_caret.
    function mapCaret(hunks, caret) {
        var delta = 0
        for (var i = 0; i < hunks.length; i++) {
            var h = hunks[i]
            if (h.pos + h.remove <= caret)
                delta += h.insert.length - h.remove
            else if (h.pos <= caret)
                return h.pos + h.insert.length
            else
                break
        }
        return Math.max(0, caret + delta)
    }

    Connections {
        target: Sync
        // The open note's file changed under us (a remote merge).
        function onNote_changed(id) {
            if (id === NoteEditor.current_id())
                view.applyMerge()
        }
        // A flag may have been raised or dismissed elsewhere.
        function onStatus_changed(payload) { view.refreshFlag() }
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

    // Which line a position falls on. Only ever called on a click, so counting
    // the newlines before it is cheap enough.
    function lineOf(position) {
        return editor.text.substring(0, position).split("\n").length - 1
    }

    // The title of the [[wiki-link]] under (x, y), or "". The point has to land
    // on the link's glyphs, which is why this takes a point and not a position:
    // `positionAt` snaps to the *nearest* position, so a click in the empty
    // margin beside a line resolves to that line's end, and a click below the
    // text resolves to the note's end. A line ending in a link would otherwise
    // be followed from anywhere to the right of it.
    function linkAtPoint(x, y) {
        var text = editor.text
        var position = editor.positionAt(x, y)

        var open = text.lastIndexOf("[[", position)
        if (open < 0)
            return ""

        var close = text.indexOf("]]", open)
        if (close < 0 || close + 2 < position)
            return ""

        var inner = text.substring(open + 2, close)
        if (inner.indexOf("\n") !== -1) // not a link, just stray brackets
            return ""

        // The link's glyphs run from `open` to `close + 2`: off the caret's line
        // the markers are collapsed to no width, so those bounds sit exactly on
        // the rendered title, and on the caret's line they wrap the visible
        // `[[Title]]` — either way, the span the eye sees.
        var start = editor.positionToRectangle(open)
        var end = editor.positionToRectangle(close + 2)
        if (y < start.y || y > end.y + end.height)
            return ""

        // A long title can soft-wrap, and then the two ends sit on different
        // rows and no single x range describes it. The rows above still bound
        // it; that is loose by a margin's width and beats not following a
        // wrapped link at all.
        if (start.y === end.y && (x < start.x || x > end.x))
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
            highlighter.decorations = NoteEditor.decorations()

            var meta = JSON.parse(NoteEditor.meta())
            view.section = meta.section !== undefined ? meta.section : ""
            view.modified = meta.modified !== undefined ? meta.modified : -1
            view.refreshFlag()
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
        // ScrollView measures its content by the child's *implicit* size, and
        // the sheet below has an explicit height instead — leaving this unset
        // left it at -1, so a note of any length reported as fitting and would
        // not scroll.
        contentHeight: sheet.height
        ScrollBar.vertical.policy: ScrollBar.AsNeeded

        Item {
            id: sheet
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
                    font.pixelSize: Theme.px(11)
                    font.letterSpacing: 1.5 * Theme.uiScale
                }

                // The flagged-merge banner: an auto-merge landed here and wants a
                // look. Styled like Notice — an ember bar on the left — sitting
                // just below the meta line, shifting the editor down.
                Rectangle {
                    id: flagBanner
                    x: 34
                    y: meta.visible ? meta.y + meta.height + 8 : 22
                    width: page.width - 60
                    height: view.flagged ? Theme.row(30) : 0
                    visible: view.flagged
                    clip: true
                    color: Theme.editBg
                    radius: Theme.radiusSmall
                    border.color: Theme.pageLine
                    border.width: 1

                    Rectangle {
                        width: 3
                        height: parent.height
                        color: Theme.ember
                    }

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        x: 14
                        text: "Merged automatically — review it."
                        color: Theme.text
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(12)
                    }

                    Row {
                        anchors.right: parent.right
                        anchors.rightMargin: 8
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 8

                        TextButton {
                            label: "View history"
                            onClicked: view.requestHistory(NoteEditor.current_id())
                        }

                        // × = "I checked it" — clears the flag.
                        Rectangle {
                            width: 22
                            height: 22
                            anchors.verticalCenter: parent.verticalCenter
                            radius: 4
                            color: dismissHover.hovered ? Theme.activePill : "transparent"
                            HoverHandler { id: dismissHover }
                            Text {
                                anchors.centerIn: parent
                                text: "×"
                                color: dismissHover.hovered ? Theme.textBright : Theme.textDim
                                font.pixelSize: Theme.px(15)
                            }
                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: Sync.dismiss_flag(NoteEditor.current_id())
                            }
                        }
                    }
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
                                font.pixelSize: Theme.px(11)
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
                    y: flagBanner.visible ? flagBanner.y + flagBanner.height + 8
                                          : (meta.visible ? meta.y + meta.height + 8 : 22)
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
                        highlighter.decorations = NoteEditor.decorations()
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
                        id: highlighter
                        // Detaching from the document is what "Source" means:
                        // no styling, no hidden markers, just the markdown.
                        document: view.livePreview ? editor.textDocument : null
                        cursorPosition: editor.cursorPosition
                        knownTitles: view.knownTitles
                        markerColor: Theme.textDim
                        textColor: Theme.textBright
                        linkColor: Theme.ember
                        unresolvedColor: Theme.textSoft
                        codeBackground: Theme.codeBg
                        headingFamily: Theme.display
                        headingPixelSize: view.headingSize
                    }

                    // A link on a line you are not editing is rendered text —
                    // its markers are collapsed — so clicking it follows it, as
                    // Obsidian does. On the line holding the caret the markers
                    // are showing, you are editing the source, and the click
                    // belongs to the caret.
                    TapHandler {
                        id: linkTap
                        acceptedModifiers: Qt.NoModifier

                        // The caret as it was *before* this click: the editor
                        // moves it to the click on press, so by the time the tap
                        // completes every click looks like it landed on the
                        // caret's own line. Handlers see the press first, which
                        // is what makes this the line you were editing.
                        property int caretLineOnPress: -1

                        onPressedChanged: {
                            // No focus means no line is being edited, so a first
                            // click into the note still follows a link.
                            linkTap.caretLineOnPress =
                                editor.activeFocus ? view.lineOf(editor.cursorPosition) : -1
                        }

                        onTapped: (point) => {
                            var at = editor.positionAt(point.position.x, point.position.y)
                            if (view.lineOf(at) === linkTap.caretLineOnPress)
                                return

                            var title = view.linkAtPoint(point.position.x, point.position.y)
                            if (title !== "")
                                NoteEditor.open_by_title(title)
                        }
                    }

                    // ⌘+click follows a link even on the line you are editing,
                    // where a plain click has to stay the caret's.
                    TapHandler {
                        acceptedModifiers: Qt.ControlModifier
                        onTapped: (point) => {
                            var title = view.linkAtPoint(point.position.x, point.position.y)
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
        font.pixelSize: Theme.px(18)
    }
}
