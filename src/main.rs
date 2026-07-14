use qtbridge::QApp;

mod library;
mod links;
mod note;

fn main() {
    // NOTE (qtbridge 0.2 beta): additional QML component files must be made
    // available to the engine through the Qt resource system. The macro for
    // this is `qtbridge::include_bytes_qml!` — verify its exact signature
    // against the `color_palette` example in qt/qtbridge-rust, which uses
    // multiple QML files. If it differs, the fallback is to inline the
    // components into Main.qml for a first run.
    qtbridge::include_bytes_qml!("../qml/Theme.qml");
    qtbridge::include_bytes_qml!("../qml/qmldir");
    qtbridge::include_bytes_qml!("../qml/TreePane.qml");
    qtbridge::include_bytes_qml!("../qml/EditorView.qml");
    qtbridge::include_bytes_qml!("../qml/Marginalia.qml");

    QApp::new()
        .register::<library::Library>()
        .register::<note::NoteEditor>()
        .register::<links::Backlinks>()
        .load_qml(include_bytes!("../qml/Main.qml"))
        .run();
}
