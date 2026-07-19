use qtbridge::QApp;

mod library;
mod links;
mod note;
mod sync;

/// The bundled OFL fonts, compiled by build.rs (see src/booklet/fonts.qrc and
/// COPYRIGHT.md). They are too large for `include_bytes_qml!`, which would turn
/// every byte into a token literal.
const FONTS_RCC: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fonts.rcc"));

extern "C" {
    /// Registers `MarkdownHighlighter` into the `booklet` QML module. It is C++
    /// because the highlighter has to attach to `TextEdit.textDocument`, which
    /// qtbridge exposes no way to reach — see CLAUDE.md and src/cpp/.
    fn booklet_register_highlighter();
    /// Registers `ClipboardImage`, which reads a pasted image off the clipboard —
    /// `QClipboard` is likewise out of qtbridge's reach.
    fn booklet_register_clipboard_image();
}

fn main() {
    // Booklet styles every control itself, and the native macOS style refuses
    // `background` customization ("the current style does not support
    // customization of this control"). Basic is the neutral, fully customizable
    // style, so pin it rather than inherit the platform default.
    std::env::set_var("QT_QUICK_CONTROLS_STYLE", "Basic");

    if !qtbridge::qresource::register_bytes(FONTS_RCC) {
        eprintln!("booklet: could not register the bundled fonts; falling back to system fonts");
    }

    // Ship the QML module through the Qt resource system. Each file is
    // registered under qrc:/qt/qml/booklet/, so `import booklet` finds the
    // module's qmldir on the qrc:/qt/qml import path (see the color_palette
    // example in qt/qtbridge-rust). Paths are relative to this file (src/), so
    // the QML lives in src/booklet/ to keep them free of `..`.
    qtbridge::include_bytes_qml!("booklet/qmldir", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/Main.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/Theme.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/TreePane.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/EditorView.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/Marginalia.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/StarMap.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/QuickSwitcher.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/SettingsView.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/SettingsPane.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/SettingSlider.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/VaultPicker.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/ActionRow.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/BindingDialog.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/TextButton.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/AppMenu.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/AppMenuItem.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/TopBar.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/StatusBar.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/SignInDialog.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/VersionHistory.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/CloneDialog.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/DeleteVaultDialog.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/Notice.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/TabStrip.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/IconButton.qml", "qt/qml");
    qtbridge::include_bytes_qml!("booklet/Icon.qml", "qt/qml");

    let mut app = QApp::new();
    app.register::<library::Library>()
        .register::<note::NoteEditor>()
        .register::<links::Backlinks>()
        .register::<sync::Sync>();

    // SAFETY: plain qmlRegisterType calls, made after the application exists and
    // before any QML is loaded.
    unsafe {
        booklet_register_highlighter();
        booklet_register_clipboard_image();
    }

    app.add_import_path("qrc:/qt/qml")
        .load_qml_from_file("qrc:/qt/qml/booklet/Main.qml")
        .run();
}
