import QtQuick
import QtQuick.Controls
import booklet

// A row in an AppMenu: rounded, and it warms into its highlight rather than
// snapping, like the tree rows and the tabs.
MenuItem {
    id: item

    implicitHeight: Theme.row(28)
    padding: Theme.gap(8)

    contentItem: Text {
        text: item.text
        color: item.enabled ? (item.highlighted ? Theme.textBright : Theme.text)
                            : Theme.textDim
        font.family: Theme.ui
        font.pixelSize: Theme.px(13)
        verticalAlignment: Text.AlignVCenter
        leftPadding: Theme.gap(6)
    }

    background: Rectangle {
        implicitWidth: Theme.row(180)
        radius: Theme.radiusSmall
        color: item.highlighted ? Theme.activePill : "transparent"

        Behavior on color {
            ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
        }
    }
}
