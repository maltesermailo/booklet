#include "markdown_highlighter.h"

#include <QFont>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QFontMetrics>
#include <QTextBlock>
#include <QTextCursor>
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

int MarkdownHighlighter::renderedRowCount(int start, int end) const
{
    if (!document())
        return 1;

    QTextCursor cursor(document());
    cursor.setPosition(start);
    cursor.setPosition(end, QTextCursor::KeepAnchor);
    // selectedText() joins block boundaries with U+2029, not '\n'. Count only the
    // lines that hold content, so a trailing newline does not add a phantom row —
    // this must match the QML grid's own parse, which skips blank lines.
    int content = 0;
    for (const QString &line : cursor.selectedText().split(QChar::ParagraphSeparator))
        if (!line.trimmed().isEmpty())
            ++content;

    // Every GFM table has exactly one `|---|` separator row, drawn as the header's
    // underline rather than a row of its own.
    return qMax(1, content - 1);
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

QTextCharFormat MarkdownHighlighter::reservingFormat(int lineHeight) const
{
    // A font's line advances taller than its pixel size (ascent + descent +
    // leading); measure that ratio so the reserved line ends up the height asked
    // for rather than ~18% more.
    QFont probe;
    probe.setStretch(1);
    probe.setPixelSize(1000);
    const double ratio = QFontMetrics(probe).height() / 1000.0;

    QFont font;
    font.setStretch(1); // 1% width — collapses to nothing horizontally
    font.setPixelSize(qMax(1, qRound(lineHeight / ratio)));

    QTextCharFormat format;
    format.setFont(font);
    format.setForeground(Qt::transparent);
    return format;
}

QColor MarkdownHighlighter::tokenColor(const QString &klass) const
{
    if (klass == QLatin1String("keyword"))
        return m_codeKeyword;
    if (klass == QLatin1String("string"))
        return m_codeString;
    if (klass == QLatin1String("comment"))
        return m_codeComment;
    if (klass == QLatin1String("number"))
        return m_codeNumber;
    if (klass == QLatin1String("function"))
        return m_codeFunction;
    if (klass == QLatin1String("type"))
        return m_codeType;
    return m_codeConstant; // "constant"
}

void MarkdownHighlighter::setImageHeights(const QString &json)
{
    if (json == m_imageHeightsJson)
        return;
    m_imageHeightsJson = json;

    m_imageHeights.clear();
    const QJsonObject object = QJsonDocument::fromJson(json.toUtf8()).object();
    for (auto it = object.begin(); it != object.end(); ++it)
        m_imageHeights.insert(it.key().toInt(), it.value().toInt());

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
    } else if (kind == QLatin1String("code_token")) {
        // A syntax-highlighted run inside a code block: just the foreground colour
        // — it merges over the block's mono + background (emitted first).
        format.setForeground(tokenColor(deco.text));
        if (deco.text == QLatin1String("comment"))
            format.setFontItalic(true);
        return format;
    } else if (kind == QLatin1String("math_block")) {
        // Display math ($$…$$): monospace on the code ground, a styled-TeX baseline
        // until a real typesetter drops in.
        format.setFontFamilies({QStringLiteral("JetBrains Mono"), QStringLiteral("monospace")});
        format.setBackground(m_codeBackground);
    } else if (kind == QLatin1String("footnote_ref")) {
        // A [^1] reference: the accent colour, so it reads as a link to its note.
        format.setForeground(m_linkColor);
    } else if (kind == QLatin1String("html_block")) {
        // Raw block HTML: shown dimmed as source (no rendering — no script/fetch).
        format.setForeground(m_markerColor);
    } else if (kind == QLatin1String("blockquote")) {
        // Quoted prose: italic on a faint ground; the `>` markers collapse.
        format.setFontItalic(true);
        format.setBackground(m_codeBackground);
    } else if (kind == QLatin1String("list_marker")) {
        // A visible bullet in the accent colour (not collapsed like a marker).
        format.setForeground(m_linkColor);
    } else if (kind == QLatin1String("task")) {
        // The `[ ]` / `[x]` source. On the caret's line it shows dimmed so it can
        // be edited; elsewhere the ink is hidden but the width is *kept* — the QML
        // checkbox overlay draws into the space this reserves (a collapse to zero
        // width would leave it nowhere to sit).
        if (onCursorLine)
            return markerFormat(QTextCharFormat(), true);
        format.setForeground(Qt::transparent);
        return format;
    } else if (kind == QLatin1String("rule")) {
        // A thematic break (`---` / `***` / `___`): dimmed on the caret's line,
        // collapsed to an empty line elsewhere where the QML overlay draws the
        // rule across it.
        return markerFormat(QTextCharFormat(), onCursorLine);
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

    // A table is a block widget: unless the caret is inside it (then it shows raw
    // for editing), every char is hidden — zero width, transparent — and the first
    // line carries an oversized-but-invisible char so its block reserves the whole
    // grid's height. The QML overlay draws the grid into that reserved space. This
    // overrides (not merges) whatever the inline pass set, since nothing in a
    // collapsed table shows.
    for (const Deco &deco : m_decos) {
        if (deco.kind != QLatin1String("table"))
            continue;

        const int tableStart = deco.start;
        const int tableEnd = deco.start + deco.len;
        if (tableEnd <= blockStart || tableStart >= blockEnd)
            continue;
        if (m_cursorPosition >= tableStart && m_cursorPosition <= tableEnd)
            continue; // editing the table — leave the source as written

        QTextCharFormat hidden;
        QFont collapsed;
        collapsed.setPixelSize(1);
        collapsed.setStretch(1); // 1% width — the proven marker-collapse trick
        hidden.setFont(collapsed);
        hidden.setForeground(Qt::transparent);

        const int from = qMax(0, tableStart - blockStart);
        const int to = qMin(blockLength, tableEnd - blockStart);
        for (int i = from; i < to; ++i)
            formats[i] = hidden;

        // On the table's first line, one char reserves the grid's height, less
        // what the collapsed rows below still occupy (a 1px font has a floor line
        // height), so the block totals exactly the grid height with no gap beneath.
        if (tableStart >= blockStart && tableStart < blockEnd) {
            const int rows = renderedRowCount(tableStart, tableEnd);
            const int leaked = rows * QFontMetrics(collapsed).height();
            formats[tableStart - blockStart] = reservingFormat(rows * m_tableRowHeight - leaked);
        }
    }

    // An image is the same kind of block widget as a table: unless the caret is on
    // it (then the `![alt](src)` shows for editing), the source is hidden and one
    // char reserves the picture's height — measured by the QML overlay after the
    // image loads and pushed back via imageHeights. Until that arrives the reserve
    // is a placeholder line, so an image appears compact then grows to fit.
    for (const Deco &deco : m_decos) {
        if (deco.kind != QLatin1String("image"))
            continue;

        const int imageStart = deco.start;
        const int imageEnd = deco.start + deco.len;
        if (imageEnd <= blockStart || imageStart >= blockEnd)
            continue;
        if (onCursorLine)
            continue; // editing this line — leave the `![alt](src)` as written

        QTextCharFormat hidden;
        QFont collapsed;
        collapsed.setPixelSize(1);
        collapsed.setStretch(1);
        hidden.setFont(collapsed);
        hidden.setForeground(Qt::transparent);

        const int from = qMax(0, imageStart - blockStart);
        const int to = qMin(blockLength, imageEnd - blockStart);
        for (int i = from; i < to; ++i)
            formats[i] = hidden;

        if (imageStart >= blockStart && imageStart < blockEnd) {
            // A single line, so the whole picture height is reserved here.
            const int reserved = m_imageHeights.value(imageStart, m_headingPixelSize);
            formats[imageStart - blockStart] = reservingFormat(reserved);
        }
    }

    // A fenced block's ``` / ~~~ lines are syntax, not content: collapse them to a
    // near-zero-height line so the box wraps tight around the code. They reveal
    // whenever the caret is anywhere inside the block — not just on the fence line
    // — so the fences are there to edit while you work in it, as a table reveals
    // its whole source.
    const QString trimmed = text.trimmed();
    if (trimmed.startsWith(QLatin1String("```")) || trimmed.startsWith(QLatin1String("~~~"))) {
        for (const Deco &deco : m_decos) {
            if (deco.kind != QLatin1String("code_block"))
                continue;

            const int cbStart = deco.start;
            const int cbEnd = deco.start + deco.len;
            if (cbEnd <= blockStart || cbStart >= blockEnd)
                continue;
            if (m_cursorPosition >= cbStart && m_cursorPosition <= cbEnd)
                break; // caret is in this block — show the fences for editing

            const QTextCharFormat collapsed = reservingFormat(1);
            for (int i = 0; i < blockLength; ++i)
                formats[i] = collapsed;
            break;
        }
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
