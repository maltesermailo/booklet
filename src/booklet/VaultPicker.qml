import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Shapes
import booklet

// The welcome screen: shown when there is no vault to reopen, and reachable any
// time from the vault menu. A centred 520px column on --sidebar, per the
// reference — mark, wordmark, version, one primary button, the vaults you were
// last in, what else you can do, and a language row.
//
// Strings are English though the reference's picker is written in German: the
// rest of the app is English, and real translation (qsTr + Qt translations, and
// finding out whether qtbridge even exposes QTranslator) is its own milestone.
// The language selector below is built and inert, as the reference shows it.
Rectangle {
    id: picker
    color: Theme.bg

    // Closing is only possible once there is somewhere to close *to*.
    signal dismissed()
    // Opens the sign-in / clone dialogs (owned by Main).
    signal signInRequested()
    signal cloneRequested()

    property var recents: []
    property bool signedIn: false

    function reload() {
        picker.recents = JSON.parse(Library.recent_vaults())
        picker.signedIn = Sync.is_signed_in()
    }

    onVisibleChanged: if (visible) reload()

    Connections {
        target: Sync
        function onSigned_in(ok) { picker.signedIn = ok }
        function onStatus_changed(payload) { picker.signedIn = JSON.parse(payload).signed_in }
    }

    Connections {
        target: Library
        function onTree_changed() { if (picker.visible) picker.reload() }
    }

    // A folder URL is not a path; the percent-escapes have to come back out or
    // a vault called "Systems Engineering" arrives as "Systems%20Engineering".
    function localPath(folderUrl) {
        return decodeURIComponent(folderUrl.toString().replace(/^file:\/\//, ""))
    }

    function openVault(id) {
        Library.set_active(id)
        picker.dismissed()
    }

    // "vor 2 Std." in the reference; English here, and the same shape.
    function when(millis) {
        if (millis <= 0)
            return "never opened"

        var minutes = Math.floor((Date.now() - millis) / 60000)
        if (minutes < 1)
            return "just now"
        if (minutes < 60)
            return minutes + (minutes === 1 ? " min ago" : " mins ago")

        var hours = Math.floor(minutes / 60)
        if (hours < 24)
            return hours + (hours === 1 ? " hour ago" : " hours ago")

        var days = Math.floor(hours / 24)
        if (days === 1)
            return "yesterday"
        if (days < 30)
            return days + " days ago"

        return new Date(millis).toLocaleDateString(Qt.locale(), "d MMM yyyy")
    }

    // Home is where the paths in this list mostly live; showing "~" keeps them
    // readable at 10px mono.
    function shorten(path) {
        var home = Library.default_vault_path().replace(/\/Documents\/Booklet$/, "")
        return home !== "" && path.indexOf(home) === 0 ? "~" + path.substring(home.length) : path
    }

    FolderDialog {
        id: openDialog
        title: "Choose a folder with markdown files"
        onAccepted: {
            var path = picker.localPath(selectedFolder)
            Library.add_vault(path)
            picker.openVault(path)
        }
    }

    FolderDialog {
        id: createDialog
        title: "Choose where the new vault goes"
        onAccepted: {
            // The chosen folder is where the vault is made, not the vault: a
            // vault has to be its own folder, and create_vault refuses one that
            // already holds anything.
            Library.create_vault(picker.localPath(selectedFolder) + "/Booklet")
            picker.dismissed()
        }
    }

    ScrollView {
        anchors.fill: parent
        clip: true

        Item {
            width: picker.width
            height: Math.max(picker.height, column.implicitHeight + 60)

            Column {
                id: column
                width: 520
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.verticalCenter: parent.verticalCenter
                spacing: 0

                // The mark: a closed book, gilt bands on the spine.
                Shape {
                    width: 56
                    height: 56
                    anchors.horizontalCenter: parent.horizontalCenter
                    preferredRendererType: Shape.CurveRenderer

                    // The pages behind, offset — what makes it read as a book
                    // rather than a card.
                    ShapePath {
                        fillColor: Qt.rgba(Theme.brass.r, Theme.brass.g, Theme.brass.b, 0.16)
                        strokeColor: "transparent"
                        PathSvg { path: "M17 8h30a3 3 0 0 1 3 3v34a3 3 0 0 1-3 3H17a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z" }
                    }
                    ShapePath {
                        fillColor: Theme.page
                        strokeColor: Theme.brass
                        strokeWidth: 1.6
                        PathSvg { path: "M14 6h24a3 3 0 0 1 3 3v34a3 3 0 0 1-3 3H14a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3z" }
                    }
                    // The spine.
                    ShapePath {
                        fillColor: Theme.brass
                        strokeColor: "transparent"
                        PathSvg { path: "M14 6h4v40h-4a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3z" }
                    }
                    ShapePath {
                        strokeColor: Theme.brassDeep
                        strokeWidth: 1.4
                        fillColor: "transparent"
                        PathSvg { path: "M24 14h12" }
                    }
                    ShapePath {
                        strokeColor: Theme.brassDeep
                        strokeWidth: 1.4
                        fillColor: "transparent"
                        PathSvg { path: "M24 19h9" }
                    }
                }

                Item { width: 1; height: 12 }

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Booklet"
                    color: Theme.textBright
                    font.family: Theme.display
                    font.pixelSize: Theme.px(26)
                    font.weight: Font.Medium
                }

                Item { width: 1; height: 3 }

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Version " + Library.version()
                    color: Theme.textSoft
                    font.family: Theme.ui
                    font.pixelSize: Theme.px(12)
                }

                Item { width: 1; height: 16 }

                // The one filled button in the whole app.
                TextButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    label: "Quick start"
                    filled: true
                    height: Theme.row(32)
                    width: Theme.row(132)
                    onClicked: {
                        Library.create_vault(Library.default_vault_path())
                        picker.dismissed()
                    }
                }

                Item { width: 1; height: 18 }

                // --- Recently opened -----------------------------------------
                Rectangle {
                    width: parent.width
                    height: recentColumn.implicitHeight
                    visible: picker.recents.length > 0
                    color: Theme.panel
                    border.color: Theme.pageLine
                    border.width: 1
                    radius: Theme.radiusCard
                    clip: true

                    Column {
                        id: recentColumn
                        width: parent.width

                        Text {
                            text: "RECENTLY OPENED"
                            color: Theme.textSoft
                            font.family: Theme.ui
                            font.pixelSize: Theme.px(11)
                            font.letterSpacing: 2 * Theme.uiScale
                            leftPadding: 16
                            topPadding: 12
                            bottomPadding: 6
                        }

                        Repeater {
                            model: picker.recents

                            delegate: Rectangle {
                                id: vaultRow
                                required property var modelData

                                width: recentColumn.width - Theme.gap(10)
                                x: Theme.gap(5)
                                height: Theme.row(46)
                                radius: Theme.radiusSmall
                                color: rowHover.hovered ? Theme.activePill : "transparent"

                                Behavior on color {
                                    ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
                                }

                                HoverHandler { id: rowHover }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: picker.openVault(vaultRow.modelData.id)
                                }

                                Rectangle {
                                    id: dot
                                    anchors.left: parent.left
                                    anchors.leftMargin: 16
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 8
                                    height: 8
                                    radius: 4
                                    color: vaultRow.modelData.color
                                }

                                Column {
                                    anchors.left: dot.right
                                    anchors.leftMargin: 11
                                    anchors.right: whenText.left
                                    anchors.rightMargin: 10
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 1

                                    Text {
                                        text: vaultRow.modelData.name
                                        color: Theme.textBright
                                        font.family: Theme.display
                                        font.pixelSize: Theme.px(15)
                                        elide: Text.ElideRight
                                        width: parent.width
                                    }
                                    Text {
                                        text: picker.shorten(vaultRow.modelData.id)
                                        color: Theme.textDim
                                        font.family: Theme.mono
                                        font.pixelSize: Theme.px(11)
                                        elide: Text.ElideMiddle
                                        width: parent.width
                                    }
                                }

                                Text {
                                    id: whenText
                                    anchors.right: forget.left
                                    anchors.rightMargin: 8
                                    anchors.verticalCenter: parent.verticalCenter
                                    text: picker.when(vaultRow.modelData.last_opened)
                                    color: Theme.textSoft
                                    font.family: Theme.ui
                                    font.pixelSize: Theme.px(11)
                                }

                                // Removes the vault from this list and nothing
                                // else — the notes stay exactly where they are.
                                Rectangle {
                                    id: forget
                                    anchors.right: parent.right
                                    anchors.rightMargin: 12
                                    anchors.verticalCenter: parent.verticalCenter
                                    width: 18
                                    height: 18
                                    radius: 3
                                    color: forgetHover.hovered ? Theme.sidebar : "transparent"

                                    HoverHandler { id: forgetHover }

                                    ToolTip.visible: forgetHover.hovered
                                    ToolTip.text: "Remove from this list"
                                    ToolTip.delay: 400

                                    Text {
                                        anchors.centerIn: parent
                                        text: "×"
                                        color: forgetHover.hovered ? Theme.textBright : Theme.textDim
                                        font.family: Theme.ui
                                        font.pixelSize: Theme.px(13)
                                    }

                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: Library.remove_vault(vaultRow.modelData.id)
                                    }
                                }
                            }
                        }

                        Item { width: 1; height: 6 }
                    }
                }

                Item { width: 1; height: 18 }

                // --- What else you can do ------------------------------------
                Rectangle {
                    width: parent.width
                    height: actions.implicitHeight
                    color: Theme.panel
                    border.color: Theme.pageLine
                    border.width: 1
                    radius: Theme.radiusCard
                    clip: true

                    Column {
                        id: actions
                        width: parent.width

                        ActionRow {
                            width: parent.width
                            first: true
                            title: "Create a new vault."
                            blurb: "Make a new Booklet vault inside a folder."
                            action: "Create"
                            filled: true
                            onTriggered: createDialog.open()
                        }
                        ActionRow {
                            width: parent.width
                            title: "Open a folder as a vault."
                            blurb: "Choose an existing folder of markdown files."
                            action: "Open"
                            onTriggered: openDialog.open()
                        }
                        ActionRow {
                            width: parent.width
                            // Once signed in, the useful next step with no vault is
                            // to clone one you've published from another device.
                            title: picker.signedIn ? "Clone a vault from your server"
                                                   : "Connect to a sync server"
                            blurb: picker.signedIn ? "Pull down a vault you published from another device."
                                                   : "Sign in to publish this vault or clone one from the server."
                            action: picker.signedIn ? "Clone" : "Sign in"
                            onTriggered: picker.signedIn ? picker.cloneRequested() : picker.signInRequested()
                        }
                    }
                }

                Item { width: 1; height: 16 }

                // --- Language ------------------------------------------------
                Row {
                    width: parent.width
                    spacing: 10

                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 22
                        height: 22
                        radius: 4
                        color: "transparent"
                        border.color: Theme.pageLine
                        border.width: 1

                        ToolTip.visible: helpHover.hovered
                        ToolTip.text: "Booklet keeps notes as plain markdown files on disk."
                        HoverHandler { id: helpHover }

                        Text {
                            anchors.centerIn: parent
                            text: "?"
                            color: Theme.textSoft
                            font.family: Theme.ui
                            font.pixelSize: Theme.px(12)
                        }
                    }

                    // Built, and inert: the app speaks English only until
                    // translation is a milestone of its own.
                    ComboBox {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width - 32
                        height: 28
                        model: ["English"]
                        enabled: false
                        opacity: 0.6
                    }
                }
            }
        }
    }
}
