// Live preview for the block editor.
//
// The one place C++ is sanctioned (see CLAUDE.md): highlighting has to attach to
// TextEdit.textDocument, and qtbridge 0.2 exposes no text-document types, so
// this cannot be reached from Rust. Everything else stays Rust.
#pragma once

#include <QColor>
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
    Q_PROPERTY(QString headingFamily MEMBER m_headingFamily NOTIFY styleChanged)
    Q_PROPERTY(int headingPixelSize MEMBER m_headingPixelSize NOTIFY styleChanged)
    // The decoration list (JSON) from `NoteEditor.decorations()` — parsed
    // CommonMark+GFM spans this styles. Write-only: the editor pushes it after
    // each keystroke's `set_source`.
    Q_PROPERTY(QString decorations WRITE setDecorations)

public:
    explicit MarkdownHighlighter(QObject *parent = nullptr);

    QQuickTextDocument *quickDocument() const;
    void setQuickDocument(QQuickTextDocument *document);

    int cursorPosition() const;
    void setCursorPosition(int position);

    QStringList knownTitles() const;
    void setKnownTitles(const QStringList &titles);

    void setDecorations(const QString &json);

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

    QVector<Deco> m_decos;
    // The JSON last applied, so an unchanged push is a no-op — which also breaks
    // the rehighlight → text-change → re-push cycle (a format pass never changes
    // the source, so the re-pushed JSON is identical and stops here).
    QString m_decorationsJson;

    QQuickTextDocument *m_document = nullptr;
    int m_cursorPosition = -1;
    int m_cursorBlock = -1;
    QStringList m_knownTitles;
    QColor m_markerColor;
    QColor m_textColor;
    QColor m_linkColor;
    QColor m_unresolvedColor;
    QColor m_codeBackground;
    QString m_headingFamily;
    int m_headingPixelSize = 24;
};

// Called from Rust before the QML engine loads.
extern "C" void booklet_register_highlighter();
