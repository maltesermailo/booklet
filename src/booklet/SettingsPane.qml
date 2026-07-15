import QtQuick
import QtQuick.Controls
import booklet

// One settings category: its name, a line saying what it is for, and whatever
// the category puts below that. Scrolls, because the vault list has no ceiling.
Item {
    id: pane

    property string title: ""
    property string blurb: ""
    default property alias content: body.data

    ScrollView {
        anchors.fill: parent
        clip: true
        contentWidth: availableWidth

        Column {
            id: column
            x: 24
            y: 20
            width: pane.width - 48
            spacing: 12

            Text {
                text: pane.title
                color: Theme.textBright
                font.family: Theme.display
                font.pixelSize: Theme.px(20)
            }

            Text {
                width: parent.width
                visible: pane.blurb !== ""
                text: pane.blurb
                color: Theme.textSoft
                font.family: Theme.ui
                font.pixelSize: Theme.px(12)
                wrapMode: Text.Wrap
            }

            Column {
                id: body
                width: parent.width
                spacing: 10
            }
        }
    }
}
