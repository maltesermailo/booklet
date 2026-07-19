// Reads an image off the system clipboard, so a paste (⌘V) into the editor can be
// saved into the note's folder.
//
// Sanctioned C++ for the same reason as the highlighter (see CLAUDE.md): QClipboard
// and QImage are not reachable from qtbridge 0.2. Everything the image is then —
// naming, dedup, writing to disk — stays in Rust (booklet-core::image); this only
// hands the bytes across as base64 PNG.
#pragma once

#include <QObject>
#include <QString>

class ClipboardImage : public QObject
{
    Q_OBJECT

public:
    explicit ClipboardImage(QObject *parent = nullptr);

    // The clipboard's image as base64-encoded PNG, or "" when it holds no image
    // (so the caller can fall through to a normal text paste).
    Q_INVOKABLE QString pngBase64() const;
};

// Called from Rust before the QML engine loads.
extern "C" void booklet_register_clipboard_image();
