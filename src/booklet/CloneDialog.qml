import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import booklet

// Clones a server vault into an empty local folder: pick the vault, pick the
// folder, and the sync engine pulls it down.
Popup {
    id: dialog
    modal: true
    focus: true
    anchors.centerIn: Overlay.overlay
    width: 460
    padding: 1
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property var vaults: []
    property int selected: -1
    property string folder: ""
    // Reactive sign-in state: `Sync.is_signed_in()` is a plain call with no change
    // signal, so binding to it directly goes stale (it was read once, before you
    // signed in, and never again). Kept in step via the signals below instead.
    property bool signedIn: false

    // A vault was cloned and made active; Main uses this to close the picker.
    signal cloned()

    function openClone() {
        dialog.vaults = []
        dialog.selected = -1
        dialog.folder = ""
        dialog.signedIn = Sync.is_signed_in()
        if (dialog.signedIn)
            Sync.request_vaults()
        dialog.open()
    }

    function localPath(folderUrl) {
        return decodeURIComponent(folderUrl.toString().replace(/^file:\/\//, ""))
    }

    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
            NumberAnimation { property: "scale"; from: 0.97; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
        }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Theme.quick; easing.type: Theme.easing }
    }

    background: Rectangle {
        color: Theme.bg
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    Connections {
        target: Sync
        function onVaults_ready(payload) { dialog.vaults = JSON.parse(payload) }
        // Signing in while the dialog is open (or just before it) flips the body
        // from the "sign in first" hint to the vault list, and pulls the list.
        function onSigned_in(ok) {
            dialog.signedIn = ok
            if (ok)
                Sync.request_vaults()
        }
        function onStatus_changed(payload) { dialog.signedIn = JSON.parse(payload).signed_in }
    }

    FolderDialog {
        id: folderDialog
        title: "Choose an empty folder"
        onAccepted: dialog.folder = dialog.localPath(selectedFolder)
    }

    contentItem: Column {
        spacing: 12
        padding: 22

        Text {
            text: "Clone a server vault"
            color: Theme.textBright
            font.family: Theme.display
            font.pixelSize: Theme.px(19)
        }

        Text {
            visible: !dialog.signedIn
            text: "Sign in to a server first."
            color: Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(12)
        }

        ListView {
            width: parent.width - 44
            height: 160
            visible: dialog.signedIn
            clip: true
            model: dialog.vaults
            spacing: 2

            delegate: Rectangle {
                required property int index
                required property var modelData
                width: ListView.view.width
                height: Theme.row(34)
                radius: Theme.radiusSmall
                color: index === dialog.selected ? Theme.activePill
                       : (cloneRowHover.hovered ? Theme.activePill : "transparent")
                Behavior on color { ColorAnimation { duration: Theme.quick } }

                HoverHandler { id: cloneRowHover }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    x: 10
                    text: modelData.name
                    color: Theme.textBright
                    font.family: Theme.ui
                    font.pixelSize: Theme.px(13)
                }

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: dialog.selected = index
                }
            }
        }

        Row {
            width: parent.width - 44
            spacing: 10
            visible: dialog.signedIn

            TextButton {
                label: "Choose folder…"
                onClicked: folderDialog.open()
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - 130
                elide: Text.ElideMiddle
                text: dialog.folder === "" ? "No folder chosen" : dialog.folder
                color: dialog.folder === "" ? Theme.textDim : Theme.textSoft
                font.family: Theme.mono
                font.pixelSize: Theme.px(11)
            }
        }

        Row {
            anchors.right: parent.right
            spacing: 8

            TextButton {
                label: "Cancel"
                onClicked: dialog.close()
            }
            TextButton {
                label: "Clone"
                filled: true
                enabled: dialog.selected >= 0 && dialog.folder !== ""
                onClicked: {
                    Sync.clone_vault(JSON.stringify({
                        vault_id: dialog.vaults[dialog.selected].id,
                        path: dialog.folder
                    }))
                    Library.add_vault(dialog.folder)
                    Library.set_active(dialog.folder)
                    dialog.cloned()
                    dialog.close()
                }
            }
        }
    }
}
