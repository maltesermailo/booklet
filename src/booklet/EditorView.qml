import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
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

    // Inline widgets drawn over the collapsed source — task checkboxes,
    // horizontal rules, and table grids. Filled from each parse by
    // refreshDecorations(); the highlighter styles everything else as character
    // formats.
    property var taskWidgets: []
    property var ruleWidgets: []
    property var tableWidgets: []
    property var imageWidgets: []

    // The open note's folder, so a relative image src resolves beside it.
    property string noteDir: ""

    // Measured render height per image, keyed by the image's document offset.
    // The image overlay fills this as each picture loads; the highlighter reserves
    // the matching height so text flows below the drawn image.
    property var imageHeights: ({})

    // The screen rect of a document position, for placing a widget overlay. Reads
    // imageHeights so the binding re-evaluates when an image loads and reflows the
    // text below it — positionToRectangle alone does not track that reflow, which
    // otherwise leaves widgets under an image stranded at stale positions.
    function boxAt(position) {
        var reflow = view.imageHeights // dependency, intentionally unused
        return editor.positionToRectangle(position)
    }

    // Resolves an image's `src` to a URL the Image element can load: an absolute
    // URL as-is, otherwise a file URL relative to the note's folder.
    function imageUrl(src) {
        if (/^[a-z]+:\/\//i.test(src))
            return src
        var path = src.charAt(0) === "/" ? src : view.noteDir + "/" + src
        return "file://" + encodeURI(path)
    }

    // The image extensions the add methods accept (mirrors booklet_core::image).
    function isImagePath(path) {
        return /\.(png|jpe?g|gif|webp|svg|bmp)$/i.test(path)
    }

    // Writes an `![](name)` link at the caret, on its own line so it renders as a
    // block image. `name` is the file just placed in the note's folder ("" = the
    // add failed, and the error was already reported).
    function insertImage(name) {
        if (name === "")
            return
        var pos = editor.cursorPosition
        var atLineStart = pos === 0 || editor.text.charAt(pos - 1) === "\n"
        editor.insert(pos, (atLineStart ? "" : "\n") + "![](" + name + ")\n")
    }

    // Strips a file URL down to a local path (drag-drop and the picker hand out
    // `file://…` URLs, percent-encoded).
    function localPath(fileUrl) {
        return decodeURIComponent(fileUrl.toString().replace(/^file:\/\//, ""))
    }

    // Records a loaded image's height and pushes the map to the highlighter, which
    // reserves that space (a no-op when the height is unchanged).
    function reportImageHeight(start, height) {
        if (view.imageHeights[start] === height)
            return
        var next = {}
        for (var key in view.imageHeights)
            next[key] = view.imageHeights[key]
        next[start] = height
        view.imageHeights = next
    }

    // Pixel height of one rendered table row. The highlighter reserves this ×
    // (rows) of document height per table so the drawn grid has room; both sides
    // must use the same value or the grid and the text below it disagree.
    readonly property int tableRowHeight: Math.round(view.fontSize * 2.0)

    // Splits a GFM table's source into a grid: `rows` (the `|---|` separator row
    // dropped, since it is drawn as the header underline) and the column count.
    function parseTable(start, len) {
        var lines = editor.text.substring(start, start + len).split("\n")
        var rows = []
        var cols = 0
        for (var i = 0; i < lines.length; i++) {
            var line = lines[i].trim()
            if (line === "")
                continue
            if (/^[\s|:\-]+$/.test(line) && line.indexOf("-") !== -1)
                continue // the separator row

            var parts = line.split("|")
            var cells = []
            for (var j = 0; j < parts.length; j++) {
                // A leading/trailing `|` yields an empty edge piece — drop it.
                if ((j === 0 || j === parts.length - 1) && parts[j].trim() === "")
                    continue
                cells.push(parts[j].trim())
            }
            rows.push(cells)
            cols = Math.max(cols, cells.length)
        }
        return { rows: rows, cols: cols }
    }

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

    // One parse feeds both the highlighter (character formats) and the widget
    // overlays. Tasks and rules become QML items laid over the collapsed source;
    // the JSON is walked once here rather than again inside each overlay.
    function refreshDecorations() {
        var json = NoteEditor.decorations()
        highlighter.decorations = json

        var decos = JSON.parse(json)
        var tasks = []
        var rules = []
        var tables = []
        var images = []
        for (var i = 0; i < decos.length; i++) {
            if (decos[i].kind === "task")
                tasks.push(decos[i])
            else if (decos[i].kind === "rule")
                rules.push(decos[i])
            else if (decos[i].kind === "table")
                tables.push(decos[i])
            else if (decos[i].kind === "image")
                images.push(decos[i])
        }
        view.taskWidgets = tasks
        view.ruleWidgets = rules
        view.tableWidgets = tables
        view.imageWidgets = images
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
            view.noteDir = id.lastIndexOf("/") >= 0 ? id.substring(0, id.lastIndexOf("/")) : ""
            view.imageHeights = ({}) // heights belong to the note we are leaving
            saveTimer.stop()

            // Set, never bind: re-reading the source while typing would move
            // the caret. `open` has already flushed the note we came from.
            view.loading = true
            editor.text = NoteEditor.source()
            view.loading = false
            view.refreshDecorations()

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
                        view.refreshDecorations()
                        saveTimer.restart()
                    }
                    onActiveFocusChanged: {
                        // Don't sit on unsaved text when focus leaves.
                        if (!activeFocus) {
                            saveTimer.stop()
                            NoteEditor.flush()
                        }
                    }

                    // Pasting an image: save the clipboard picture into the note's
                    // folder and link it, instead of the plain-text paste that a
                    // markdown editor would otherwise get (nothing useful). A
                    // clipboard without an image falls through to the normal paste.
                    // Qt maps ⌘ to Ctrl on macOS, matching the app's shortcuts.
                    Keys.onPressed: (event) => {
                        if ((event.modifiers & Qt.ControlModifier) && event.key === Qt.Key_V) {
                            var encoded = clipboardImage.pngBase64()
                            if (encoded !== "") {
                                view.insertImage(NoteEditor.save_pasted_image(encoded))
                                event.accepted = true
                            }
                        }
                    }

                    // Dropping image files onto the sheet copies each into the
                    // note's folder and links it at the drop point. Non-images are
                    // ignored (a dropped note or PDF is not what this is for).
                    DropArea {
                        anchors.fill: parent
                        // Only claim file drops; a text drag still reaches the editor.
                        keys: ["text/uri-list"]
                        onDropped: (drop) => {
                            if (!drop.hasUrls)
                                return
                            editor.forceActiveFocus()
                            editor.cursorPosition = editor.positionAt(drop.x, drop.y)
                            for (var i = 0; i < drop.urls.length; i++) {
                                var path = view.localPath(drop.urls[i])
                                if (view.isImagePath(path))
                                    view.insertImage(NoteEditor.import_image(path))
                            }
                            drop.accept()
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
                        codeKeyword: Theme.codeKeyword
                        codeString: Theme.codeString
                        codeComment: Theme.codeComment
                        codeNumber: Theme.codeNumber
                        codeFunction: Theme.codeFunction
                        codeType: Theme.codeType
                        codeConstant: Theme.codeConstant
                        headingFamily: Theme.display
                        headingPixelSize: view.headingSize
                        tableRowHeight: view.tableRowHeight
                        imageHeights: JSON.stringify(view.imageHeights)
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

                    // Inline widgets, drawn over the source the highlighter has
                    // collapsed. They are children of the editor, so they live in
                    // its coordinate space and scroll with the text; the document
                    // layout places each one via positionToRectangle. On the
                    // caret's own line the raw markdown shows instead, so the
                    // overlay steps aside and there is something to edit. The
                    // models rebuild on every parse (refreshDecorations), which is
                    // what keeps positions fresh as the text above reflows.

                    // Task checkboxes: a real box over the hidden `[ ]` / `[x]`;
                    // clicking toggles the one char between the brackets, which
                    // flows back through the normal edit path (undoable).
                    Repeater {
                        model: view.livePreview ? view.taskWidgets : []

                        delegate: Rectangle {
                            required property var modelData

                            readonly property rect box: view.boxAt(modelData.start)
                            readonly property rect boxEnd: view.boxAt(modelData.start + modelData.len)
                            readonly property bool checked: modelData.flag

                            width: Math.round(view.fontSize * 0.9)
                            height: width
                            radius: 3
                            // Centre the box in the width the `[ ]` reserves; a
                            // wrapped end (boxEnd on another row) falls back to the left.
                            x: box.x + Math.max(0, (boxEnd.x - box.x - width) / 2)
                            y: box.y + (box.height - height) / 2
                            visible: view.lineOf(editor.cursorPosition) !== view.lineOf(modelData.start)
                            color: checked ? Theme.ember : "transparent"
                            border.color: checked ? Theme.ember : Theme.textSoft
                            border.width: 1.5

                            Text {
                                anchors.centerIn: parent
                                visible: parent.checked
                                text: "✓" // ✓
                                color: Theme.page
                                font.pixelSize: Math.round(parent.height * 0.8)
                            }

                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    var pos = modelData.start + 1 // the char inside the brackets
                                    editor.remove(pos, pos + 1)
                                    editor.insert(pos, parent.checked ? " " : "x")
                                }
                            }
                        }
                    }

                    // Horizontal rules: a hairline across the sheet where the
                    // `---` line has been collapsed to an empty line.
                    Repeater {
                        model: view.livePreview ? view.ruleWidgets : []

                        delegate: Rectangle {
                            required property var modelData

                            readonly property rect box: view.boxAt(modelData.start)

                            x: 0
                            y: box.y + Math.round(box.height / 2)
                            width: editor.width
                            height: 1
                            visible: view.lineOf(editor.cursorPosition) !== view.lineOf(modelData.start)
                            color: Theme.pageLine
                        }
                    }

                    // Tables: a drawn grid over the reserved (collapsed) source.
                    // The highlighter makes the table's first line tall enough to
                    // hold the whole grid; this Column fills exactly that space.
                    // The caret landing anywhere inside the table reveals the raw
                    // source (this hides) so it can be edited.
                    Repeater {
                        model: view.livePreview ? view.tableWidgets : []

                        delegate: Column {
                            id: tbl
                            required property var modelData

                            readonly property rect box: view.boxAt(modelData.start)
                            readonly property var table: view.parseTable(modelData.start, modelData.len)
                            readonly property real colWidth: table.cols > 0 ? width / table.cols : width
                            readonly property bool caretInside:
                                view.lineOf(editor.cursorPosition) >= view.lineOf(modelData.start)
                                && view.lineOf(editor.cursorPosition) <= view.lineOf(modelData.start + modelData.len)

                            x: box.x
                            y: box.y
                            width: editor.width
                            visible: !caretInside && table.rows.length > 0
                            spacing: 0

                            Repeater {
                                model: tbl.table.rows

                                delegate: Row {
                                    id: gridRow
                                    required property var modelData
                                    required property int index

                                    readonly property bool header: index === 0
                                    spacing: 0

                                    Repeater {
                                        model: gridRow.modelData

                                        delegate: Rectangle {
                                            id: cell
                                            required property var modelData

                                            width: tbl.colWidth
                                            height: view.tableRowHeight
                                            color: gridRow.header ? Theme.codeBg : "transparent"
                                            border.color: Theme.pageLine
                                            border.width: 1

                                            Text {
                                                anchors.fill: parent
                                                anchors.leftMargin: 8
                                                anchors.rightMargin: 8
                                                verticalAlignment: Text.AlignVCenter
                                                elide: Text.ElideRight
                                                text: cell.modelData
                                                color: Theme.text
                                                font.family: Theme.body
                                                font.pixelSize: view.fontSize
                                                font.weight: gridRow.header ? Font.Bold : Font.Normal
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Inline images: the picture drawn over the collapsed
                    // `![alt](src)`, loaded from the note's folder. Its measured
                    // height is reported back so the highlighter reserves that
                    // space; the raw source shows on the caret's line for editing.
                    Repeater {
                        model: view.livePreview ? view.imageWidgets : []

                        delegate: Item {
                            id: imageItem
                            required property var modelData

                            readonly property rect box: view.boxAt(modelData.start)

                            x: box.x
                            y: box.y
                            width: editor.width
                            height: pic.status === Image.Ready ? pic.height : 0
                            visible: view.lineOf(editor.cursorPosition) !== view.lineOf(modelData.start)

                            Image {
                                id: pic
                                source: view.imageUrl(imageItem.modelData.text)
                                // Scale down to the sheet width; never upscale past
                                // the image's own size.
                                readonly property real fit: implicitWidth > editor.width && implicitWidth > 0
                                                            ? editor.width / implicitWidth : 1
                                width: implicitWidth * fit
                                height: implicitHeight * fit
                                fillMode: Image.PreserveAspectFit
                                asynchronous: true
                                cache: true

                                onStatusChanged: {
                                    if (status === Image.Ready)
                                        view.reportImageHeight(imageItem.modelData.start, Math.ceil(height) + 8)
                                    else if (status === Image.Error)
                                        view.reportImageHeight(imageItem.modelData.start, Math.round(view.fontSize * 1.6))
                                }
                            }

                            // A src that will not load: show it as a dim marker
                            // rather than a blank gap.
                            Text {
                                visible: pic.status === Image.Error
                                text: "⚠ " + imageItem.modelData.text
                                color: Theme.textSoft
                                font.family: Theme.ui
                                font.pixelSize: Theme.px(12)
                            }
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

    // The clipboard bridge (C++), for pasting an image.
    ClipboardImage { id: clipboardImage }

    // The third way in: pick a file. ⌘⇧I, and the picker copies the chosen image
    // into the note's folder and links it at the caret.
    Shortcut {
        sequence: "Ctrl+Shift+I"
        enabled: view.hasNote
        onActivated: imageFileDialog.open()
    }

    FileDialog {
        id: imageFileDialog
        title: "Add image"
        nameFilters: ["Images (*.png *.jpg *.jpeg *.gif *.webp *.svg *.bmp)"]
        onAccepted: view.insertImage(NoteEditor.import_image(view.localPath(selectedFile)))
    }
}
