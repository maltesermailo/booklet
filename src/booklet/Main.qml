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

    // The shelf is a full-window mode; the reading layout hides while it is up.
    // Settings is a modal over it, so it needs no flag of its own.
    property bool shelfOpen: false
    // The welcome screen. Shown when there is no vault to reopen; a bare start
    // otherwise goes straight back to where you were reading.
    property bool pickerOpen: false
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

    // The vault sync is scoped to, tracked so a switch retargets the engine.
    property string activeVault: ""

    Component.onCompleted: {
        // Start the sync thread before anything can send it a command.
        Sync.start()

        // Load the persisted vault list; a path argument seeds/adds one vault
        // (handy for the bundled sample: `cargo run -- "$(pwd)/vault"`).
        Library.load()
        Theme.mode = Library.theme()
        Theme.reloadChrome()
        if (Qt.application.arguments.length > 1)
            Library.add_vault(Qt.application.arguments[1])

        root.pickerOpen = Library.active_vault() === ""

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
        // Just written to disk; push it soon (debounced).
        function onSave_state_changed(unsaved) {
            if (!unsaved)
                syncAfterSave.restart()
        }
    }

    // Retarget the sync engine when the active vault actually changes (not on
    // every tree write, which `tree_changed` also fires for).
    Connections {
        target: Library
        function onTree_changed() {
            var vault = Library.active_vault()
            if (vault !== root.activeVault) {
                root.activeVault = vault
                Sync.set_active_vault(vault)
            }
        }
    }

    // Sync on a slow cadence: flush the open note first so its edits are on disk
    // and go through the normal push-409-merge path, then run a cycle.
    Timer {
        id: syncTimer
        interval: 30000
        repeat: true
        running: root.activeVault !== ""
        onTriggered: {
            NoteEditor.flush()
            Sync.sync_now()
        }
    }
    Timer {
        id: syncAfterSave
        interval: 4000
        onTriggered: Sync.sync_now()
    }

    SignInDialog { id: signInDialog }
    VersionHistory { id: versionHistory }

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
    // The theme toggle has no shortcut by design — it lives in Settings.
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
    // ⌘, is where every Mac app keeps its settings, and the comma is unshifted
    // on a German layout as well.
    Shortcut {
        sequence: "Ctrl+,"
        // `visible`, not `opened`: with an enter transition, `opened` only
        // turns true once the animation finishes, so a quick second ⌘, would
        // open it again instead of closing it.
        onActivated: settingsView.visible ? settingsView.close() : settingsView.open()
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
        // The picker closes only when there is a vault to close to; with none,
        // Escape would drop you on an empty reading pane. Settings is a Popup
        // and closes itself.
        enabled: root.shelfOpen || (root.pickerOpen && Library.active_vault() !== "")
        onActivated: {
            root.shelfOpen = false
            root.pickerOpen = false
        }
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
        visible: !root.shelfOpen && !root.pickerOpen

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
            onOpenSettings: settingsView.open()
            onOpenPicker: root.pickerOpen = true
            onOpenShelf: root.shelfOpen = true
            onSignInRequested: signInDialog.open()
            onOpenHistory: if (NoteEditor.current_id() !== "") versionHistory.openFor(NoteEditor.current_id())
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: Qt.Horizontal

            // The reference's rule between panes is 1px and stays 1px — but that
            // is a width to look at, not one to hit, and Qt takes a handle's
            // grab area from the handle itself (its own docs use 4). The right
            // handle had it worse than the left: the editor's scrollbar sits
            // hard against it and takes the presses aimed at it. The mask widens
            // the hit area alone, so the hairline is untouched.
            handle: Rectangle {
                id: splitHandle
                implicitWidth: 1
                color: SplitHandle.pressed || SplitHandle.hovered
                       ? Theme.brass : Theme.sidebarLine

                containmentMask: Item {
                    x: -4
                    width: 9
                    height: splitHandle.height
                }
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
                    id: editorView
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    onRequestHistory: (path) => versionHistory.openFor(path)
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

    // While a name is being typed in the tree, a press anywhere but the field
    // itself closes it. The field already cancels on focus-out, but only the
    // editor (a TextArea) actually takes focus on a click — every other surface
    // (tree rows, the topbar, the panes, empty space) is a MouseArea or a
    // Control that leaves the field focused, so the caret would linger. This
    // overlay, live only while a field is up, is the one place that press is
    // caught wherever it lands. A press on the field is passed through so its
    // caret can still be placed; every other press dismisses (and is consumed —
    // it closes the field rather than also acting, which keeps the behaviour the
    // same whatever lies beneath). It sits below the full-window modes and is
    // inert unless the tree is editing, which only happens in the reading view.
    MouseArea {
        id: editDismissOverlay
        anchors.fill: parent
        enabled: treePane.isEditing
        acceptedButtons: Qt.AllButtons
        onPressed: (mouse) => {
            var field = treePane.editField
            if (field) {
                var local = mapToItem(field, mouse.x, mouse.y)
                if (local.x >= 0 && local.y >= 0
                        && local.x < field.width && local.y < field.height) {
                    mouse.accepted = false
                    return
                }
            }
            treePane.cancelEdit()
        }
    }

    // One full-window mode at a time; two stacked would strand the lower one.
    onShelfOpenChanged: if (shelfOpen) pickerOpen = false
    onPickerOpenChanged: if (pickerOpen) shelfOpen = false

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

    SettingsView {
        id: settingsView
        onSignInRequested: signInDialog.open()
        onCloneRequested: cloneDialog.openClone()
    }

    CloneDialog { id: cloneDialog }

    VaultPicker {
        anchors.fill: parent
        visible: root.pickerOpen
        onDismissed: root.pickerOpen = false
        onSignInRequested: signInDialog.open()
    }
}
