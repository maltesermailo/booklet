//! Compiles the bundled fonts into a Qt binary resource.
//!
//! `include_bytes_qml!` expands every byte of a file into a token literal, which
//! does not scale to megabytes of font data, so the fonts go through Qt's own
//! `rcc` instead and `main.rs` embeds the resulting blob with `include_bytes!`.

use std::path::PathBuf;
use std::process::Command;

const FONTS_QRC: &str = "src/booklet/fonts.qrc";
const FONTS_DIR: &str = "src/booklet/fonts";

fn main() {
    println!("cargo:rerun-if-changed={FONTS_QRC}");
    for entry in std::fs::read_dir(FONTS_DIR).expect("fonts directory is present") {
        let path = entry.expect("readable font entry").path();
        if path.extension().is_some_and(|ext| ext == "ttf") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    let out_path = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set")).join("fonts.rcc");
    let rcc = rcc_path();

    let status = Command::new(&rcc)
        .args(["-binary", FONTS_QRC, "-o"])
        .arg(&out_path)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", rcc.display()));
    assert!(status.success(), "{} failed on {FONTS_QRC}", rcc.display());
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
