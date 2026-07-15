import QtQuick
import QtQuick.Shapes
import booklet

// A stroked icon from the reference: SVG path data drawn on a 24×24 grid.
// Scaling the shape scales the stroke with it, which is exactly what the
// reference's "15px icons at 1.8 stroke on a 24 viewBox" works out to.
Item {
    id: icon

    property string path: ""
    property color stroke: "white"
    property int size: Theme.px(15)

    implicitWidth: size
    implicitHeight: size

    Shape {
        width: 24
        height: 24
        preferredRendererType: Shape.CurveRenderer
        transform: Scale { xScale: icon.size / 24; yScale: icon.size / 24 }

        ShapePath {
            strokeColor: icon.stroke
            strokeWidth: 1.8
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.path }
        }
    }
}
