#include "markdown_highlighter.h"

#include <QFont>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
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

QStringList MarkdownHighlighter::knownTitles() const
{
    return m_knownTitles;
}

void MarkdownHighlighter::setKnownTitles(const QStringList &titles)
{
    if (m_knownTitles == titles)
        return;

    m_knownTitles = titles;
    // A note appearing or vanishing changes which links resolve.
    rehighlight();
    Q_EMIT knownTitlesChanged();
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

void MarkdownHighlighter::setDecorations(const QString &json)
{
    if (json == m_decorationsJson)
        return;
    m_decorationsJson = json;

    m_decos.clear();
    const QJsonArray array = QJsonDocument::fromJson(json.toUtf8()).array();
    m_decos.reserve(array.size());
    for (const QJsonValue &value : array) {
        const QJsonObject object = value.toObject();
        Deco deco;
        deco.start = object.value(QStringLiteral("start")).toInt();
        deco.len = object.value(QStringLiteral("len")).toInt();
        deco.kind = object.value(QStringLiteral("kind")).toString();
        deco.level = object.value(QStringLiteral("level")).toInt();
        deco.text = object.value(QStringLiteral("text")).toString();
        deco.flag = object.value(QStringLiteral("flag")).toBool();
        m_decos.push_back(deco);
    }

    rehighlight();
}

QTextCharFormat MarkdownHighlighter::formatFor(const Deco &deco, bool onCursorLine) const
{
    const QString &kind = deco.kind;
    QTextCharFormat format;

    if (kind == QLatin1String("marker")) {
        // The delimiter chars: dimmed on the caret's line, collapsed to nothing
        // elsewhere (see markerFormat).
        return markerFormat(QTextCharFormat(), onCursorLine);
    }
    if (kind == QLatin1String("heading")) {
        // Size by level: # largest, ###### smallest, as ratios of the H1 size.
        static const double kScale[] = {1.0, 1.0, 0.84, 0.72, 0.63, 0.57, 0.52};
        const int level = qBound(1, deco.level, 6);
        QFont face(m_headingFamily);
        face.setPixelSize(static_cast<int>(m_headingPixelSize * kScale[level] + 0.5));
        face.setWeight(QFont::Medium);
        format.setFont(face);
        format.setForeground(m_textColor);
    } else if (kind == QLatin1String("strong")) {
        format.setFontWeight(QFont::Bold);
    } else if (kind == QLatin1String("em")) {
        format.setFontItalic(true);
    } else if (kind == QLatin1String("strike")) {
        format.setFontStrikeOut(true);
    } else if (kind == QLatin1String("code") || kind == QLatin1String("math")) {
        format.setFontFamilies({QStringLiteral("JetBrains Mono"), QStringLiteral("monospace")});
    } else if (kind == QLatin1String("code_block")) {
        // A fenced/indented block: monospace on a filled ground, so it reads as a
        // box. The ``` fences stay visible in mono for this phase.
        format.setFontFamilies({QStringLiteral("JetBrains Mono"), QStringLiteral("monospace")});
        format.setBackground(m_codeBackground);
    } else if (kind == QLatin1String("blockquote")) {
        // Quoted prose: italic on a faint ground; the `>` markers collapse.
        format.setFontItalic(true);
        format.setBackground(m_codeBackground);
    } else if (kind == QLatin1String("list_marker")) {
        // A visible bullet in the accent colour (not collapsed like a marker).
        format.setForeground(m_linkColor);
    } else if (kind == QLatin1String("link")) {
        format.setForeground(m_linkColor);
        format.setUnderlineStyle(QTextCharFormat::SingleUnderline);
    } else if (kind == QLatin1String("wikilink")) {
        // A link to a note that does not exist yet is dimmed and dashed, so a
        // link broken by a rename is visible rather than a surprise.
        const bool resolved = m_knownTitles.contains(deco.text);
        format.setForeground(resolved ? m_linkColor : m_unresolvedColor);
        format.setUnderlineStyle(resolved ? QTextCharFormat::SingleUnderline : QTextCharFormat::DashUnderline);
    } else if (kind == QLatin1String("html")) {
        format.setForeground(m_markerColor);
    }
    // Unknown / block-only kinds contribute no character format in this phase.

    return format;
}

void MarkdownHighlighter::highlightBlock(const QString &text)
{
    const int blockLength = text.length();
    if (blockLength == 0)
        return;

    const int blockStart = currentBlock().position();
    const int blockEnd = blockStart + blockLength;
    const bool onCursorLine = currentBlock().blockNumber() == m_cursorBlock;

    // Merge each overlapping decoration's format per character, so nested spans
    // compose (bold in a heading, italic in bold). Qt's setFormat replaces rather
    // than merges, so we accumulate first, then emit contiguous runs.
    QVector<QTextCharFormat> formats(blockLength);
    for (const Deco &deco : m_decos) {
        const int decoEnd = deco.start + deco.len;
        if (decoEnd <= blockStart || deco.start >= blockEnd)
            continue;

        const int from = qMax(0, deco.start - blockStart);
        const int to = qMin(blockLength, decoEnd - blockStart);
        const QTextCharFormat format = formatFor(deco, onCursorLine);
        for (int i = from; i < to; ++i)
            formats[i].merge(format);
    }

    int runStart = 0;
    for (int i = 1; i <= blockLength; ++i) {
        if (i == blockLength || formats[i] != formats[runStart]) {
            if (formats[runStart].propertyCount() > 0)
                setFormat(runStart, i - runStart, formats[runStart]);
            runStart = i;
        }
    }
}

extern "C" void booklet_register_highlighter()
{
    qmlRegisterType<MarkdownHighlighter>("booklet", 1, 0, "MarkdownHighlighter");
}
