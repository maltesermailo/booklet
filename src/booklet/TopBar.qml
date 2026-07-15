import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import booklet

// The reference's topbar. One vault is read at a time; the menu switches it and
// manages the list. Breadcrumb, ⌘K hint and sync pill land in 5b.
Rectangle {
    id: bar

    height: 38
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

    // Same 24×24 grid as the reference's icons; the divider says which side.
    readonly property string sidebarIcon: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M9 4v16"
    readonly property string marginaliaIcon: "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M15 4v16"

    function reload() {
        bar.vaults = JSON.parse(Library.vaults())
        var active = bar.vaults.find(function (vault) { return vault.active })
        bar.activeName = active ? active.name : "No vault"
    }

    // FolderDialog hands back a file:// url; the engine wants a plain path.
    function urlToPath(url) {
        return decodeURIComponent(url.toString().replace(/^file:\/\//, ""))
    }

    Component.onCompleted: reload()

    Connections {
        target: Library
        function onTree_changed() { bar.reload() }
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
            font.pixelSize: 17
        }

        Rectangle {
            id: vaultButton
            anchors.verticalCenter: parent.verticalCenter
            width: vaultLabel.width + 22
            height: 24
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
                    font.pixelSize: 13
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "▾"
                    color: Theme.textDim
                    font.pixelSize: 9
                }
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: vaultMenu.popup(vaultButton, 0, vaultButton.height + 2)
            }
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
                        font.pixelSize: 13
                    }
                    Text {
                        readonly property bool last: index === bar.crumbs.length - 1
                        text: modelData
                        color: last ? Theme.textBright : Theme.textSoft
                        font.family: Theme.ui
                        font.pixelSize: 13
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

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: kbdHint.implicitWidth + 12
            height: 17
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
                font.pixelSize: 11
            }
        }

        // Sync pill. Inert until the sync engine (M2) reports status: accent dot
        // = synced, dim = offline, pulsing = syncing.
        Row {
            anchors.verticalCenter: parent.verticalCenter
            spacing: 5

            HoverHandler { id: syncHover }

            ToolTip.visible: syncHover.hovered
            ToolTip.text: "Sync status — no sync server is configured yet"
            ToolTip.delay: 400

            Rectangle {
                anchors.verticalCenter: parent.verticalCenter
                width: 7
                height: 7
                radius: 3.5
                color: Theme.textDim
            }
            Text {
                text: "offline"
                color: Theme.textSoft
                font.family: Theme.ui
                font.pixelSize: 12
            }
        }
    }

    Menu {
        id: vaultMenu

        Instantiator {
            model: bar.vaults
            delegate: MenuItem {
                required property var modelData
                text: (modelData.active ? "● " : "    ") + modelData.name
                onTriggered: Library.set_active(modelData.id)
            }
            onObjectAdded: (index, object) => vaultMenu.insertItem(index, object)
            onObjectRemoved: (index, object) => vaultMenu.removeItem(object)
        }

        MenuSeparator {}

        MenuItem {
            text: "Add vault…"
            onTriggered: vaultPicker.open()
        }
        MenuItem {
            text: bar.vaults.length > 0 ? "Remove " + bar.activeName : "Remove vault"
            enabled: bar.vaults.length > 0
            // Removes it from the library only; the files on disk are untouched.
            onTriggered: Library.remove_vault(Library.active_vault())
        }
    }

    FolderDialog {
        id: vaultPicker
        title: "Choose a vault folder"
        onAccepted: Library.add_vault(bar.urlToPath(selectedFolder))
    }
}
