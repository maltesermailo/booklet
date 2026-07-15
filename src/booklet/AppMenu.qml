import QtQuick
import QtQuick.Controls
import booklet

// A context menu in the app's own language. The stock Basic menu is a grey box
// with square corners — it belongs to no theme at all, which shows badly against
// the night palette. This is a rounded card that fades in, with rows that light
// up like every other row in the app.
//
// Put `AppMenuItem`s in it; a plain MenuItem would keep the stock look.
Menu {
    id: menu

    padding: Theme.gap(5)

    background: Rectangle {
        implicitWidth: Theme.row(190)
        color: Theme.panel
        border.color: Theme.pageLine
        border.width: 1
        radius: Theme.radiusCard
    }

    // A menu should arrive, not appear.
    enter: Transition {
        NumberAnimation {
            property: "opacity"
            from: 0
            to: 1
            duration: Theme.quick
            easing.type: Theme.easing
        }
        NumberAnimation {
            property: "scale"
            from: 0.96
            to: 1
            duration: Theme.quick
            easing.type: Theme.easing
        }
    }
    exit: Transition {
        NumberAnimation {
            property: "opacity"
            from: 1
            to: 0
            duration: Theme.quick
            easing.type: Theme.easing
        }
    }
}
