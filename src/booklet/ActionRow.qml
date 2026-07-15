import QtQuick
import booklet

// A row in the picker's actions card: what you can do, said plainly, with the
// button that does it on the right.
Rectangle {
    id: row

    property string title: ""
    property string blurb: ""
    property string action: ""
    property bool filled: false
    // The card's rows are divided from each other, but not from its cap.
    property bool first: false

    signal triggered()

    implicitHeight: Math.max(text.implicitHeight + Theme.gap(26), Theme.row(52))
    color: "transparent"

    Rectangle {
        anchors.top: parent.top
        width: parent.width
        height: 1
        visible: !row.first
        color: Theme.pageLine
    }

    Column {
        id: text
        anchors.left: parent.left
        anchors.leftMargin: 16
        anchors.right: button.left
        anchors.rightMargin: 14
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Text {
            text: row.title
            color: Theme.textBright
            font.family: Theme.ui
            font.pixelSize: Theme.px(14)
            font.weight: Font.Medium
            elide: Text.ElideRight
            width: parent.width
        }
        Text {
            text: row.blurb
            color: Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(12)
            wrapMode: Text.Wrap
            width: parent.width
        }
    }

    TextButton {
        id: button
        anchors.right: parent.right
        anchors.rightMargin: 16
        anchors.verticalCenter: parent.verticalCenter
        label: row.action
        filled: row.filled
        enabled: row.enabled
        opacity: row.enabled ? 1 : 0.4
        onClicked: row.triggered()
    }
}
