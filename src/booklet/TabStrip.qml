import QtQuick
import QtQuick.Controls
import booklet

// Open notes as tabs. Tabs are UI state — the vault knows nothing about them.
// Opening a note replaces the tab you are on, as Obsidian does; ⌘T (or +) makes
// the next one land in a tab of its own.
Rectangle {
    id: strip

    height: 33
    color: Theme.sidebar

    property var tabs: []
    property int current: -1
    property bool openInNewTab: false

    signal newTabRequested()

    function noteOpened(id, title) {
        // An empty id means the editor was closed, not that a note opened.
        if (id === "")
            return

        for (var i = 0; i < strip.tabs.length; i++) {
            if (strip.tabs[i].id === id) {
                strip.current = i
                strip.openInNewTab = false
                return
            }
        }

        var next = strip.tabs.slice()
        if (strip.openInNewTab || strip.current < 0 || next.length === 0) {
            next.push({ id: id, title: title })
            strip.current = next.length - 1
        } else {
            next[strip.current] = { id: id, title: title }
        }
        strip.tabs = next
        strip.openInNewTab = false
    }

    function closeTab(index) {
        var next = strip.tabs.slice()
        next.splice(index, 1)
        strip.tabs = next

        if (next.length === 0) {
            // Nothing left to show, so clear the page too.
            strip.current = -1
            NoteEditor.close()
            return
        }
        if (index < strip.current) {
            strip.current -= 1
        } else if (index === strip.current) {
            strip.current = Math.min(index, next.length - 1)
            NoteEditor.open(next[strip.current].id)
        }
    }

    function closeCurrent() {
        if (strip.current >= 0)
            strip.closeTab(strip.current)
    }

    Connections {
        target: NoteEditor
        function onNote_opened(id, title) { strip.noteOpened(id, title) }
    }

    Rectangle {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 1
        color: Theme.sidebarLine
    }

    Row {
        anchors.left: parent.left
        anchors.leftMargin: 8
        anchors.bottom: parent.bottom
        spacing: 2

        Repeater {
            model: strip.tabs

            delegate: Rectangle {
                id: tab
                required property var modelData
                required property int index

                readonly property bool active: index === strip.current

                width: Math.min(180, tabLabel.implicitWidth + 46)
                height: 28
                topLeftRadius: 6
                topRightRadius: 6
                color: tab.active ? Theme.bg : "transparent"
                border.color: tab.active ? Theme.sidebarLine : "transparent"
                border.width: 1

                // Erase the bottom border so the active tab fuses with the page.
                Rectangle {
                    visible: tab.active
                    anchors.bottom: parent.bottom
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: parent.width - 2
                    height: 1
                    color: Theme.bg
                }

                Text {
                    id: tabLabel
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.leftMargin: 12
                    width: Math.min(implicitWidth, tab.width - 46)
                    text: tab.modelData.title
                    color: tab.active ? Theme.textBright : Theme.textSoft
                    font.family: Theme.ui
                    font.pixelSize: 13
                    elide: Text.ElideRight
                }

                Rectangle {
                    id: closeButton
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.right: parent.right
                    anchors.rightMargin: 7
                    width: 16
                    height: 16
                    radius: 3
                    color: closeHover.hovered ? Theme.activePill : "transparent"

                    HoverHandler { id: closeHover }

                    ToolTip.visible: closeHover.hovered
                    ToolTip.text: "Close tab (⌘W)"
                    ToolTip.delay: 400

                    Text {
                        anchors.centerIn: parent
                        text: "×"
                        color: closeHover.hovered ? Theme.textBright : Theme.textDim
                        font.pixelSize: 13
                    }

                    MouseArea {
                        anchors.fill: parent
                        onClicked: strip.closeTab(tab.index)
                    }
                }

                MouseArea {
                    anchors.fill: parent
                    anchors.rightMargin: 22 // leave the × its own hit area
                    acceptedButtons: Qt.LeftButton | Qt.MiddleButton
                    cursorShape: Qt.PointingHandCursor
                    onClicked: (mouse) => {
                        if (mouse.button === Qt.MiddleButton) {
                            strip.closeTab(tab.index)
                            return
                        }
                        strip.current = tab.index
                        NoteEditor.open(tab.modelData.id)
                    }
                }
            }
        }

        Rectangle {
            anchors.bottom: parent.bottom
            anchors.bottomMargin: 3
            width: 24
            height: 24
            radius: 5
            color: addHover.hovered ? Theme.activePill : "transparent"

            HoverHandler { id: addHover }

            ToolTip.visible: addHover.hovered
            ToolTip.text: "New tab (⌘T)"
            ToolTip.delay: 400

            Text {
                anchors.centerIn: parent
                text: "+"
                color: addHover.hovered ? Theme.textBright : Theme.textSoft
                font.pixelSize: 15
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: strip.newTabRequested()
            }
        }
    }
}
