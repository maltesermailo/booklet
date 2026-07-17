import QtQuick
import QtQuick.Controls
import booklet

// Trades server credentials for a device token (saved to its own 0600 file).
// Same modal motion as Settings.
Popup {
    id: dialog
    modal: true
    focus: true
    anchors.centerIn: Overlay.overlay
    width: 440
    padding: 1
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

    property string error: ""

    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
            NumberAnimation { property: "scale"; from: 0.97; to: 1; duration: Theme.gentle; easing.type: Theme.easing }
        }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Theme.quick; easing.type: Theme.easing }
    }

    onOpened: dialog.error = ""

    background: Rectangle {
        color: Theme.bg
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    Connections {
        target: Sync
        function onSigned_in(ok) {
            if (ok)
                dialog.close()
            else
                dialog.error = "Could not sign in. Check the server and your details."
        }
    }

    // A labelled input; the fields all share it.
    component Field: Column {
        property alias text: input.text
        property alias placeholder: input.placeholderText
        property alias echo: input.echoMode
        property string title
        width: parent ? parent.width : 0
        spacing: 4

        Text {
            text: parent.title
            color: Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(11)
        }
        TextField {
            id: input
            width: parent.width
            color: Theme.text
            font.family: Theme.ui
            font.pixelSize: Theme.px(13)
            selectionColor: Theme.brassDeep
            placeholderTextColor: Theme.textDim
            leftPadding: 8
            rightPadding: 8
            background: Rectangle {
                color: Theme.editBg
                border.color: input.activeFocus ? Theme.brass : Theme.pageLine
                border.width: 1
                radius: Theme.radiusSmall
            }
        }
    }

    // An Item (not a padded Column) so the anchored inner column is sized to the
    // dialog's width minus its margins — fields that fill `parent.width` then fit
    // instead of overrunning the frame.
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
                text: "Connect to a sync server"
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: Theme.px(19)
            }

            Field { id: serverField; title: "Server URL"; placeholder: "https://notes.example" }
            Field { id: handleField; title: "Account" }
            Field { id: passwordField; title: "Password"; echo: TextInput.Password }
            Field { id: deviceField; title: "Device name"; text: "This device" }

            Text {
                visible: dialog.error !== ""
                text: dialog.error
                color: Theme.ember
                width: parent.width
                wrapMode: Text.WordWrap
                font.family: Theme.ui
                font.pixelSize: Theme.px(11)
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
                        label: "Sign in"
                        filled: true
                        onClicked: {
                            dialog.error = ""
                            Sync.sign_in(JSON.stringify({
                                server: serverField.text.trim(),
                                handle: handleField.text.trim(),
                                password: passwordField.text,
                                device: deviceField.text.trim()
                            }))
                        }
                    }
                }
            }
        }
    }
}
