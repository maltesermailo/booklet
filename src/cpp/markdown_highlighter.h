// Live preview for the block editor.
//
// The one place C++ is sanctioned (see CLAUDE.md): highlighting has to attach to
// TextEdit.textDocument, and qtbridge 0.2 exposes no text-document types, so
// this cannot be reached from Rust. Everything else stays Rust.
#pragma once

#include <QColor>
#include <QQuickTextDocument>
#include <QSyntaxHighlighter>
#include <QTextCharFormat>

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
    Q_PROPERTY(QColor markerColor MEMBER m_markerColor NOTIFY styleChanged)
    Q_PROPERTY(QColor textColor MEMBER m_textColor NOTIFY styleChanged)
    Q_PROPERTY(QColor linkColor MEMBER m_linkColor NOTIFY styleChanged)
    Q_PROPERTY(QString headingFamily MEMBER m_headingFamily NOTIFY styleChanged)
    Q_PROPERTY(int headingPixelSize MEMBER m_headingPixelSize NOTIFY styleChanged)

public:
    explicit MarkdownHighlighter(QObject *parent = nullptr);

    QQuickTextDocument *quickDocument() const;
    void setQuickDocument(QQuickTextDocument *document);

    int cursorPosition() const;
    void setCursorPosition(int position);

Q_SIGNALS:
    void documentChanged();
    void cursorPositionChanged();
    void styleChanged();

protected:
    void highlightBlock(const QString &text) override;

private:
    // `base` carries the face the marker should take if it is shown.
    QTextCharFormat markerFormat(const QTextCharFormat &base, bool onCursorLine) const;
    int blockNumberAt(int position) const;

    QQuickTextDocument *m_document = nullptr;
    int m_cursorPosition = -1;
    int m_cursorBlock = -1;
    QColor m_markerColor;
    QColor m_textColor;
    QColor m_linkColor;
    QString m_headingFamily;
    int m_headingPixelSize = 24;
};

// Called from Rust before the QML engine loads.
extern "C" void booklet_register_highlighter();
