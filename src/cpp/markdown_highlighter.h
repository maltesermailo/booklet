// Live preview for the block editor.
//
// The one place C++ is sanctioned (see CLAUDE.md): highlighting has to attach to
// TextEdit.textDocument, and qtbridge 0.2 exposes no text-document types, so
// this cannot be reached from Rust. Everything else stays Rust.
#pragma once

#include <QColor>
#include <QHash>
#include <QQuickTextDocument>
#include <QString>
#include <QSyntaxHighlighter>
#include <QTextCharFormat>
#include <QVector>

// Dims markdown's syntax markers and styles the text as it will render, so
// "# Test" already reads as a heading while you type it.
class MarkdownHighlighter : public QSyntaxHighlighter
{
    Q_OBJECT

    // Named apart from QSyntaxHighlighter::document()/setDocument(), which take
    // a QTextDocument — same names here would hide the base's and recurse.
    Q_PROPERTY(QQuickTextDocument *document READ quickDocument WRITE setQuickDocument NOTIFY documentChanged)
    // Where the caret is. Syntax markers show only on the line holding it, and
    // collapse to nothing everywhere else — Obsidian's live preview.
    Q_PROPERTY(int cursorPosition READ cursorPosition WRITE setCursorPosition NOTIFY cursorPositionChanged)
    // The titles that exist in the vault. A [[link]] to anything else is drawn
    // as unresolved — which matters because renaming a note deliberately does
    // not rewrite the links pointing at it.
    Q_PROPERTY(QStringList knownTitles READ knownTitles WRITE setKnownTitles NOTIFY knownTitlesChanged)
    Q_PROPERTY(QColor markerColor MEMBER m_markerColor NOTIFY styleChanged)
    Q_PROPERTY(QColor textColor MEMBER m_textColor NOTIFY styleChanged)
    Q_PROPERTY(QColor linkColor MEMBER m_linkColor NOTIFY styleChanged)
    Q_PROPERTY(QColor unresolvedColor MEMBER m_unresolvedColor NOTIFY styleChanged)
    // Fill behind code blocks and blockquotes (the `--code-bg` / `--edit-bg` token).
    Q_PROPERTY(QColor codeBackground MEMBER m_codeBackground NOTIFY styleChanged)
    // Syntax-highlighting colours for fenced code, one per semantic token class
    // (from render.rs). Kept as theme properties so highlighting follows the app's
    // light/dark themes rather than a bundled syntect theme.
    Q_PROPERTY(QColor codeKeyword MEMBER m_codeKeyword NOTIFY styleChanged)
    Q_PROPERTY(QColor codeString MEMBER m_codeString NOTIFY styleChanged)
    Q_PROPERTY(QColor codeComment MEMBER m_codeComment NOTIFY styleChanged)
    Q_PROPERTY(QColor codeNumber MEMBER m_codeNumber NOTIFY styleChanged)
    Q_PROPERTY(QColor codeFunction MEMBER m_codeFunction NOTIFY styleChanged)
    Q_PROPERTY(QColor codeType MEMBER m_codeType NOTIFY styleChanged)
    Q_PROPERTY(QColor codeConstant MEMBER m_codeConstant NOTIFY styleChanged)
    // Pixel height of one rendered table row. The QML grid overlay draws each row
    // this tall; the highlighter reserves the matching height in the document so
    // the grid has room and the text below it flows clear.
    Q_PROPERTY(int tableRowHeight MEMBER m_tableRowHeight NOTIFY styleChanged)
    Q_PROPERTY(QString headingFamily MEMBER m_headingFamily NOTIFY styleChanged)
    Q_PROPERTY(int headingPixelSize MEMBER m_headingPixelSize NOTIFY styleChanged)
    // The decoration list (JSON) from `NoteEditor.decorations()` — parsed
    // CommonMark+GFM spans this styles. Write-only: the editor pushes it after
    // each keystroke's `set_source`.
    Q_PROPERTY(QString decorations WRITE setDecorations)
    // Measured render height per inline image, keyed by the image's document
    // offset (JSON object {"<offset>": <px>}). The QML overlay writes it once each
    // image loads and knows its size; the highlighter reserves that height on the
    // image's line so the text below flows clear of the drawn picture. Write-only.
    Q_PROPERTY(QString imageHeights WRITE setImageHeights)

public:
    explicit MarkdownHighlighter(QObject *parent = nullptr);

    QQuickTextDocument *quickDocument() const;
    void setQuickDocument(QQuickTextDocument *document);

    int cursorPosition() const;
    void setCursorPosition(int position);

    QStringList knownTitles() const;
    void setKnownTitles(const QStringList &titles);

    void setDecorations(const QString &json);
    void setImageHeights(const QString &json);

Q_SIGNALS:
    void documentChanged();
    void cursorPositionChanged();
    void knownTitlesChanged();
    void styleChanged();

protected:
    void highlightBlock(const QString &text) override;

private:
    // One decoration span, in document (UTF-16) coordinates, from the Rust parser.
    struct Deco
    {
        int start = 0;
        int len = 0;
        QString kind;
        int level = 0;
        QString text;
        bool flag = false;
    };

    // The character format a decoration contributes (merged per character so
    // nested spans — bold inside a heading, italic inside bold — compose).
    QTextCharFormat formatFor(const Deco &deco, bool onCursorLine) const;
    // `base` carries the face the marker should take if it is shown.
    QTextCharFormat markerFormat(const QTextCharFormat &base, bool onCursorLine) const;
    int blockNumberAt(int position) const;
    // Rows the grid draws for a table (its lines minus the `|---|` separator) —
    // what the reserved height is a multiple of.
    int renderedRowCount(int start, int end) const;
    // The colour for a fenced-code semantic token class ("keyword", "string", …).
    QColor tokenColor(const QString &klass) const;
    // A transparent, zero-width character format whose line reserves `lineHeight`
    // px — how a table/image block widget makes room for what the QML overlay
    // draws. A font's line advances taller than its pixel size, so the target is
    // converted back through that ratio.
    QTextCharFormat reservingFormat(int lineHeight) const;

    QVector<Deco> m_decos;
    // The JSON last applied, so an unchanged push is a no-op — which also breaks
    // the rehighlight → text-change → re-push cycle (a format pass never changes
    // the source, so the re-pushed JSON is identical and stops here).
    QString m_decorationsJson;
    // Image offset → measured render height (see the imageHeights property).
    QHash<int, int> m_imageHeights;
    QString m_imageHeightsJson;

    QQuickTextDocument *m_document = nullptr;
    int m_cursorPosition = -1;
    int m_cursorBlock = -1;
    QStringList m_knownTitles;
    QColor m_markerColor;
    QColor m_textColor;
    QColor m_linkColor;
    QColor m_unresolvedColor;
    QColor m_codeBackground;
    QColor m_codeKeyword;
    QColor m_codeString;
    QColor m_codeComment;
    QColor m_codeNumber;
    QColor m_codeFunction;
    QColor m_codeType;
    QColor m_codeConstant;
    QString m_headingFamily;
    int m_headingPixelSize = 24;
    int m_tableRowHeight = 40;
};

// Called from Rust before the QML engine loads.
extern "C" void booklet_register_highlighter();
