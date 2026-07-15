#include "markdown_highlighter.h"

#include <QFont>
#include <QRegularExpression>
#include <QTextBlock>
#include <QTextDocument>
#include <qqml.h>

MarkdownHighlighter::MarkdownHighlighter(QObject *parent)
    : QSyntaxHighlighter(parent)
{
}

QQuickTextDocument *MarkdownHighlighter::quickDocument() const
{
    return m_document;
}

void MarkdownHighlighter::setQuickDocument(QQuickTextDocument *document)
{
    if (m_document == document)
        return;

    m_document = document;
    // Attaching to the QTextDocument is the whole reason this class exists.
    QSyntaxHighlighter::setDocument(m_document ? m_document->textDocument() : nullptr);
    Q_EMIT documentChanged();
}

int MarkdownHighlighter::cursorPosition() const
{
    return m_cursorPosition;
}

void MarkdownHighlighter::setCursorPosition(int position)
{
    if (m_cursorPosition == position)
        return;

    m_cursorPosition = position;
    Q_EMIT cursorPositionChanged();

    // Only the caret's line shows its markers, so a re-highlight is needed when
    // the caret crosses into another line — not on every keystroke within one.
    const int block = blockNumberAt(position);
    if (block != m_cursorBlock) {
        m_cursorBlock = block;
        rehighlight();
    }
}

int MarkdownHighlighter::blockNumberAt(int position) const
{
    // document() is QSyntaxHighlighter's — the QTextDocument we attached to.
    if (!document() || position < 0)
        return -1;

    return document()->findBlock(position).blockNumber();
}

QTextCharFormat MarkdownHighlighter::markerFormat(const QTextCharFormat &base, bool onCursorLine) const
{
    QTextCharFormat format = base;

    if (onCursorLine) {
        // The line you are editing shows its syntax, dimmed.
        format.setForeground(m_markerColor);
        return format;
    }

    // Everywhere else the marker collapses to nothing, so the text reflows as
    // if it were not written. Qt cannot hide text outright, and letter spacing
    // cannot do it either: the text engine skips its spacing pass entirely when
    // the value is 0, and even non-zero it never adjusts the last glyph's
    // advance. Condensing the font to 1% does work — stretch scales every
    // glyph — and transparent ink hides the hairline that is left.
    format.setFontStretch(1);
    format.setForeground(Qt::transparent);
    return format;
}

void MarkdownHighlighter::highlightBlock(const QString &text)
{
    const bool onCursorLine = currentBlock().blockNumber() == m_cursorBlock;

    // ATX heading: the leading #s are the marker, the rest is the heading. The
    // heading takes its face immediately, so "# Test" reads as a heading while
    // you are still typing it.
    static const QRegularExpression heading(QStringLiteral("^(#{1,6})(\\s+)(.*)$"));
    const QRegularExpressionMatch headingMatch = heading.match(text);
    if (headingMatch.hasMatch()) {
        QFont face(m_headingFamily);
        face.setPixelSize(m_headingPixelSize);
        face.setWeight(QFont::Medium);

        QTextCharFormat body;
        body.setFont(face);
        body.setForeground(m_textColor);
        setFormat(headingMatch.capturedStart(3), headingMatch.capturedLength(3), body);

        setFormat(headingMatch.capturedStart(1),
                  headingMatch.capturedLength(1) + headingMatch.capturedLength(2),
                  markerFormat(body, onCursorLine));
    }

    // **bold**: the text thickens, the asterisks go.
    static const QRegularExpression bold(QStringLiteral("(\\*\\*)([^*]+)(\\*\\*)"));
    for (auto it = bold.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        QTextCharFormat strong;
        strong.setFontWeight(QFont::Bold);
        setFormat(match.capturedStart(2), match.capturedLength(2), strong);

        const QTextCharFormat marker = markerFormat(strong, onCursorLine);
        setFormat(match.capturedStart(1), 2, marker);
        setFormat(match.capturedStart(3), 2, marker);
    }

    // *italic*: same, for single asterisks that are not part of a **pair**.
    static const QRegularExpression italic(QStringLiteral("(?<!\\*)(\\*)([^*]+)(\\*)(?!\\*)"));
    for (auto it = italic.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        QTextCharFormat emphasis;
        emphasis.setFontItalic(true);
        setFormat(match.capturedStart(2), match.capturedLength(2), emphasis);

        const QTextCharFormat marker = markerFormat(emphasis, onCursorLine);
        setFormat(match.capturedStart(1), 1, marker);
        setFormat(match.capturedStart(3), 1, marker);
    }

    // [[wiki links]]: the target reads as a link, the brackets go.
    static const QRegularExpression link(QStringLiteral("(\\[\\[)([^\\]]+)(\\]\\])"));
    for (auto it = link.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        QTextCharFormat target;
        target.setForeground(m_linkColor);
        target.setFontUnderline(true);
        setFormat(match.capturedStart(2), match.capturedLength(2), target);

        const QTextCharFormat marker = markerFormat(target, onCursorLine);
        setFormat(match.capturedStart(1), 2, marker);
        setFormat(match.capturedStart(3), 2, marker);
    }
}

extern "C" void booklet_register_highlighter()
{
    qmlRegisterType<MarkdownHighlighter>("booklet", 1, 0, "MarkdownHighlighter");
}
