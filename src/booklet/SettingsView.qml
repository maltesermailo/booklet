import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import booklet

// Settings, as a modal over the reading layout: categories down the left, the
// chosen one filling the right. The reference draws no settings screen, so the
// vocabulary is borrowed from the parts that exist — the tree's sidebar for the
// category list, the picker's cards for the panes.
Popup {
    id: settings

    modal: true
    focus: true
    // 1px of room for the frame's border. The rail and the panes fill the
    // content item, and at 0 the rail painted straight over the border and the
    // rounded corners on its side — the frame stopped where the sidebar began.
    padding: 1
    // Big enough for the vault list to breathe, never bigger than the window.
    width: Math.min(760, Overlay.overlay ? Overlay.overlay.width - 60 : 760)
    height: Math.min(520, Overlay.overlay ? Overlay.overlay.height - 60 : 520)
    anchors.centerIn: Overlay.overlay
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

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

    property var vaults: []
    // The user's vaults on the sync server (VaultSummary: id, name, seq), for the
    // server-vault manager in the Sync pane. Pulled whenever signed in.
    property var serverVaults: []
    property int category: 0
    property bool signedIn: false
    property bool published: false

    // Opening the sign-in / clone dialogs, which live in Main.
    signal signInRequested()
    signal cloneRequested()
    // `id` is the server vault id, "" for the active vault.
    signal deleteVaultRequested(string name, string id)

    readonly property var categories: ["Vaults", "Appearance", "Editor", "Sync", "About"]

    function activeVaultName() {
        var active = settings.vaults.find(function (vault) { return vault.active })
        return active ? active.name : "Vault"
    }

    function reload() {
        settings.vaults = JSON.parse(Library.vaults())
        settings.signedIn = Sync.is_signed_in()
        settings.published = Sync.is_published()
        sizeSlider.value = Library.editor_font_size()
        scaleSlider.value = Library.ui_scale()
        densitySlider.value = Library.density()

        settings.serverVaults = []
        if (settings.signedIn)
            Sync.request_vaults() // answered by onVaults_ready
    }

    onOpened: reload()

    Connections {
        target: Sync
        function onSigned_in(ok) {
            settings.signedIn = ok
            if (ok)
                Sync.request_vaults()
            else
                settings.serverVaults = []
        }
        function onStatus_changed(payload) {
            var status = JSON.parse(payload)
            settings.signedIn = status.signed_in
            settings.published = status.published
        }
        // The server-vault list, and its refresh after a delete.
        function onVaults_ready(payload) { settings.serverVaults = JSON.parse(payload) }
    }

    Connections {
        target: Library
        function onTree_changed() {
            if (settings.visible)
                settings.vaults = JSON.parse(Library.vaults())
        }
    }

    // FolderDialog hands back a URL; Rust wants a path. Percent-escapes are the
    // part that matters — a vault called "Systems Engineering" arrives as
    // "Systems%20Engineering" if this is skipped.
    function localPath(folderUrl) {
        return decodeURIComponent(folderUrl.toString().replace(/^file:\/\//, ""))
    }

    FolderDialog {
        id: folderDialog
        title: "Choose a vault folder"
        onAccepted: Library.add_vault(settings.localPath(selectedFolder))
    }

    background: Rectangle {
        color: Theme.bg
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    // Escape and a click outside already close this, but neither is visible.
    // Same × as the tab strip and the picker's rows.
    Rectangle {
        id: closeButton
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 10
        z: 1 // over the pane, which fills the modal
        width: 22
        height: 22
        radius: 4
        color: closeHover.hovered ? Theme.activePill : "transparent"

        HoverHandler { id: closeHover }

        ToolTip.visible: closeHover.hovered
        ToolTip.text: "Close (Esc)"
        ToolTip.delay: 400

        Text {
            anchors.centerIn: parent
            text: "×"
            color: closeHover.hovered ? Theme.textBright : Theme.textDim
            font.family: Theme.ui
            font.pixelSize: Theme.px(16)
        }

        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: settings.close()
        }
    }

    Row {
        anchors.fill: parent
        spacing: 0

        // --- Categories --------------------------------------------------
        Rectangle {
            id: rail
            width: 168
            height: parent.height
            color: Theme.sidebar
            // Round where the rail meets the frame, square where it meets the
            // pane, so the border reads as one line around the whole modal. One
            // less than the frame's radius: that is the room its border leaves.
            topLeftRadius: Theme.radiusCard - 1
            bottomLeftRadius: Theme.radiusCard - 1
            // Only the right edge, as a divider against the pane.
            Rectangle {
                anchors.right: parent.right
                width: 1
                height: parent.height
                color: Theme.sidebarLine
            }

            Column {
                width: parent.width
                spacing: 2

                Text {
                    text: "Settings"
                    color: Theme.textBright
                    font.family: Theme.display
                    font.pixelSize: Theme.px(19)
                    leftPadding: 16
                    topPadding: 16
                    bottomPadding: 10
                }

                Repeater {
                    model: settings.categories

                    delegate: Rectangle {
                        id: tab
                        required property string modelData
                        required property int index

                        readonly property bool on: settings.category === index

                        width: rail.width - 16
                        x: 8
                        height: Theme.row(30)
                        radius: Theme.radiusSmall
                        color: tab.on || tabHover.hovered ? Theme.activePill : "transparent"

                        Behavior on color {
                            ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
                        }

                        HoverHandler { id: tabHover }

                        Text {
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            text: tab.modelData
                            color: tab.on ? Theme.textBright : Theme.textSoft
                            font.family: Theme.ui
                            font.pixelSize: Theme.px(13)
                            font.weight: tab.on ? Font.Medium : Font.Normal
                        }

                        MouseArea {
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            onClicked: settings.category = tab.index
                        }
                    }
                }
            }
        }

        // --- The chosen category -----------------------------------------
        Item {
            width: parent.width - rail.width
            height: parent.height

            StackLayout {
                anchors.fill: parent
                currentIndex: settings.category

                // Vaults ---------------------------------------------------
                SettingsPane {
                    title: "Vaults"
                    blurb: "One vault is open at a time. Forgetting a vault drops "
                         + "it from this list and leaves every note on disk."

                    Repeater {
                        model: settings.vaults

                        delegate: Rectangle {
                            id: vaultRow
                            required property var modelData

                            width: parent.width
                            height: Theme.row(46)
                            color: modelData.active ? Theme.activePill : Theme.panel
                            border.color: modelData.active ? Theme.brass : Theme.pageLine
                            border.width: 1
                            radius: Theme.radiusCard

                            Rectangle {
                                id: vaultDot
                                anchors.left: parent.left
                                anchors.leftMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                width: 8
                                height: 8
                                radius: 4
                                color: vaultRow.modelData.color
                            }

                            Column {
                                anchors.left: vaultDot.right
                                anchors.leftMargin: 10
                                anchors.right: vaultActions.left
                                anchors.rightMargin: 10
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1

                                Text {
                                    text: vaultRow.modelData.name
                                    color: Theme.textBright
                                    font.family: Theme.display
                                    font.pixelSize: Theme.px(14)
                                    elide: Text.ElideRight
                                    width: parent.width
                                }
                                Text {
                                    text: vaultRow.modelData.id
                                    color: Theme.textDim
                                    font.family: Theme.mono
                                    font.pixelSize: Theme.px(10)
                                    elide: Text.ElideMiddle
                                    width: parent.width
                                }
                            }

                            Row {
                                id: vaultActions
                                anchors.right: parent.right
                                anchors.rightMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 8

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: vaultRow.modelData.active
                                    text: "READING"
                                    color: Theme.brass
                                    font.family: Theme.ui
                                    font.pixelSize: Theme.px(10)
                                    font.letterSpacing: 1 * Theme.uiScale
                                }
                                TextButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: !vaultRow.modelData.active
                                    label: "Open"
                                    onClicked: Library.set_active(vaultRow.modelData.id)
                                }
                                TextButton {
                                    anchors.verticalCenter: parent.verticalCenter
                                    label: "Forget"
                                    onClicked: Library.remove_vault(vaultRow.modelData.id)
                                }
                            }
                        }
                    }

                    Text {
                        visible: settings.vaults.length === 0
                        text: "No vaults yet."
                        color: Theme.textDim
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(13)
                    }

                    TextButton {
                        label: "Add vault…"
                        onClicked: folderDialog.open()
                    }
                }

                // Appearance -----------------------------------------------
                SettingsPane {
                    title: "Appearance"
                    blurb: "The theme, and how large and how roomy the interface "
                         + "draws. All three are remembered."

                    // Two columns: four swatches in a row would be 790px and the
                    // pane is 592 before its padding.
                    Grid {
                        columns: 2
                        spacing: 10

                        Repeater {
                            // Four themes, named as Theme.qml names them.
                            model: [{ "id": "night", "label": "Night",
                                      "blurb": "Warm near-black reading room" },
                                    { "id": "atlas", "label": "Celestial Atlas",
                                      "blurb": "Void blue-black, comet links" },
                                    { "id": "graphite", "label": "Graphite",
                                      "blurb": "Near-OLED black, silver accents" },
                                    { "id": "vellum", "label": "Vellum",
                                      "blurb": "Light: warm paper, oxblood links" }]

                            delegate: Rectangle {
                                id: swatch
                                required property var modelData

                                readonly property bool on: Theme.mode === modelData.id

                                width: 190
                                height: 62
                                color: Theme.panel
                                border.color: swatch.on ? Theme.brass : Theme.pageLine
                                border.width: swatch.on ? 2 : 1
                                radius: Theme.radiusCard

                                Column {
                                    anchors.left: parent.left
                                    anchors.leftMargin: 11
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 3

                                    Text {
                                        text: swatch.modelData.label
                                        color: Theme.textBright
                                        font.family: Theme.display
                                        font.pixelSize: Theme.px(14)
                                    }
                                    Text {
                                        text: swatch.modelData.blurb
                                        color: Theme.textDim
                                        font.family: Theme.ui
                                        font.pixelSize: Theme.px(10)
                                    }
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: {
                                        Theme.mode = swatch.modelData.id
                                        Library.set_theme(swatch.modelData.id)
                                    }
                                }
                            }
                        }
                    }

                    Item { width: 1; height: Theme.gap(6) }

                    Text {
                        text: "INTERFACE SIZE"
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(11)
                        font.letterSpacing: 1.5 * Theme.uiScale
                    }

                    // Scales every size in the interface at once, so the whole
                    // thing stays in proportion instead of the text outgrowing
                    // the rows it sits in.
                    SettingSlider {
                        id: scaleSlider
                        width: parent.width
                        from: 80
                        to: 160
                        suffix: "%"
                        onChosen: (value) => Library.set_ui_scale(value)
                    }

                    Text {
                        text: "DENSITY"
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(11)
                        font.letterSpacing: 1.5 * Theme.uiScale
                    }

                    // Only the room around things: the type stays where the
                    // interface size put it.
                    SettingSlider {
                        id: densitySlider
                        width: parent.width
                        from: 80
                        to: 150
                        suffix: "%"
                        onChosen: (value) => Library.set_density(value)
                    }
                }

                // Editor ---------------------------------------------------
                SettingsPane {
                    title: "Editor"
                    blurb: "Reading size, which ⌘+ and ⌘− also change while you read."

                    SettingSlider {
                        id: sizeSlider
                        width: parent.width
                        // The engine clamps to this same range; matching it here
                        // stops the slider offering a size it would refuse.
                        from: 11
                        to: 40
                        suffix: " px"
                        onChosen: (value) => Library.set_editor_font_size(value)
                    }

                    // The setting, shown as what it actually does.
                    Rectangle {
                        width: parent.width
                        height: sample.implicitHeight + 24
                        color: Theme.page
                        border.color: Theme.pageLine
                        border.width: 1
                        radius: 4

                        Text {
                            id: sample
                            anchors.left: parent.left
                            anchors.right: parent.right
                            anchors.margins: 12
                            anchors.verticalCenter: parent.verticalCenter
                            text: "The rig is wired to the board, and the log is honest."
                            color: Theme.text
                            font.family: Theme.body
                            font.pixelSize: Math.round(sizeSlider.value)
                            wrapMode: Text.Wrap
                        }
                    }
                }

                // Sync -----------------------------------------------------
                SettingsPane {
                    title: "Sync"
                    blurb: "Sync this vault against a self-hosted server. Notes stay "
                         + "plain markdown on disk; the server holds the history."

                    Rectangle {
                        width: parent.width
                        height: Theme.row(46)
                        color: Theme.panel
                        border.color: Theme.pageLine
                        border.width: 1
                        radius: Theme.radiusCard

                        Rectangle {
                            id: syncDot
                            anchors.left: parent.left
                            anchors.leftMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: 8
                            height: 8
                            radius: 4
                            color: settings.signedIn ? Theme.brass : Theme.textDim
                        }
                        Text {
                            anchors.left: syncDot.right
                            anchors.leftMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            text: settings.signedIn ? "Signed in to a sync server" : "Not signed in"
                            color: Theme.textBright
                            font.family: Theme.ui
                            font.pixelSize: Theme.px(13)
                        }
                        TextButton {
                            anchors.right: parent.right
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            label: settings.signedIn ? "Sign out" : "Sign in…"
                            onClicked: {
                                if (settings.signedIn)
                                    Sync.sign_out()
                                else
                                    settings.signInRequested()
                            }
                        }
                    }

                    Text {
                        text: "THIS VAULT"
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(11)
                        font.letterSpacing: 1.5 * Theme.uiScale
                    }
                    Row {
                        width: parent.width
                        spacing: 10

                        TextButton {
                            label: settings.published ? "Published ✓" : "Publish this vault"
                            enabled: settings.signedIn && !settings.published
                            onClicked: Sync.publish(settings.activeVaultName())
                        }
                        TextButton {
                            label: "Clone a server vault…"
                            enabled: settings.signedIn
                            onClicked: settings.cloneRequested()
                        }
                        TextButton {
                            label: "Delete server vault…"
                            visible: settings.published
                            onClicked: settings.deleteVaultRequested(settings.activeVaultName(), "")
                        }
                    }
                    Text {
                        width: parent.width
                        text: "Publishing binds this vault to a new server vault and uploads it. "
                            + "Cloning pulls an existing server vault into a new local folder."
                        color: Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(12)
                        wrapMode: Text.Wrap
                    }

                    // Everything the signed-in user has on the server, so a vault
                    // can be managed (deleted) whether or not it is cloned here.
                    Text {
                        visible: settings.signedIn
                        text: "YOUR SERVER VAULTS"
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(11)
                        font.letterSpacing: 1.5 * Theme.uiScale
                        topPadding: 6
                    }

                    Repeater {
                        model: settings.signedIn ? settings.serverVaults : []

                        delegate: Rectangle {
                            id: serverRow
                            required property var modelData

                            width: parent.width
                            height: Theme.row(42)
                            color: Theme.panel
                            border.color: Theme.pageLine
                            border.width: 1
                            radius: Theme.radiusCard

                            Text {
                                anchors.left: parent.left
                                anchors.leftMargin: 14
                                anchors.right: serverDelete.left
                                anchors.rightMargin: 10
                                anchors.verticalCenter: parent.verticalCenter
                                text: serverRow.modelData.name
                                color: Theme.textBright
                                font.family: Theme.ui
                                font.pixelSize: Theme.px(13)
                                elide: Text.ElideRight
                            }

                            TextButton {
                                id: serverDelete
                                anchors.right: parent.right
                                anchors.rightMargin: 12
                                anchors.verticalCenter: parent.verticalCenter
                                label: "Delete…"
                                onClicked: settings.deleteVaultRequested(serverRow.modelData.name, serverRow.modelData.id)
                            }
                        }
                    }

                    Text {
                        visible: settings.signedIn && settings.serverVaults.length === 0
                        text: "No vaults on the server yet — publish one above."
                        color: Theme.textDim
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(12)
                    }
                }

                // About ----------------------------------------------------
                SettingsPane {
                    title: "About"
                    blurb: "Booklet " + Library.version()

                    Text {
                        text: "CONFIGURATION"
                        color: Theme.brass
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(11)
                        font.letterSpacing: 1.5 * Theme.uiScale
                    }
                    Text {
                        width: parent.width
                        text: Library.config_path()
                        color: Theme.textDim
                        font.family: Theme.mono
                        font.pixelSize: Theme.px(11)
                        elide: Text.ElideMiddle
                    }
                    Text {
                        width: parent.width
                        text: "Plain JSON. Notes are plain markdown in the vault folders — "
                            + "nothing here is a database, and nothing here is a format only "
                            + "Booklet can read."
                        color: Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: Theme.px(12)
                        wrapMode: Text.Wrap
                    }
                }
            }
        }
    }
}
