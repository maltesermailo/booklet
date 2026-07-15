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

const FONTS_QRC: &str = "src/booklet/fonts.qrc";
const FONTS_DIR: &str = "src/booklet/fonts";
const HIGHLIGHTER_HEADER: &str = "src/cpp/markdown_highlighter.h";
const HIGHLIGHTER_SOURCE: &str = "src/cpp/markdown_highlighter.cpp";

/// The Qt modules the highlighter includes and links against.
const QT_MODULES: [&str; 4] = ["Core", "Gui", "Qml", "Quick"];

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"));
    let qt = QtInstallation::new();

    compile_fonts(&out_dir);
    compile_highlighter(&qt, &out_dir);
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

/// The markdown highlighter is the one sanctioned piece of C++ (see CLAUDE.md):
/// it must attach to `TextEdit.textDocument`, which qtbridge does not expose.
fn compile_highlighter(qt: &QtInstallation, out_dir: &Path) {
    println!("cargo:rerun-if-changed={HIGHLIGHTER_SOURCE}");

    // moc turns the Q_OBJECT macro into the meta-object C++ that Qt needs.
    let moc_out = out_dir.join("moc_markdown_highlighter.cpp");
    qt.run_moc(Path::new(HIGHLIGHTER_HEADER), &moc_out);

    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file(HIGHLIGHTER_SOURCE).file(&moc_out);
    for dir in qt.include_dirs(QT_MODULES, false) {
        build.include(dir);
    }
    qt.configure_builder(&mut build);
    build.compile("booklet_highlighter");

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
