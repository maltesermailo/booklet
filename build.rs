//! Builds the two things the Rust toolchain cannot on its own: the bundled
//! fonts (a Qt binary resource) and the live-preview highlighter (C++ + moc).
//!
//! Qt paths come from `qtbridge-build-utils` — the same crate qtbridge's own
//! build uses — so the macOS framework layout, where some modules ship as
//! frameworks and others (QtQmlIntegration) as plain include dirs, is handled
//! for us rather than hardcoded here.

use qtbridge_build_utils::qt_build::QtInstallation;
use std::path::{Path, PathBuf};
use std::process::Command;

const QML_DIR: &str = "src/booklet";
const FONTS_QRC: &str = "src/booklet/fonts.qrc";
const FONTS_DIR: &str = "src/booklet/fonts";
/// The sanctioned C++ pieces (see CLAUDE.md), each a `(header, source)` compiled
/// with moc: the live-preview highlighter, and the clipboard-image bridge that a
/// paste needs (QClipboard is not reachable from qtbridge).
const CPP_CLASSES: [(&str, &str); 2] = [
    ("src/cpp/markdown_highlighter.h", "src/cpp/markdown_highlighter.cpp"),
    ("src/cpp/clipboard_image.h", "src/cpp/clipboard_image.cpp"),
];

/// The Qt modules the C++ includes and links against.
const QT_MODULES: [&str; 4] = ["Core", "Gui", "Qml", "Quick"];

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"));
    let qt = QtInstallation::new();

    watch_qml();
    compile_fonts(&out_dir);
    compile_cpp(&qt, &out_dir);
}

/// `include_bytes_qml!` reads the QML at macro-expansion time and expands it to
/// token literals, so rustc never records it as a dependency. Without this,
/// editing a `.qml` and rebuilding silently keeps the old UI in the binary.
fn watch_qml() {
    for entry in std::fs::read_dir(QML_DIR).expect("QML directory is present") {
        let path = entry.expect("readable QML entry").path();
        let watched = path.extension().is_some_and(|ext| ext == "qml")
            || path.file_name().is_some_and(|name| name == "qmldir");

        if watched {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// `include_bytes_qml!` expands every byte of a file into a token literal, which
/// does not scale to megabytes of font data, so the fonts go through Qt's own
/// `rcc` instead and `main.rs` embeds the resulting blob with `include_bytes!`.
fn compile_fonts(out_dir: &Path) {
    println!("cargo:rerun-if-changed={FONTS_QRC}");
    for entry in std::fs::read_dir(FONTS_DIR).expect("fonts directory is present") {
        let path = entry.expect("readable font entry").path();
        if path.extension().is_some_and(|ext| ext == "ttf") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let out_path = out_dir.join("fonts.rcc");
    let rcc = rcc_path();

    let status = Command::new(&rcc)
        .args(["-binary", FONTS_QRC, "-o"])
        .arg(&out_path)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", rcc.display()));
    assert!(status.success(), "{} failed on {FONTS_QRC}", rcc.display());
}

/// Compiles the sanctioned C++ (see CLAUDE.md): pieces that must reach Qt types
/// qtbridge does not expose — the highlighter (`TextEdit.textDocument`) and the
/// clipboard-image bridge (`QClipboard`). Each header is run through moc and
/// compiled with its source into one static lib.
fn compile_cpp(qt: &QtInstallation, out_dir: &Path) {
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17");

    for (header, source) in CPP_CLASSES {
        println!("cargo:rerun-if-changed={source}");
        println!("cargo:rerun-if-changed={header}");

        // moc turns the Q_OBJECT macro into the meta-object C++ that Qt needs.
        let stem = Path::new(header).file_stem().expect("header has a stem");
        let moc_out = out_dir.join(format!("moc_{}.cpp", stem.to_string_lossy()));
        qt.run_moc(Path::new(header), &moc_out);

        build.file(source).file(&moc_out);
    }

    for dir in qt.include_dirs(QT_MODULES, false) {
        build.include(dir);
    }
    qt.configure_builder(&mut build);
    build.compile("booklet_cpp");

    qt.link_modules(QT_MODULES);
}

/// `rcc` ships in Qt's libexec directory. Ask qmake where that is, using the
/// same `QMAKE` override qtbridge itself honors.
fn rcc_path() -> PathBuf {
    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "qmake".into());
    let output = Command::new(&qmake)
        .args(["-query", "QT_INSTALL_LIBEXECS"])
        .output()
        .unwrap_or_else(|error| panic!("failed to run '{qmake} -query': {error}"));

    let libexec = String::from_utf8_lossy(&output.stdout).trim().to_string();
    PathBuf::from(libexec).join("rcc")
}
