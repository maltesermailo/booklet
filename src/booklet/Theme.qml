pragma Singleton
import QtQuick

// Two themes sharing one token vocabulary. Components only ever read the
// flat aliases at the bottom, so adding a theme means adding one palette
// object and one branch in `p`.
//   "night" — warm near-black reading room, brass foil, ember links
//   "atlas" — Celestial Atlas: void blue-black, starlight ink, gilt
//             accents, comet-teal links
QtObject {
    id: theme

    // Toggle at runtime with Ctrl+T (see Main.qml).
    property string mode: "night"

    readonly property QtObject night: QtObject {
        readonly property color bg:          "#171512"
        readonly property color sidebar:     "#161514"
        readonly property color sidebarLine: "#26241F"
        readonly property color page:        "#221F1A"
        readonly property color pageLine:    "#35302A"
        readonly property color panel:       "#191714"
        readonly property color codeBg:      "#10150F"
        readonly property color editBg:      "#1C1916"
        readonly property color text:        "#E6DDC9"
        readonly property color textBright:  "#F0E8D2"
        readonly property color textSoft:    "#9C9280"
        readonly property color textDim:     "#615B4E"
        readonly property color accent:      "#C2A45C"  // brass
        readonly property color accentDeep:  "#A8842C"
        readonly property color link:        "#C4695A"  // ember
        readonly property color activePill:  "#2A2721"
    }

    readonly property QtObject atlas: QtObject {
        readonly property color bg:          "#090D15"  // void
        readonly property color sidebar:     "#0C111C"
        readonly property color sidebarLine: "#1C2635"
        readonly property color page:        "#0E1420"  // chart
        readonly property color pageLine:    "#1C2635"
        readonly property color panel:       "#0C111C"
        readonly property color codeBg:      "#070B12"
        readonly property color editBg:      "#101826"
        readonly property color text:        "#D6DEEB"  // starlight
        readonly property color textBright:  "#EDF2FA"
        readonly property color textSoft:    "#8A96AC"
        readonly property color textDim:     "#55607A"
        readonly property color accent:      "#DFC078"  // gilt
        readonly property color accentDeep:  "#B89A54"
        readonly property color link:        "#74BCC4"  // comet
        readonly property color activePill:  "#16202E"
    }

    readonly property QtObject p: mode === "atlas" ? atlas : night

    readonly property color bg:          p.bg
    readonly property color sidebar:     p.sidebar
    readonly property color sidebarLine: p.sidebarLine
    readonly property color page:        p.page
    readonly property color pageLine:    p.pageLine
    readonly property color panel:       p.panel
    readonly property color codeBg:      p.codeBg
    readonly property color editBg:      p.editBg
    readonly property color text:        p.text
    readonly property color textBright:  p.textBright
    readonly property color textSoft:    p.textSoft
    readonly property color textDim:     p.textDim
    readonly property color brass:       p.accent      // kept under old names so
    readonly property color brassDeep:   p.accentDeep  // existing components need
    readonly property color ember:       p.link        // no changes
    readonly property color activePill:  p.activePill

    // Binding colors are per-book data (booklet.json), not theme.
    readonly property var bindings: ["#7C3128", "#2F3E5C", "#3C5240",
                                     "#A8842C", "#55364F", "#4A5560"]

    // Bundled OFL fonts (see COPYRIGHT.md), compiled into qrc by build.rs.
    // Loading them registers the families by name, so components keep using the
    // plain family strings below. Spectral carries the body text, so its italic
    // and bold faces are bundled too rather than letting Qt synthesize them.
    readonly property FontLoader displayFace:        FontLoader { source: "qrc:/fonts/EBGaramond.ttf" }
    readonly property FontLoader uiFace:             FontLoader { source: "qrc:/fonts/AlegreyaSans-Regular.ttf" }
    readonly property FontLoader uiMediumFace:       FontLoader { source: "qrc:/fonts/AlegreyaSans-Medium.ttf" }
    readonly property FontLoader bodyFace:           FontLoader { source: "qrc:/fonts/Spectral-Regular.ttf" }
    readonly property FontLoader bodyItalicFace:     FontLoader { source: "qrc:/fonts/Spectral-Italic.ttf" }
    readonly property FontLoader bodyBoldFace:       FontLoader { source: "qrc:/fonts/Spectral-Bold.ttf" }
    readonly property FontLoader bodyBoldItalicFace: FontLoader { source: "qrc:/fonts/Spectral-BoldItalic.ttf" }
    readonly property FontLoader monoFace:           FontLoader { source: "qrc:/fonts/JetBrainsMono.ttf" }

    readonly property string display: "EB Garamond"
    readonly property string ui:      "Alegreya Sans"
    readonly property string body:    "Spectral"
    readonly property string mono:    "JetBrains Mono"
}
