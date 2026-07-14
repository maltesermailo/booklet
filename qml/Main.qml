import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import booklet

ApplicationWindow {
    id: root
    visible: true
    width: 1280
    height: 820
    title: "Folio"
    color: Theme.bg

    // Point this at your vault. For a first run the bundled sample vault
    // works: pass an absolute path, e.g. via an environment variable read
    // in Rust, or hardcode while developing.
    readonly property string vaultPath: Qt.application.arguments.length > 1
        ? Qt.application.arguments[1]
        : "vault"

    Component.onCompleted: {
        Library.open_vault(vaultPath)
        NoteEditor.set_vault(vaultPath)
        Backlinks.set_vault(vaultPath)
    }

    Shortcut {
        sequence: "Ctrl+T"
        onActivated: Theme.mode = Theme.mode === "night" ? "atlas" : "night"
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        TreePane {
            Layout.preferredWidth: 260
            Layout.fillHeight: true
        }
        Rectangle { width: 1; Layout.fillHeight: true; color: Theme.sidebarLine }
        EditorView {
            Layout.fillWidth: true
            Layout.fillHeight: true
        }
        Rectangle { width: 1; Layout.fillHeight: true; color: Theme.pageLine }
        Marginalia {
            Layout.preferredWidth: 280
            Layout.fillHeight: true
        }
    }
}
