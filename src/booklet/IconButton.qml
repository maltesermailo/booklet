import QtQuick
import QtQuick.Controls
import booklet

// A toolbar button from the reference: 26px hit target, radius 5, hover fills
// with --active-pill and brightens the icon.
Rectangle {
    id: button

    property string path: ""
    property string tip: ""
    signal clicked()

    width: Theme.row(26)
    height: Theme.row(26)
    radius: Theme.radiusSmall
    color: hover.hovered && button.enabled ? Theme.activePill : "transparent"
    opacity: button.enabled ? 1 : 0.35

    Behavior on color {
        ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
    }
    Behavior on opacity {
        NumberAnimation { duration: Theme.quick; easing.type: Theme.easing }
    }

    HoverHandler {
        id: hover
        enabled: button.enabled
    }

    ToolTip.visible: hover.hovered && button.tip !== ""
    ToolTip.text: button.tip
    ToolTip.delay: 400

    Icon {
        anchors.centerIn: parent
        path: button.path
        stroke: hover.hovered && button.enabled ? Theme.textBright : Theme.textSoft
    }

    MouseArea {
        anchors.fill: parent
        enabled: button.enabled
        cursorShape: Qt.PointingHandCursor
        onClicked: button.clicked()
    }
}
