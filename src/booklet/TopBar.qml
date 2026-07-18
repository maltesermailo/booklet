import QtQuick
import QtQuick.Controls
import booklet

// The reference's topbar. One vault is read at a time; the menu switches it and
// manages the list. Breadcrumb, ⌘K hint and sync pill land in 5b.
Rectangle {
    id: bar

    height: Theme.row(38)
    color: Theme.sidebar

    property var vaults: []
    property string activeName: ""
    property var crumbs: []

    // A hidden panel takes its own toolbar with it, so the only way back has to
    // live out here.
    property bool sidebarHidden: false
    property bool marginaliaHidden: false
    signal showSidebar()
    signal showMarginalia()
    signal openSettings()
    signal openPicker()
    signal openShelf()

    property bool canGoBack: false
    property bool canGoForward: false
    signal goBack()
    signal goForward()

    // Live sync status, driven by the Sync engine.
    property string syncState: "offline"
    property int syncFlagged: 0
    property bool signedIn: false
    property bool published: false
    signal signInRequested()
    signal deleteVaultRequested(string name)
    signal openHistory()

    readonly property string backIcon: "M15 5l-7 7 7 7"
    readonly property string forwardIcon: "M9 5l7 7-7 7"

    // Same 24×24 grid as the reference's icons; the divider says which side.
    readonly property string sidebarIcon: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M9 4v16"
    readonly property string marginaliaIcon: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M15 4v16"
    // A gear: the ring, plus eight teeth spoked around it.
    readonly property string settingsIcon: "M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z M12 2.6v2.6 M12 18.8v2.6 M5.4 5.4l1.9 1.9 M16.7 16.7l1.9 1.9 M2.6 12h2.6 M18.8 12h2.6 M5.4 18.6l1.9-1.9 M16.7 7.3l1.9-1.9"
    // Books on a shelf, the last one leaning. Spines are lines rather than
    // outlines: a spine wide enough to outline is ~2.5px once the 24 grid is
    // drawn at 15, and its own 1.8 stroke would fill that in solid. The lean is
    // what keeps three upright lines from reading as a bar chart.
    readonly property string shelfIcon: "M6 20V6 M10.5 20V4 M15 20l4-13"

    function reload() {
        bar.vaults = JSON.parse(Library.vaults())
        var active = bar.vaults.find(function (vault) { return vault.active })
        bar.activeName = active ? active.name : "No vault"
    }

    function refreshSync() {
        var status = JSON.parse(Sync.status())
        bar.syncState = status.state
        bar.syncFlagged = status.flagged_count
        bar.signedIn = status.signed_in
        bar.published = status.published
    }

    Component.onCompleted: {
        reload()
        refreshSync()
    }

    Connections {
        target: Library
        function onTree_changed() { bar.reload() }
    }
    Connections {
        target: Sync
        function onStatus_changed(payload) {
            var status = JSON.parse(payload)
            bar.syncState = status.state
            bar.syncFlagged = status.flagged_count
            bar.signedIn = status.signed_in
            bar.published = status.published
        }
    }
    Connections {
        target: NoteEditor
        function onNote_opened(id, title) { bar.crumbs = JSON.parse(NoteEditor.breadcrumb()) }
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.sidebarLine
    }

    Row {
        id: leftGroup
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 14
        spacing: 12

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            visible: bar.sidebarHidden
            path: bar.sidebarIcon
            tip: "Show sidebar (⌘⌥S)"
            onClicked: bar.showSidebar()
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "Booklet"
            color: Theme.brass
            font.family: Theme.display
            font.pixelSize: Theme.px(17)
        }

        // Following a link takes you away from where you were.
        Row {
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2

            IconButton {
                path: bar.backIcon
                tip: "Back (⌘⌥←)"
                enabled: bar.canGoBack
                onClicked: bar.goBack()
            }
            IconButton {
                path: bar.forwardIcon
                tip: "Forward (⌘⌥→)"
                enabled: bar.canGoForward
                onClicked: bar.goForward()
            }
        }

        Rectangle {
            id: vaultButton
            anchors.verticalCenter: parent.verticalCenter
            width: vaultLabel.width + 22
            height: Theme.row(24)
            radius: 5
            color: vaultHover.hovered ? Theme.activePill : "transparent"

            HoverHandler { id: vaultHover }

            ToolTip.visible: vaultHover.hovered
            ToolTip.text: "Switch vault, add or remove one"
            ToolTip.delay: 400

            Row {
                id: vaultLabel
                anchors.centerIn: parent
                spacing: 5

                Text {
                    text: bar.activeName
                    color: Theme.text
                    font.family: Theme.ui
                    font.pixelSize: Theme.px(13)
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "▾"
                    color: Theme.textDim
                    font.pixelSize: Theme.px(9)
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: vaultMenu.popup(vaultButton, 0, vaultButton.height + 2)
            }
        }

        // Beside the vault menu on purpose: switching vaults and browsing the
        // books inside one are the same errand a level apart, and until now the
        // second had no button at all — only ⌘L, which you had to be told about.
        // Opens only; the shelf is full-window, so this bar is gone while it is
        // up and Esc is the way back.
        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            path: bar.shelfIcon
            tip: "Shelf — every book in this vault (⌘L)"
            onClicked: bar.openShelf()
        }

        // Breadcrumb: book / sections / note, the last segment bright.
        Row {
            anchors.verticalCenter: parent.verticalCenter
            spacing: 0

            Repeater {
                model: bar.crumbs

                delegate: Row {
                    required property var modelData
                    required property int index
                    spacing: 0

                    Text {
                        text: index > 0 ? " / " : ""
                        color: Theme.textDim
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(13)
                    }
                    Text {
                        readonly property bool last: index === bar.crumbs.length - 1
                        text: modelData
                        color: last ? Theme.textBright : Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(13)
                        font.weight: last ? Font.Medium : Font.Normal
                    }
                }
            }
        }
    }

    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 14
        spacing: 12

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            visible: bar.marginaliaHidden
            path: bar.marginaliaIcon
            tip: "Show marginalia (⌘⌥M)"
            onClicked: bar.showMarginalia()
        }

        IconButton {
            anchors.verticalCenter: parent.verticalCenter
            path: bar.settingsIcon
            tip: "Settings (⌘,)"
            onClicked: bar.openSettings()
        }

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: kbdHint.implicitWidth + 12
            height: Theme.row(17)
            radius: 4
            color: "transparent"
            border.color: Theme.pageLine
            border.width: 1

            Text {
                id: kbdHint
                anchors.centerIn: parent
                text: "⌘K"
                color: Theme.textSoft
                font.family: Theme.mono
                font.pixelSize: Theme.px(11)
            }
        }

        // Sync pill — live status: brass dot = synced, pulsing = syncing, dim =
        // offline, ember = error. A flagged-merge count rides alongside. Click for
        // the sync menu. An Item so the layout Row and the click area can coexist.
        Item {
            id: syncPill
            anchors.verticalCenter: parent.verticalCenter
            implicitWidth: pillRow.implicitWidth
            implicitHeight: pillRow.implicitHeight

            readonly property color dotColor: bar.syncState === "synced" ? Theme.brass
                                             : bar.syncState === "syncing" ? Theme.brass
                                             : bar.syncState === "error" ? Theme.ember
                                             : Theme.textDim

            HoverHandler { id: syncHover }
            ToolTip.visible: syncHover.hovered
            ToolTip.text: "Sync: " + bar.syncState + (bar.syncFlagged > 0 ? " · " + bar.syncFlagged + " to review" : "")
            ToolTip.delay: 400

            Row {
                id: pillRow
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6

                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 7
                    height: 7
                    radius: 3.5
                    color: syncPill.dotColor

                    // The syncing state breathes.
                    SequentialAnimation on opacity {
                        running: bar.syncState === "syncing"
                        loops: Animation.Infinite
                        NumberAnimation { to: 0.3; duration: 600; easing.type: Theme.easing }
                        NumberAnimation { to: 1.0; duration: 600; easing.type: Theme.easing }
                    }
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: bar.syncState
                    color: Theme.textSoft
                    font.family: Theme.ui
                    font.pixelSize: Theme.px(12)
                }

                // Flagged-merge badge.
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: bar.syncFlagged > 0
                    width: flagLabel.implicitWidth + 12
                    height: 16
                    radius: 8
                    color: Theme.ember
                    Text {
                        id: flagLabel
                        anchors.centerIn: parent
                        text: "⚑ " + bar.syncFlagged
                        color: Theme.page
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(10)
                    }
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: syncMenu.popup()
            }
        }
    }

    AppMenu {
        id: syncMenu

        AppMenuItem {
            text: "Sync now"
            onTriggered: Sync.sync_now()
        }
        AppMenuItem {
            text: "Version history"
            enabled: NoteEditor.current_id() !== ""
            onTriggered: bar.openHistory()
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: Theme.pageLine }
        }
        AppMenuItem {
            text: "Publish this vault"
            // Only once, and only when signed in — republishing makes a duplicate
            // server vault.
            visible: bar.signedIn && !bar.published
            height: visible ? implicitHeight : 0
            onTriggered: Sync.publish(bar.activeName)
        }
        AppMenuItem {
            text: "Published ✓"
            enabled: false
            visible: bar.published
            height: visible ? implicitHeight : 0
        }
        AppMenuItem {
            text: "Delete server vault…"
            visible: bar.published
            height: visible ? implicitHeight : 0
            onTriggered: bar.deleteVaultRequested(bar.activeName)
        }
        MenuSeparator {
            contentItem: Rectangle { implicitHeight: 1; color: Theme.pageLine }
        }
        AppMenuItem {
            text: "Sign in…"
            visible: !bar.signedIn
            height: visible ? implicitHeight : 0
            onTriggered: bar.signInRequested()
        }
        AppMenuItem {
            text: "Sign out"
            visible: bar.signedIn
            height: visible ? implicitHeight : 0
            onTriggered: Sync.sign_out()
        }
    }

    AppMenu {
        id: vaultMenu

        Instantiator {
            model: bar.vaults
            delegate: AppMenuItem {
                required property var modelData
                text: (modelData.active ? "● " : "    ") + modelData.name
                onTriggered: Library.set_active(modelData.id)
            }
            onObjectAdded: (index, object) => vaultMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => vaultMenu.removeItem(object)
        }

        MenuSeparator {
            contentItem: Rectangle {
                implicitHeight: 1
                color: Theme.pageLine
            }
        }

        AppMenuItem {
            text: "Open another vault…"
            onTriggered: bar.openPicker()
        }
        AppMenuItem {
            text: bar.vaults.length > 0 ? "Remove " + bar.activeName : "Remove vault"
            enabled: bar.vaults.length > 0
            // Removes it from the library only; the files on disk are untouched.
            onTriggered: Library.remove_vault(Library.active_vault())
        }
    }

}
