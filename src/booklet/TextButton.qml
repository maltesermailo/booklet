import QtQuick
import booklet

// A text button in IconButton's idiom, for the same reason it exists: the stock
// Controls button speaks none of the reference's language, and Basic style is
// what the app runs under.
//
// `filled` is the reference's one primary button (--accent fill); everything
// else is a quiet outline that only warms on hover.
Rectangle {
    id: button

    property string label: ""
    property bool filled: false
    signal clicked()

    width: text.implicitWidth + Theme.gap(22)
    height: Theme.row(26)
    radius: Theme.radiusSmall
    color: button.filled ? (hover.hovered ? Theme.brassDeep : Theme.brass)
                         : (hover.hovered ? Theme.activePill : "transparent")
    border.color: button.filled ? "transparent" : Theme.pageLine
    border.width: button.filled ? 0 : 1

    Behavior on color {
        ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
    }

    HoverHandler { id: hover }

    Text {
        id: text
        anchors.centerIn: parent
        text: button.label
        color: button.filled ? Theme.page
                             : (hover.hovered ? Theme.textBright : Theme.textSoft)
        font.family: Theme.ui
        font.pixelSize: Theme.px(12)
    }

    MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: button.clicked()
    }
}
