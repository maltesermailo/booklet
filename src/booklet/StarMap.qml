import QtQuick
import QtQuick.Controls
import booklet

// Sightlines: the open note as the centre star, everything it links to (and
// everything linking back) spread around it.
//
// The layout is a plain radial spread — angle by index, a little radius jitter —
// not a force simulation: with a handful of dots a simulation buys nothing but
// drift, and a map that settles differently each time you open a note is a map
// you cannot learn.
Rectangle {
    id: map
    color: Theme.codeBg
    border.color: Theme.pageLine
    border.width: 1
    radius: Theme.radiusCard
    height: Theme.row(150)

    property string title: ""
    // [{ title, id, kind }] — kind is "in" (references this note), "out" (this
    // note points at it) or "unresolved" (points at a note not written yet).
    property var stars: []

    // Past this the map turns into a smudge; the cards below carry the rest.
    readonly property int maxStars: 10

    readonly property real centreX: width / 2
    readonly property real centreY: height / 2
    // Leaves room for the labels, which sit outside their dot.
    readonly property real spread: Math.min(width, height) / 2 - 30

    // The visible stars, each given a place. Recomputed as one array so the
    // canvas and the dots can never disagree about where a star is.
    readonly property var placed: {
        var shown = stars.slice(0, maxStars)
        var out = []

        for (var i = 0; i < shown.length; i++) {
            var angle = -Math.PI / 2 + i * 2 * Math.PI / shown.length
            // Deterministic jitter: a map that reshuffles on every repaint
            // would be unreadable. Rings the dots between 88% and 100% out.
            var jitter = 1 - 0.12 * ((i * 3) % 4) / 3

            out.push({
                "title": shown[i].title,
                "id": shown[i].id,
                "kind": shown[i].kind,
                "x": map.centreX + Math.cos(angle) * map.spread * jitter,
                "y": map.centreY + Math.sin(angle) * map.spread * jitter
            })
        }

        return out
    }

    function colorFor(kind) {
        if (kind === "out")
            return Theme.ember
        if (kind === "unresolved")
            return Theme.textDim
        return Theme.textSoft
    }

    function follow(star) {
        // An unresolved star has no note behind it yet, so opening it by title
        // writes one — the same bargain ⌘+click makes in the editor.
        if (star.id === "")
            NoteEditor.open_by_title(star.title)
        else
            NoteEditor.open(star.id)
    }

    Canvas {
        id: sightlines
        anchors.fill: parent
        // Only the lines: the dots are real Items below, so they hit-test
        // themselves instead of this having to work out what was clicked.
        onPaint: {
            var context = getContext("2d")
            context.reset()
            context.lineWidth = 0.7
            context.strokeStyle = Theme.textDim

            for (var i = 0; i < map.placed.length; i++) {
                var star = map.placed[i]
                var unresolved = star.kind === "unresolved"

                context.globalAlpha = unresolved ? 0.35 : 0.55
                context.setLineDash(unresolved ? [3, 3] : [])

                context.beginPath()
                context.moveTo(map.centreX, map.centreY)
                context.lineTo(star.x, star.y)
                context.stroke()
            }
        }
    }

    onPlacedChanged: sightlines.requestPaint()
    // The palette moves under us when the theme changes.
    Connections {
        target: Theme
        function onModeChanged() { sightlines.requestPaint() }
    }

    Repeater {
        model: map.placed

        delegate: Item {
            id: star
            required property var modelData

            readonly property bool unresolved: modelData.kind === "unresolved"
            readonly property color tint: map.colorFor(modelData.kind)

            x: modelData.x
            y: modelData.y

            // An unresolved note is drawn hollow: an outline of something that
            // is not there.
            Rectangle {
                id: dot
                anchors.centerIn: parent
                width: 5
                height: 5
                radius: 2.5
                color: star.unresolved ? "transparent" : star.tint
                border.color: star.tint
                border.width: star.unresolved ? 1 : 0
            }

            Text {
                id: label
                text: star.modelData.title.toUpperCase()
                // Uniformly dim, per the reference: the dot carries the kind,
                // and tinted labels would make the map shout.
                color: Theme.textDim
                font.family: Theme.mono
                font.pixelSize: Theme.px(7)
                font.letterSpacing: 0.6 * Theme.uiScale
                elide: Text.ElideRight
                width: Math.min(implicitWidth, 62)

                // Labels sit away from the centre, so a dot's name never lands
                // on top of the star it belongs to.
                x: (star.x < map.centreX ? -width - 5 : 5)
                y: (star.y < map.centreY ? -height - 3 : 3)
            }

            MouseArea {
                anchors.centerIn: parent
                width: 18
                height: 18 // a 5px dot is not a click target
                cursorShape: Qt.PointingHandCursor
                hoverEnabled: true
                onClicked: map.follow(star.modelData)
                onEntered: dot.scale = 1.4
                onExited: dot.scale = 1
                ToolTip.visible: containsMouse
                ToolTip.text: star.unresolved ? star.modelData.title + " — not written yet"
                                              : star.modelData.title
            }
        }
    }

    // The open note, last so it sits over the sightlines meeting it.
    Item {
        x: map.centreX
        y: map.centreY
        visible: map.title !== ""

        Rectangle {
            anchors.centerIn: parent
            width: 13
            height: 13
            radius: 6.5
            color: "transparent"
            border.color: Theme.ember
            border.width: 1
        }
        Rectangle {
            anchors.centerIn: parent
            width: 5
            height: 5
            radius: 2.5
            color: Theme.textBright
        }
        Text {
            text: map.title.toUpperCase()
            color: Theme.textSoft // the centre reads a shade above the rest
            font.family: Theme.mono
            font.pixelSize: Theme.px(7)
            font.letterSpacing: 0.6 * Theme.uiScale
            elide: Text.ElideRight
            width: Math.min(implicitWidth, 80)
            x: -width / 2
            y: 10
        }
    }

    Text {
        anchors.centerIn: parent
        visible: map.title === ""
        text: "No note open"
        color: Theme.textDim
        font.family: Theme.mono
        font.pixelSize: Theme.px(8)
    }
}
