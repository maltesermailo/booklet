import QtQuick
import QtQuick.Controls
import booklet

// Confirms deleting the active vault from the sync server. The server keeps the
// data as a backup and every local file stays on disk — this only unbinds and
// removes the server copy. Same modal motion as SignInDialog.
Popup {
    id: dialog
    modal: true
    focus: true
    anchors.centerIn: Overlay.overlay
    width: 440
    padding: 1
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property string vaultName: ""

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

    contentItem: Item {
        implicitHeight: form.implicitHeight + 44

        Column {
            id: form
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 22
            spacing: 14

            Text {
                width: parent.width
                text: "Delete this vault from the server?"
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: Theme.px(19)
            }

            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: "“" + dialog.vaultName + "” will be removed from the sync server and stop syncing. "
                    + "The server keeps a backup, and every note stays on this device — nothing on disk is deleted. "
                    + "You can publish it again later."
                color: Theme.textSoft
                font.family: Theme.ui
                font.pixelSize: Theme.px(12)
            }

            Item {
                width: parent.width
                height: buttons.height

                Row {
                    id: buttons
                    anchors.right: parent.right
                    spacing: 8

                    TextButton {
                        label: "Cancel"
                        onClicked: dialog.close()
                    }
                    TextButton {
                        label: "Delete from server"
                        onClicked: {
                            Sync.delete_vault()
                            dialog.close()
                        }
                    }
                }
            }
        }
    }
}
