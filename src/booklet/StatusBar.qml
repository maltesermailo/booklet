import QtQuick
import booklet

// A slim footer: which vault you are in, and how much is in it. The saved/unsaved
// indicator joins this in 5e, once the editor reports it.
Rectangle {
    id: bar

    height: Theme.row(22)
    color: Theme.sidebar

    property string vaultName: ""
    property int noteCount: 0
    property bool hasNote: false
    property bool unsaved: false

    function reload() {
        var vaults = JSON.parse(Library.vaults())
        var active = vaults.find(function (vault) { return vault.active })
        bar.vaultName = active ? active.name : ""
        bar.noteCount = JSON.parse(Library.notes()).length
    }

    Component.onCompleted: reload()

    Connections {
        target: Library
        function onTree_changed() { bar.reload() }
    }
    Connections {
        target: NoteEditor
        function onNote_opened(id, title) {
            bar.hasNote = id !== ""
            bar.unsaved = NoteEditor.is_unsaved()
        }
        function onSave_state_changed(unsaved) { bar.unsaved = unsaved }
    }

    Rectangle {
        anchors.top: parent.top
        width: parent.width
        height: 1
        color: Theme.sidebarLine
    }

    Text {
        anchors.verticalCenter: parent.verticalCenter
        anchors.left: parent.left
        anchors.leftMargin: 14
        text: bar.vaultName === ""
              ? "No vault"
              : bar.vaultName + " · " + bar.noteCount + (bar.noteCount === 1 ? " note" : " notes")
        color: Theme.textSoft
        font.family: Theme.ui
        font.pixelSize: Theme.px(11)
    }

    // Saving is debounced and silent, so the state has to be visible somewhere.
    Row {
        anchors.verticalCenter: parent.verticalCenter
        anchors.right: parent.right
        anchors.rightMargin: 14
        spacing: 5
        visible: bar.hasNote

        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 6
            height: 6
            radius: 3
            color: bar.unsaved ? Theme.ember : "transparent"
            border.color: bar.unsaved ? Theme.ember : Theme.textDim
            border.width: 1
        }
        Text {
            text: bar.unsaved ? "unsaved" : "saved"
            color: bar.unsaved ? Theme.text : Theme.textSoft
            font.family: Theme.ui
            font.pixelSize: Theme.px(11)
        }
    }
}
