pragma Singleton
import QtQuick
import booklet

// Four themes sharing one token vocabulary. Components only ever read the
// flat aliases at the bottom, so adding a theme means adding one palette
// object and one branch in `p`.
//   "night"    — warm near-black reading room, brass foil, ember links
//   "atlas"    — Celestial Atlas: void blue-black, starlight ink, gilt
//                accents, comet-teal links
//   "graphite" — near-OLED black, warm gray ink, silver accents, steel-blue
//                links (which stay scannable where silver would not)
//   "vellum"   — the only light theme: warm paper, dark brass, oxblood links
//
// Vellum is why a tint must derive from a token (`Theme.text`, `Theme.brass`)
// instead of hardcoding white or black: a 3% white lift reads as nothing on
// paper. Three themes hid that for free; this one does not.
//
// It also owns how big the chrome draws. Every size in the reference is a
// number designed against 100%, so components write `Theme.px(13)` rather than
// `13` and the whole interface scales from one setting. `px` is for type and
// fixed furniture, `gap` for the room between things — they are separate
// because "I want bigger text" and "I want it less cramped" are separate
// wishes.
QtObject {
    id: theme

    // Set from the persisted settings (see Main.qml); Settings changes them.
    property string mode: "night"
    property real uiScale: 1
    property real density: 1

    // Rounded to whole pixels: font.pixelSize is an int, and a half-pixel
    // border draws blurred.
    function px(size) {
        return Math.max(1, Math.round(size * theme.uiScale))
    }
    function gap(space) {
        return Math.max(0, Math.round(space * theme.density))
    }
    // Anything sized to hold type: a tree row, a tab, a button. It has to grow
    // with the text or the text spills out of it, and density is then the room
    // it keeps around that text.
    function row(height) {
        return Math.round(height * theme.uiScale * theme.density)
    }

    // The house motion. One duration and one easing everywhere, so hovering a
    // tree row and hovering a button feel like the same app.
    readonly property int quick: 110
    readonly property int gentle: 180
    readonly property int easing: Easing.OutCubic

    // Corner radius, one vocabulary: pills for rows and buttons, cards for
    // panels and menus.
    readonly property int radiusSmall: 5
    readonly property int radiusCard: 8

    function reloadChrome() {
        theme.uiScale = Library.ui_scale() / 100
        theme.density = Library.density() / 100
    }

    readonly property Connections chromeWatch: Connections {
        target: Library
        function onChrome_changed() { theme.reloadChrome() }
    }

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
        readonly property color codeKeyword:  "#CE7B57"
        readonly property color codeString:   "#9AAE68"
        readonly property color codeComment:  "#6B6455"
        readonly property color codeNumber:   "#D2A24C"
        readonly property color codeFunction: "#77A6A0"
        readonly property color codeType:     "#B191C0"
        readonly property color codeConstant: "#CF9A54"
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
        readonly property color codeKeyword:  "#7F9CD6"
        readonly property color codeString:   "#7FB894"
        readonly property color codeComment:  "#55607A"
        readonly property color codeNumber:   "#DFC078"
        readonly property color codeFunction: "#74BCC4"
        readonly property color codeType:     "#B49BD8"
        readonly property color codeConstant: "#D69A6A"
    }

    readonly property QtObject graphite: QtObject {
        readonly property color bg:          "#0E0E0E"  // near-OLED black
        readonly property color sidebar:     "#121212"
        readonly property color sidebarLine: "#242424"
        readonly property color page:        "#171717"
        readonly property color pageLine:    "#2A2A2A"
        readonly property color panel:       "#101010"
        readonly property color codeBg:      "#0A0A0A"
        readonly property color editBg:      "#1E1E1E"
        readonly property color text:        "#D8D8D3"  // warm gray ink
        readonly property color textBright:  "#F2F2EE"
        readonly property color textSoft:    "#9C9C97"
        readonly property color textDim:     "#5E5E5A"
        readonly property color accent:      "#C4C4BC"  // silver
        readonly property color accentDeep:  "#8F8F88"
        readonly property color link:        "#82A7C4"  // steel blue
        readonly property color activePill:  "#242424"
        readonly property color codeKeyword:  "#C08A78"
        readonly property color codeString:   "#93A886"
        readonly property color codeComment:  "#5E5E5A"
        readonly property color codeNumber:   "#C4B27C"
        readonly property color codeFunction: "#8AA7B4"
        readonly property color codeType:     "#AF9BB4"
        readonly property color codeConstant: "#BE9A76"
    }

    readonly property QtObject vellum: QtObject {
        readonly property color bg:          "#EAE3D3"  // warm paper
        readonly property color sidebar:     "#E2DAC7"
        readonly property color sidebarLine: "#CDC2A8"
        readonly property color page:        "#F7F2E6"  // the sheet, lighter than bg
        readonly property color pageLine:    "#D8CDB2"
        readonly property color panel:       "#E7E0CF"
        readonly property color codeBg:      "#EAE2CD"
        readonly property color editBg:      "#EFE8D7"
        readonly property color text:        "#2E2820"  // ink
        readonly property color textBright:  "#1E1913"
        readonly property color textSoft:    "#6E6350"
        readonly property color textDim:     "#9C8F76"
        readonly property color accent:      "#8A6D28"  // dark brass for light ground
        readonly property color accentDeep:  "#6F571F"
        readonly property color link:        "#8C3A2F"  // oxblood
        readonly property color activePill:  "#DCD2BB"
        readonly property color codeKeyword:  "#9A4420"
        readonly property color codeString:   "#5A6E2E"
        readonly property color codeComment:  "#9C8F76"
        readonly property color codeNumber:   "#8A5A1E"
        readonly property color codeFunction: "#2E6A6A"
        readonly property color codeType:     "#6A4A8A"
        readonly property color codeConstant: "#8A5A28"
    }

    readonly property QtObject p: mode === "atlas" ? atlas
                                : mode === "graphite" ? graphite
                                : mode === "vellum" ? vellum
                                : night

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

    // Fenced-code syntax colours, one per semantic token class from render.rs.
    readonly property color codeKeyword:  p.codeKeyword
    readonly property color codeString:   p.codeString
    readonly property color codeComment:  p.codeComment
    readonly property color codeNumber:   p.codeNumber
    readonly property color codeFunction: p.codeFunction
    readonly property color codeType:     p.codeType
    readonly property color codeConstant: p.codeConstant

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
