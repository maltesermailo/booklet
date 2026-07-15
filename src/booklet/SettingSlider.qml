import QtQuick
import QtQuick.Controls
import booklet

// A slider for a whole-number setting, with its value read out beside it. The
// stock Basic slider is grey plastic; this is brass on a page line, like
// everything else. `chosen` fires while dragging, so the interface answers as
// you move it rather than after you let go.
Item {
    id: control

    property int from: 0
    property int to: 100
    property alias value: slider.value
    property string suffix: ""

    signal chosen(int value)

    implicitHeight: Theme.row(24)

    Slider {
        id: slider
        width: control.width - readout.implicitWidth - Theme.gap(14)
        anchors.verticalCenter: parent.verticalCenter
        from: control.from
        to: control.to
        stepSize: 1
        snapMode: Slider.SnapAlways
        onMoved: control.chosen(Math.round(value))

        background: Rectangle {
            x: slider.leftPadding
            y: slider.topPadding + slider.availableHeight / 2 - height / 2
            width: slider.availableWidth
            height: 3
            radius: 1.5
            color: Theme.pageLine

            Rectangle {
                width: slider.visualPosition * parent.width
                height: parent.height
                radius: 1.5
                color: Theme.brass
            }
        }

        handle: Rectangle {
            x: slider.leftPadding + slider.visualPosition * (slider.availableWidth - width)
            y: slider.topPadding + slider.availableHeight / 2 - height / 2
            width: Theme.px(14)
            height: Theme.px(14)
            radius: width / 2
            color: slider.pressed ? Theme.brassDeep : Theme.brass
            border.color: Theme.page
            border.width: 1
            scale: hover.hovered || slider.pressed ? 1.15 : 1

            HoverHandler { id: hover }

            Behavior on scale {
                NumberAnimation { duration: Theme.quick; easing.type: Theme.easing }
            }
            Behavior on color {
                ColorAnimation { duration: Theme.quick; easing.type: Theme.easing }
            }
        }
    }

    Text {
        id: readout
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        text: Math.round(slider.value) + control.suffix
        color: Theme.textSoft
        font.family: Theme.mono
        font.pixelSize: Theme.px(12)
    }
}
