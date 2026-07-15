import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import booklet

ApplicationWindow {
    id: root
    visible: true
    width: 1280
    height: 820
    title: root.noteTitle === "" ? "Booklet" : root.noteTitle + " — Booklet"
    color: Theme.bg

    // The shelf is a full-window browse mode; the reading layout hides while
    // it is up.
    property bool shelfOpen: false
    property bool sidebarVisible: true
    property bool marginaliaVisible: true
    property string noteTitle: ""

    // Where you have been. Following a link takes you away, so there has to be
    // a way back. `navigating` keeps back/forward from recording their own
    // moves as new history.
    property var history: []
    property int historyAt: -1
    property bool navigating: false
    readonly property bool canGoBack: historyAt > 0
    readonly property bool canGoForward: historyAt >= 0 && historyAt < history.length - 1

    function recordVisit(id) {
        if (root.navigating || id === "")
            return
        if (root.historyAt >= 0 && root.history[root.historyAt] === id)
            return

        // Going somewhere new drops the forward trail, as a browser does.
        var trail = root.history.slice(0, root.historyAt + 1)
        trail.push(id)
        root.history = trail
        root.historyAt = trail.length - 1
    }

    function goTo(index) {
        root.navigating = true
        root.historyAt = index
        NoteEditor.open(root.history[index])
        root.navigating = false
    }

    function goBack() {
        if (root.canGoBack)
            root.goTo(root.historyAt - 1)
    }

    function goForward() {
        if (root.canGoForward)
            root.goTo(root.historyAt + 1)
    }

    Component.onCompleted: {
        // Load the persisted vault list; a path argument seeds/adds one vault
        // (handy for the bundled sample: `cargo run -- "$(pwd)/vault"`).
        Library.load()
        if (Qt.application.arguments.length > 1)
            Library.add_vault(Qt.application.arguments[1])

        // Links resolve inside a note's own vault; both need the vault list.
        NoteEditor.load()
        Backlinks.load()
    }

    Connections {
        target: NoteEditor
        function onNote_opened(id, title) {
            root.noteTitle = title
            root.recordVisit(id)
        }
    }

    // StandardKey rather than a written-out "Ctrl++": where the punctuation sits
    // differs per layout, and Qt knows where. The engine clamps the result.
    Shortcut {
        sequences: [StandardKey.ZoomIn]
        onActivated: Library.set_editor_font_size(Library.editor_font_size() + 1)
    }
    Shortcut {
        sequences: [StandardKey.ZoomOut]
        onActivated: Library.set_editor_font_size(Library.editor_font_size() - 1)
    }

    // Arrows, not brackets: ⌘[ / ⌘] need ⌥5 / ⌥6 on a German layout.
    Shortcut {
        sequence: "Ctrl+Alt+Left"
        onActivated: root.goBack()
    }
    Shortcut {
        sequence: "Ctrl+Alt+Right"
        onActivated: root.goForward()
    }

    // Qt maps Ctrl to Cmd on macOS, so these are Cmd+K / Cmd+L / … there.
    // The theme toggle has no shortcut by design — it lives in Settings (5g).
    Shortcut {
        sequence: "Ctrl+K"
        onActivated: quickSwitcher.openSwitcher()
    }
    Shortcut {
        sequence: "Ctrl+L"
        onActivated: root.shelfOpen = !root.shelfOpen
    }
    Shortcut {
        sequence: "Ctrl+T"
        onActivated: root.newTab()
    }
    Shortcut {
        sequence: "Ctrl+W"
        onActivated: tabStrip.closeCurrent()
    }
    Shortcut {
        sequence: "Ctrl+N"
        onActivated: {
            root.sidebarVisible = true // the name field lives in the tree
            treePane.startCreate("note")
        }
    }
    // Not ⌘\ : the backslash needs ⌥⇧7 on a German layout, so Qt can never
    // match it. Letters keep these reachable on every keyboard, and ⌘B stays
    // free for bold text later.
    Shortcut {
        sequence: "Ctrl+Alt+S"
        onActivated: root.sidebarVisible = !root.sidebarVisible
    }
    Shortcut {
        sequence: "Ctrl+Alt+M"
        onActivated: root.marginaliaVisible = !root.marginaliaVisible
    }
    Shortcut {
        sequence: "Escape"
        enabled: root.shelfOpen
        onActivated: root.shelfOpen = false
    }

    // A new tab has nothing to show until you pick a note, so open the switcher
    // and let the note it opens land in a tab of its own.
    function newTab() {
        tabStrip.openInNewTab = true
        quickSwitcher.openSwitcher()
    }

    QuickSwitcher { id: quickSwitcher }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        visible: !root.shelfOpen

        TopBar {
            Layout.fillWidth: true

            // A hidden panel's own toggle goes with it, so the way back lives
            // in the topbar.
            canGoBack: root.canGoBack
            canGoForward: root.canGoForward
            onGoBack: root.goBack()
            onGoForward: root.goForward()

            sidebarHidden: !root.sidebarVisible
            marginaliaHidden: !root.marginaliaVisible
            onShowSidebar: root.sidebarVisible = true
            onShowMarginalia: root.marginaliaVisible = true
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            handle: Rectangle {
                implicitWidth: 1
                color: SplitHandle.pressed || SplitHandle.hovered
                       ? Theme.brass : Theme.sidebarLine
            }

            TreePane {
                id: treePane
                visible: root.sidebarVisible
                SplitView.preferredWidth: 230
                SplitView.minimumWidth: 170

                onHideRequested: root.sidebarVisible = false
                onSearchRequested: quickSwitcher.openSwitcher()
            }

            ColumnLayout {
                SplitView.fillWidth: true
                SplitView.minimumWidth: 320
                spacing: 0

                TabStrip {
                    id: tabStrip
                    Layout.fillWidth: true
                    onNewTabRequested: root.newTab()
                }
                EditorView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                }
            }

            Marginalia {
                visible: root.marginaliaVisible
                SplitView.preferredWidth: 220
                SplitView.minimumWidth: 170
            }
        }

        Notice { Layout.fillWidth: true }
        StatusBar { Layout.fillWidth: true }
    }

    ShelfView {
        id: shelfView
        anchors.fill: parent
        visible: root.shelfOpen

        // Picking a book opens it in the tree and drops back to reading.
        onBookPicked: (id) => {
            Library.reveal(id)
            root.shelfOpen = false
        }
    }
}
