#include "chat_panel.h"
#include "editor_theme.h"

#include <QAbstractTextDocumentLayout>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QContextMenuEvent>
#include <QEnterEvent>
#include <QFrame>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QLinearGradient>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPushButton>
#include <QRegularExpression>
#include <QScrollArea>
#include <QScrollBar>
#include <QStyle>
#include <QTextBlock>
#include <QTextLayout>
#include <QTextDocument>
#include <QTextEdit>
#include <QTimer>
#include <QVBoxLayout>
#include <QWheelEvent>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// Text a label can wrap anywhere it must: a run of more than 24 characters
// with no space (a path, a URL) gets a zero-width break every 24, so it
// wraps inside the column instead of widening it.
QString wrappable(const QString &text) {
    QString out;
    out.reserve(text.size() + text.size() / 24);
    int run = 0;
    for (const QChar c : text) {
        run = c.isSpace() ? 0 : run + 1;
        if (run > 24) {
            out += QChar(0x200B);
            run = 1;
        }
        out += c;
    }
    return out;
}

// A file name ("fisher-close.mov") read as one word: a word joiner after
// each of its hyphens, so a line never breaks there ("fisher-" / "close.mov"
// reads as two words). The hyphen itself stays the same glyph.
QString keepNamesWhole(const QString &text) {
    static const QRegularExpression name(
        QStringLiteral("[\\w.\\-]*\\w-[\\w.\\-]*\\.(?:mov|mp4|m4v|mxf|mkv|avi|webm|wav|aif|aiff|mp3|m4a|flac|png|jpe?g|tiff?|exr|cube|srt|vtt)\\b"),
        QRegularExpression::CaseInsensitiveOption | QRegularExpression::UseUnicodePropertiesOption);
    QString out;
    out.reserve(text.size() + 4);
    qsizetype from = 0;
    for (QRegularExpressionMatchIterator it = name.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        out += text.mid(from, match.capturedStart() - from);
        out += match.captured().replace(QLatin1Char('-'), QStringLiteral("-\u2060"));
        from = match.capturedEnd();
    }
    out += text.mid(from);
    return out;
}

// The text without the invisible breaks wrappable() and keepNamesWhole()
// put in, as it should land on the clipboard.
QString withoutBreakHints(QString text) {
    text.remove(QChar(0x200B));
    text.remove(QChar(0x2060));
    return text;
}

// Selected comment text copies as it was written: ⌘C and the context menu's
// Copy take the selection without the break hints, so a file name pasted
// into Search media, Finder or a terminal still matches.
class CleanCopy final : public QObject {
public:
    using QObject::QObject;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        auto *body = qobject_cast<QLabel *>(watched);
        if (!body) return false;
        if (event->type() == QEvent::ShortcutOverride || event->type() == QEvent::KeyPress) {
            if (!static_cast<QKeyEvent *>(event)->matches(QKeySequence::Copy)) return false;
            if (event->type() == QEvent::ShortcutOverride) {
                event->accept();
                return true;
            }
            copy(body);
            return true;
        }
        if (event->type() == QEvent::ContextMenu) {
            QMenu menu(body);
            QAction *copyAction = menu.addAction(QObject::tr("Copy"));
            copyAction->setEnabled(body->hasSelectedText());
            QAction *all = menu.addAction(QObject::tr("Select All"));
            QAction *chosen = menu.exec(static_cast<QContextMenuEvent *>(event)->globalPos());
            if (chosen == copyAction) copy(body);
            else if (chosen == all) selectAll(body);
            return true;
        }
        return false;
    }

public:
    // The whole text selected; rich text counts its shown characters, not
    // its markup.
    static void selectAll(QLabel *body) {
        QTextDocument shown;
        if (body->textFormat() == Qt::RichText) shown.setHtml(body->text());
        else shown.setPlainText(body->text());
        body->setSelection(0, int(shown.characterCount()) - 1);
    }

private:
    static void copy(const QLabel *body) {
        if (body->hasSelectedText()) QApplication::clipboard()->setText(withoutBreakHints(body->selectedText()));
    }
};

void copiesCleanly(QLabel *body) {
    body->setContextMenuPolicy(Qt::DefaultContextMenu);
    body->installEventFilter(new CleanCopy(body));
}

// A comment's text as the row shows it: wrappable, file names kept whole, on
// 20-point lines for 14-point type, as a comments panel sets its text.
QString bodyMarkup(const QString &text) {
    return QStringLiteral("<div style=\"line-height:120%\">%1</div>")
        .arg(wrappable(keepNamesWhole(text)).toHtmlEscaped().replace(QLatin1Char('\n'), QStringLiteral("<br>")));
}

// The end of the chips' row fading into the panel's surface, while the row
// has more chips that way: clear, then the surface over kChipFadeWidth,
// then solid under the round button that pages the row. Clicks pass
// through it.
class ChipFade final : public QWidget {
public:
    ChipFade(bool leading, QWidget *parent) : QWidget(parent), m_leading(leading) {
        setObjectName(leading ? QStringLiteral("chipFadeLeading") : QStringLiteral("chipFadeTrailing"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        hide();
    }
    // How far its ramp runs in from its open edge; 0 covers it all, so a
    // chip that does not fit whole is hidden rather than half shown.
    void setRamp(int ramp) {
        if (ramp == m_ramp) return;
        m_ramp = ramp;
        update();
    }
    int ramp() const { return m_ramp; }

protected:
    void paintEvent(QPaintEvent *) override {
        const QColor surface = editorTheme().chatSurface;
        QColor clear = surface;
        clear.setAlpha(0);
        if (m_ramp <= 0) {
            QPainter(this).fillRect(rect(), surface);
            return;
        }
        const double ramp = std::clamp(double(m_ramp) / std::max(1, width()), 0.0, 1.0);
        QLinearGradient fade(0, 0, width(), 0);
        if (m_leading) {
            fade.setColorAt(0, surface);
            fade.setColorAt(1.0 - ramp, surface);
            fade.setColorAt(1, clear);
        } else {
            fade.setColorAt(0, clear);
            fade.setColorAt(ramp, surface);
            fade.setColorAt(1, surface);
        }
        QPainter(this).fillRect(rect(), fade);
    }

private:
    bool m_leading;
    int m_ramp = ChatPanel::kChipFadeWidth;
};

// The composer's underline: a hairline at rest, two points of the accent
// while a field in the composer has focus, as a comment field's.
class ComposerRule final : public QWidget {
public:
    explicit ComposerRule(QWidget *parent = nullptr) : QWidget(parent) {
        setObjectName(QStringLiteral("chatComposerRule"));
        setFixedHeight(2);
        setAttribute(Qt::WA_TransparentForMouseEvents);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        const EditorTheme &theme = editorTheme();
        QWidget *composer = parentWidget();
        while (composer && composer->objectName() != QLatin1String("chatComposer")) composer = composer->parentWidget();
        const QString state = composer ? composer->property("state").toString() : QString();
        QPainter painter(this);
        if (state == QLatin1String("focused")) {
            painter.fillRect(rect(), theme.chatComposerRuleFocus);
        } else {
            painter.fillRect(QRect(0, height() - 1, width(), 1), theme.chatComposerRule);
        }
    }
};

// A pin, drawn in code: the "Pinned by Kerf" line's symbol.
// One line of text that ends in an ellipsis where its width cuts it, as
// the compact proposal bar's summary does beside its pills.
class ElidedLabel final : public QLabel {
public:
    explicit ElidedLabel(QWidget *parent = nullptr) : QLabel(parent) { setTextFormat(Qt::PlainText); }
    // `fallback` (shorter, "3 changes") replaces the text whole wherever the
    // text does not fit whole, so a list is never cut mid-way.
    void setFullText(const QString &text, const QString &fallback = QString()) {
        m_full = text;
        m_fallback = fallback;
        elide();
    }

protected:
    void resizeEvent(QResizeEvent *event) override {
        QLabel::resizeEvent(event);
        elide();
    }

private:
    void elide() {
        const int room = std::max(0, contentsRect().width());
        const bool fits = fontMetrics().horizontalAdvance(m_full) <= room;
        const QString full = !m_fallback.isEmpty() && !fits ? m_fallback : m_full;
        const QString shown = fontMetrics().elidedText(full, Qt::ElideRight, room);
        if (shown != text()) setText(shown);
    }
    QString m_full;
    QString m_fallback;
};

class PinGlyph final : public QWidget {
public:
    explicit PinGlyph(QWidget *parent = nullptr) : QWidget(parent) {
        setObjectName(QStringLiteral("chatPinGlyph"));
        setFixedSize(12, 12);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        const QColor ink = editorTheme().chatSecondary;
        painter.setPen(QPen(ink, 1.2, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.setBrush(ink);
        // A head, a collar and a needle, leaning as a pin in a board.
        painter.translate(6, 6);
        painter.rotate(35);
        painter.drawRoundedRect(QRectF(-2.2, -5.5, 4.4, 5.0), 1.0, 1.0);
        painter.drawLine(QPointF(-3.6, -0.5), QPointF(3.6, -0.5));
        painter.drawLine(QPointF(0, -0.5), QPointF(0, 5.5));
    }
};

// Three dots after the latest comment while Kerf works, as a typing
// indicator, in the avatars' column.
class TypingDots final : public QWidget {
public:
    explicit TypingDots(QWidget *parent = nullptr) : QWidget(parent) {
        setObjectName(QStringLiteral("chatTyping"));
        setFixedSize(ChatPanel::kAvatar, 18);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        if (!property("active").toBool()) return;
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        painter.setBrush(editorTheme().chatSecondary);
        for (int dot = 0; dot < 3; ++dot) painter.drawEllipse(QPointF(width() / 2.0 + (dot - 1) * 7.0, height() / 2.0), 2.0, 2.0);
    }
};

QPushButton *pill(const QString &text, const char *objectName, int height) {
    auto *button = new QPushButton(text);
    button->setObjectName(QString::fromLatin1(objectName));
    button->setCursor(Qt::PointingHandCursor);
    // Its focus ring follows its round ends.
    button->setProperty("bezelRadius", height / 2);
    return button;
}

QLabel *label(const QString &text, const char *objectName) {
    auto *out = new QLabel(text);
    out->setObjectName(QString::fromLatin1(objectName));
    out->setTextFormat(Qt::PlainText);
    return out;
}

void repolish(QWidget *widget) {
    widget->style()->unpolish(widget);
    widget->style()->polish(widget);
    widget->update();
}

} // namespace

// Over the list's view, passing the pointer through: strips of the
// panel's surface where a row's cut would leave only a sliver of it.
class EdgeCover final : public QWidget {
public:
    explicit EdgeCover(QWidget *parent) : QWidget(parent) {
        setObjectName(QStringLiteral("chatEdgeCover"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setAttribute(Qt::WA_NoSystemBackground);
        setFocusPolicy(Qt::NoFocus);
    }
    void setStrips(const QList<QRect> &strips) {
        if (strips == m_strips) return;
        m_strips = strips;
        update();
    }
    QList<QRect> strips() const { return m_strips; }

protected:
    void paintEvent(QPaintEvent *) override {
        if (m_strips.isEmpty()) return;
        QPainter painter(this);
        for (const QRect &strip : std::as_const(m_strips)) painter.fillRect(strip, editorTheme().chatSurface);
    }

private:
    QList<QRect> m_strips;
};


QString relativeTime(const QDateTime &then, const QDateTime &now) {
    const qint64 seconds = std::max<qint64>(0, then.secsTo(now));
    if (seconds < 60) return QStringLiteral("now");
    if (seconds < 3600) return QStringLiteral("%1 min ago").arg(seconds / 60);
    if (seconds < 86400) return QStringLiteral("%1 h ago").arg(seconds / 3600);
    return then.date().toString(QStringLiteral("d MMM"));
}

// ---- ProposalChangeRow ----------------------------------------------------

namespace {

// A change's 16px glyph, drawn in code with a 1.5px pen in `ink`: scissors
// for a trim, a minus in a circle for a removal, a plus for an addition, a
// T for a title, arrows for a move, a cut line for a split, a speaker for
// a level, a dial for an effect, a dot for anything else.
void drawChangeGlyph(QPainter &p, const QRectF &box, ProposalChange::Glyph glyph, const QColor &ink) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.translate(box.topLeft());
    p.scale(box.width() / 16.0, box.height() / 16.0);
    p.setPen(QPen(ink, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    p.setBrush(Qt::NoBrush);
    switch (glyph) {
    case ProposalChange::Glyph::Trim:
        p.drawEllipse(QPointF(4.5, 11.5), 2.2, 2.2);
        p.drawEllipse(QPointF(11.5, 11.5), 2.2, 2.2);
        p.drawLine(QPointF(5.8, 9.8), QPointF(11.5, 2.5));
        p.drawLine(QPointF(10.2, 9.8), QPointF(4.5, 2.5));
        break;
    case ProposalChange::Glyph::Remove:
        p.drawEllipse(QPointF(8, 8), 6, 6);
        p.drawLine(QPointF(5, 8), QPointF(11, 8));
        break;
    case ProposalChange::Glyph::Add:
        p.drawEllipse(QPointF(8, 8), 6, 6);
        p.drawLine(QPointF(5, 8), QPointF(11, 8));
        p.drawLine(QPointF(8, 5), QPointF(8, 11));
        break;
    case ProposalChange::Glyph::Title:
        p.drawLine(QPointF(3, 3.5), QPointF(13, 3.5));
        p.drawLine(QPointF(8, 3.5), QPointF(8, 13));
        break;
    case ProposalChange::Glyph::Move:
        p.drawLine(QPointF(2.5, 8), QPointF(13.5, 8));
        p.drawPolyline(QPolygonF{QPointF(5, 5.5), QPointF(2.5, 8), QPointF(5, 10.5)});
        p.drawPolyline(QPolygonF{QPointF(11, 5.5), QPointF(13.5, 8), QPointF(11, 10.5)});
        break;
    case ProposalChange::Glyph::Split:
        p.drawRoundedRect(QRectF(2, 4.5, 12, 7), 1.5, 1.5);
        p.drawLine(QPointF(8, 2), QPointF(8, 14));
        break;
    case ProposalChange::Glyph::Level:
        p.drawPolygon(QPolygonF{QPointF(2.5, 6), QPointF(5, 6), QPointF(8.5, 3), QPointF(8.5, 13), QPointF(5, 10), QPointF(2.5, 10)});
        p.drawArc(QRectF(7, 5, 6, 6), -60 * 16, 120 * 16);
        break;
    case ProposalChange::Glyph::Effect:
        p.drawEllipse(QPointF(8, 8), 5.5, 5.5);
        p.drawLine(QPointF(8, 8), QPointF(11, 5));
        break;
    case ProposalChange::Glyph::Other:
        p.setPen(Qt::NoPen);
        p.setBrush(ink);
        p.drawEllipse(QPointF(8, 8), 2, 2);
        break;
    }
    p.restore();
}

} // namespace

ProposalChangeRow::ProposalChangeRow(const ProposalChange &change, QWidget *parent) : QWidget(parent), m_change(change) {
    setObjectName(QStringLiteral("proposalChange"));
    if (m_change.shortDescription.isEmpty()) m_change.shortDescription = m_change.description;
    setFixedHeight(kHeight);
    setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    setAccessibleName(change.detail.isEmpty() ? change.description : change.description + QStringLiteral(", ") + change.detail);
}

QFont ProposalChangeRow::descriptionFont() const {
    QFont font = this->font();
    font.setPixelSize(13);
    return font;
}

QFont ProposalChangeRow::detailFont() const {
    QFont font = this->font();
    font.setPixelSize(12);
    font.setFeature(QFont::Tag("tnum"), 1);
    return font;
}

ProposalChangeRow::Fit ProposalChangeRow::fit(int width) const {
    // The detail (an amount, a time) is never cut. The description beside
    // it, whole; else with its clips' extensions dropped; else the detail
    // takes a line of its own, and the description has the whole width.
    const QFontMetrics words(descriptionFont());
    const QFontMetrics detail(detailFont());
    const int room = std::max(0, width - kTextLeft);
    const auto wide = [&](const QString &text) { return words.horizontalAdvance(text) + 1; };
    Fit out;
    const QString &full = m_change.description;
    const QString &brief = m_change.shortDescription;
    const auto alone = [&] { return wide(full) <= room ? full : wide(brief) <= room ? brief : words.elidedText(brief, Qt::ElideRight, room); };
    if (m_change.detail.isEmpty()) {
        out.description = alone();
        return out;
    }
    out.detailWidth = detail.horizontalAdvance(m_change.detail) + 1;
    if (wide(full) + kDetailGap + out.detailWidth <= room) {
        out.description = full;
    } else if (wide(brief) + kDetailGap + out.detailWidth <= room) {
        out.description = brief;
    } else {
        out.twoLines = true;
        out.description = alone();
    }
    // Cut only where the detail alone is wider than the whole row.
    out.detail = out.detailWidth <= room ? m_change.detail : detail.elidedText(m_change.detail, Qt::ElideRight, room);
    out.detailWidth = std::min(out.detailWidth, room);
    return out;
}

void ProposalChangeRow::updateLines() {
    const Fit shown = fit(width());
    setFixedHeight(shown.twoLines ? kTwoLineHeight : kHeight);
    const bool cut = shown.description != m_change.description || shown.detail != m_change.detail;
    setToolTip(!cut ? QString() : m_change.detail.isEmpty() ? m_change.description : m_change.description + QStringLiteral(" · ") + m_change.detail);
}

void ProposalChangeRow::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    updateLines();
}

void ProposalChangeRow::changeEvent(QEvent *event) {
    QWidget::changeEvent(event);
    if (event->type() == QEvent::FontChange) updateLines();
}

QRect ProposalChangeRow::glyphRect() const {
    // On two lines the glyph stands beside the description's line.
    const int top = twoLines() ? (kTwoLineHeight - kDescriptionLine - kDetailLine) / 2 : (height() - kDescriptionLine) / 2;
    return QRect(0, top + (kDescriptionLine - 16) / 2, 16, 16);
}

QRect ProposalChangeRow::detailRect() const {
    const Fit shown = fit(width());
    if (m_change.detail.isEmpty()) return QRect(width(), 0, 0, height());
    if (shown.twoLines) {
        const int top = (kTwoLineHeight - kDescriptionLine - kDetailLine) / 2 + kDescriptionLine;
        return QRect(kTextLeft, top, std::max(0, width() - kTextLeft), kDetailLine);
    }
    return QRect(width() - shown.detailWidth, 0, shown.detailWidth, height());
}

QRect ProposalChangeRow::descriptionRect() const {
    const int left = kTextLeft;
    if (twoLines())
        return QRect(left, (kTwoLineHeight - kDescriptionLine - kDetailLine) / 2, std::max(0, width() - left), kDescriptionLine);
    return QRect(left, 0, std::max(0, detailRect().left() - kDetailGap - left), height());
}

void ProposalChangeRow::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    const Fit shown = fit(width());
    drawChangeGlyph(painter, glyphRect(), m_change.glyph, theme.chatSecondary);
    painter.setFont(descriptionFont());
    painter.setPen(theme.chatText);
    painter.drawText(descriptionRect(), Qt::AlignLeft | Qt::AlignVCenter, shown.description);
    if (!m_change.detail.isEmpty()) {
        painter.setFont(detailFont());
        painter.setPen(theme.chatSecondary);
        painter.drawText(detailRect(), (shown.twoLines ? Qt::AlignLeft : Qt::AlignRight) | Qt::AlignVCenter, shown.detail);
    }
}

// ---- SuggestionButton -----------------------------------------------------

namespace {

constexpr int kSuggestionGlyph = 16;
constexpr int kSuggestionGap = 8;
constexpr int kSuggestionPadding = 6;

QFont suggestionFont(const QFont &base) {
    QFont font = base;
    font.setPixelSize(13);
    return font;
}

} // namespace

SuggestionButton::SuggestionButton(const QString &text, QWidget *parent) : QAbstractButton(parent) {
    setText(text);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    QSizePolicy policy = sizePolicy();
    policy.setHeightForWidth(true);
    setSizePolicy(policy);
}

QRect SuggestionButton::textRect() const {
    const int left = 8 + kSuggestionGlyph + kSuggestionGap;
    return QRect(left, kSuggestionPadding, std::max(1, width() - left - 8), std::max(1, height() - 2 * kSuggestionPadding));
}

int SuggestionButton::heightForWidth(int width) const {
    // Its words on at most two lines, and never under 32 points.
    const QFontMetrics metrics(suggestionFont(font()));
    const int room = std::max(1, width - (8 + kSuggestionGlyph + kSuggestionGap) - 8);
    const int lines = std::min(2, std::max(1, metrics.boundingRect(QRect(0, 0, room, 10000), Qt::TextWordWrap, text()).height() /
                                                  std::max(1, metrics.lineSpacing())));
    return std::max(32, lines * metrics.lineSpacing() + 2 * kSuggestionPadding);
}

void SuggestionButton::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    if (underMouse() || isDown()) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(isDown() ? theme.chatChipHover : theme.chatChip);
        painter.drawRoundedRect(QRectF(rect()), 8, 8);
    }
    // An arrow pointing on, in secondary ink, level with the first line.
    const QFont font = suggestionFont(this->font());
    const QFontMetrics metrics(font);
    const QRect words = textRect();
    const qreal mid = words.top() + metrics.lineSpacing() / 2.0;
    painter.setPen(QPen(theme.chatSecondary, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    const qreal x = 8;
    painter.drawLine(QPointF(x + 3, mid), QPointF(x + 13, mid));
    painter.drawPolyline(QPolygonF{QPointF(x + 9, mid - 4), QPointF(x + 13, mid), QPointF(x + 9, mid + 4)});
    // The words, two lines at most, the second ending in an ellipsis if
    // they run on.
    painter.setFont(font);
    painter.setPen(theme.chatText);
    QTextLayout layout(text(), font);
    QTextOption option;
    option.setWrapMode(QTextOption::WordWrap);
    layout.setTextOption(option);
    layout.beginLayout();
    QStringList lines;
    for (int line = 0; line < 2; ++line) {
        QTextLine textLine = layout.createLine();
        if (!textLine.isValid()) break;
        textLine.setLineWidth(words.width());
        lines << text().mid(textLine.textStart(), line == 1 ? -1 : textLine.textLength());
    }
    layout.endLayout();
    for (int line = 0; line < lines.size(); ++line) {
        const QString shown = line == lines.size() - 1 ? metrics.elidedText(lines.at(line).trimmed(), Qt::ElideRight, words.width())
                                                        : lines.at(line).trimmed();
        painter.drawText(QRect(words.left(), words.top() + line * metrics.lineSpacing(), words.width(), metrics.lineSpacing()),
                         Qt::AlignLeft | Qt::AlignVCenter, shown);
    }
}

// ---- FlowLayout -----------------------------------------------------------

FlowLayout::FlowLayout(QWidget *parent, int spacing) : QLayout(parent), m_spacing(spacing) { setContentsMargins(0, 0, 0, 0); }

FlowLayout::~FlowLayout() {
    while (QLayoutItem *item = FlowLayout::takeAt(0)) delete item;
}

QLayoutItem *FlowLayout::takeAt(int index) { return index >= 0 && index < m_items.size() ? m_items.takeAt(index) : nullptr; }

void FlowLayout::setGeometry(const QRect &rect) {
    QLayout::setGeometry(rect);
    arrange(rect, true);
}

void FlowLayout::setScroll(int scroll) {
    scroll = std::max(0, scroll);
    if (scroll == m_scroll) return;
    m_scroll = scroll;
    if (!geometry().isNull()) arrange(geometry(), true);
}

int FlowLayout::contentWidth() const {
    int width = 0;
    for (const QLayoutItem *item : m_items)
        if (!item->isEmpty()) width += (width ? m_spacing : 0) + item->sizeHint().width();
    return width;
}

QSize FlowLayout::minimumSize() const {
    if (m_singleLine) {
        // As tall as its tallest chip, and as narrow as anything.
        int height = 0;
        for (const QLayoutItem *item : m_items)
            if (!item->isEmpty()) height = std::max(height, item->sizeHint().height());
        return QSize(0, height).grownBy(contentsMargins());
    }
    // As tall as its tallest chip, and as narrow as its column: a chip
    // wider than the column is narrowed to it (and ends in an ellipsis),
    // never widens it.
    int height = 0;
    for (const QLayoutItem *item : m_items)
        if (!item->isEmpty()) height = std::max(height, item->sizeHint().height());
    return QSize(0, height).grownBy(contentsMargins());
}

int FlowLayout::arrange(const QRect &rect, bool apply) const {
    const QRect area = rect.marginsRemoved(contentsMargins());
    if (m_singleLine) {
        int x = area.x() - m_scroll;
        int line = 0;
        for (QLayoutItem *item : m_items) {
            if (item->isEmpty()) continue;
            const QSize hint = item->sizeHint();
            if (apply) item->setGeometry(QRect(QPoint(x, area.y()), hint));
            x += hint.width() + m_spacing;
            line = std::max(line, hint.height());
        }
        return area.y() + line - rect.y() + contentsMargins().bottom();
    }
    int x = area.x();
    int y = area.y();
    int line = 0;
    for (QLayoutItem *item : m_items) {
        if (item->isEmpty()) continue;
        QSize hint = item->sizeHint();
        hint.setWidth(std::min(hint.width(), std::max(1, area.width())));
        if (x > area.x() && x + hint.width() > area.x() + area.width()) {
            x = area.x();
            y += line + m_spacing;
            line = 0;
        }
        if (apply) item->setGeometry(QRect(QPoint(x, y), hint));
        x += hint.width() + m_spacing;
        line = std::max(line, hint.height());
    }
    return y + line - rect.y() + contentsMargins().bottom();
}

// ---- ChatAvatar -----------------------------------------------------------

ChatAvatar::ChatAvatar(Who who, int diameter, QWidget *parent) : QWidget(parent), m_who(who) {
    setObjectName(who == Who::Kerf ? QStringLiteral("chatAvatarKerf") : QStringLiteral("chatAvatarUser"));
    setFixedSize(diameter, diameter);
    setAttribute(Qt::WA_TransparentForMouseEvents);
}

QString ChatAvatar::monogram() const {
    if (m_who == Who::Kerf) return QStringLiteral("K");
    // The reader's initial, from the account they are signed in to here.
    QString name = qEnvironmentVariable("USER").trimmed();
    if (name.isEmpty()) name = qEnvironmentVariable("USERNAME").trimmed();
    return name.isEmpty() ? QStringLiteral("Y") : name.left(1).toUpper();
}

void ChatAvatar::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    const bool kerf = m_who == Who::Kerf;
    painter.setBrush(kerf ? theme.chatAvatarKerf : theme.chatAvatarUser);
    painter.drawEllipse(QRectF(rect()));
    QFont font = painter.font();
    font.setPixelSize(std::max(10, int(std::round(width() * 0.46))));
    font.setWeight(QFont::DemiBold);
    painter.setFont(font);
    painter.setPen(kerf ? theme.chatAvatarKerfText : theme.chatAvatarUserText);
    painter.drawText(rect(), Qt::AlignCenter, monogram());
}

// ---- ChatIconButton -------------------------------------------------------

ChatIconButton::ChatIconButton(Glyph glyph, int diameter, QWidget *parent)
    : QAbstractButton(parent), m_glyph(glyph), m_diameter(diameter) {
    setFixedSize(diameter, diameter);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setProperty("bezelRadius", diameter / 2);
}

QSize ChatIconButton::sizeHint() const { return {m_diameter, m_diameter}; }

void ChatIconButton::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    if (m_filled) {
        painter.setBrush(theme.chatSurface);
        painter.drawEllipse(QRectF(rect()));
    }
    if (isDown() || underMouse()) {
        painter.setBrush(isDown() ? theme.chatChipHover : theme.chatChip);
        painter.drawEllipse(QRectF(rect()));
    }
    const QPointF c = QRectF(rect()).center();
    const QColor ink = isEnabled() ? theme.chatText : theme.textDisabled;
    if (m_glyph == Glyph::Close) {
        painter.setPen(QPen(ink, 1.6, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(c + QPointF(-6, -6), c + QPointF(6, 6));
        painter.drawLine(c + QPointF(6, -6), c + QPointF(-6, 6));
    } else if (m_glyph == Glyph::Back || m_glyph == Glyph::Forward) {
        const double turn = m_glyph == Glyph::Back ? -1.0 : 1.0;
        painter.setPen(QPen(ink, 1.6, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawPolyline(QPolygonF{c + QPointF(-2.25 * turn, -4.5), c + QPointF(2.25 * turn, 0), c + QPointF(-2.25 * turn, 4.5)});
    } else {
        painter.setPen(Qt::NoPen);
        painter.setBrush(ink);
        for (int dot = -1; dot <= 1; ++dot) painter.drawEllipse(c + QPointF(dot * 6.0, 0), 1.8, 1.8);
    }
}

// ---- ChatExpander ---------------------------------------------------------

ChatExpander::ChatExpander(QWidget *parent) : QAbstractButton(parent) {
    setObjectName(QStringLiteral("chatExpander"));
    setCheckable(true);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setProperty("bezelRadius", 14);
    QFont bold = font();
    bold.setPixelSize(13);
    bold.setWeight(QFont::DemiBold);
    setFont(bold);
}

QSize ChatExpander::sizeHint() const {
    // A chevron, 6 points, then the label, in a 28-point pill; compact, the
    // chevron at its leading edge, 4 points, the label, 4 points.
    if (m_compact) return {12 + 4 + fontMetrics().horizontalAdvance(text()) + 4, 28};
    return {10 + 12 + 6 + fontMetrics().horizontalAdvance(text()) + 12, 28};
}

QSize ChatExpander::minimumSizeHint() const { return m_compact ? QSize(20, 28) : sizeHint(); }

void ChatExpander::setCompact(bool compact) {
    if (m_compact == compact) return;
    m_compact = compact;
    QFont face = font();
    face.setPixelSize(compact ? 12 : 13);
    face.setWeight(compact ? QFont::Medium : QFont::DemiBold);
    setFont(face);
    setSizePolicy(compact ? QSizePolicy::Preferred : QSizePolicy::Minimum, QSizePolicy::Fixed);
    updateGeometry();
    update();
}

bool ChatExpander::labelShown() const { return !m_compact || width() >= sizeHint().width(); }

void ChatExpander::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    if (isDown() || underMouse()) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(theme.chatChip);
        painter.drawRoundedRect(QRectF(rect()), height() / 2.0, height() / 2.0);
    }
    const QColor ink = theme.chatExpander;
    // The chevron points down while folded and up while open.
    const double lead = m_compact ? 0.0 : 10.0;
    const QPointF c(lead + 6, height() / 2.0);
    const qreal turn = isChecked() ? -1.0 : 1.0;
    painter.setPen(QPen(ink, 1.6, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.drawPolyline(QPolygonF{c + QPointF(-4.5, -2.25 * turn), c + QPointF(0, 2.25 * turn), c + QPointF(4.5, -2.25 * turn)});
    if (!labelShown()) return;
    painter.setFont(font());
    const double textLeft = lead + 12 + (m_compact ? 4 : 6);
    painter.drawText(QRectF(textLeft, 0, width() - textLeft, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
}

// ---- CommentRow -----------------------------------------------------------

CommentRow::CommentRow(Kind kind, const QString &text, const QDateTime &time, QWidget *parent)
    : QWidget(parent), m_kind(kind), m_text(text), m_time(time) {
    setObjectName(QStringLiteral("commentRow"));
    auto *row = new QHBoxLayout(this);
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(12);
    m_avatar = new ChatAvatar(kind == Kind::User ? ChatAvatar::Who::User : ChatAvatar::Who::Kerf, ChatPanel::kAvatar);
    row->addWidget(m_avatar, 0, Qt::AlignTop);
    auto *column = new QVBoxLayout;
    column->setContentsMargins(0, 0, 0, 0);
    column->setSpacing(0);
    row->addLayout(column, 1);

    // The name line, 18 points: the author, then the time.
    auto *line = new QHBoxLayout;
    line->setContentsMargins(0, 0, 0, 0);
    line->setSpacing(4);
    m_nameLabel = label(name(), "chatName");
    m_timeLabel = label(QString(), "chatTime");
    line->addWidget(m_nameLabel);
    if (kind == Kind::Error) {
        // Short, and able to give way, so the name line fits the panel at
        // its narrowest.
        QLabel *tag = label(QStringLiteral("· couldn’t edit"), "chatErrorTag");
        tag->setMinimumWidth(1);
        line->addWidget(tag);
    }
    line->addWidget(m_timeLabel);
    line->addStretch(1);
    column->addLayout(line);
    column->addSpacing(2);
    m_body = label(QString(), "chatBody");
    m_body->setTextFormat(Qt::RichText);
    m_body->setText(bodyMarkup(text));
    m_body->setWordWrap(true);
    m_body->setTextInteractionFlags(Qt::TextSelectableByMouse);
    copiesCleanly(m_body);
    m_body->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    m_body->setVisible(!text.isEmpty());
    column->addWidget(m_body);

    // Kerf's steps for this turn, folded under "▸ 2 steps", as replies are.
    m_expander = new ChatExpander;
    m_expander->hide();
    connect(m_expander, &QAbstractButton::toggled, this, [this](bool open) {
        m_replyList->setVisible(open);
        updateExpander();
    });
    m_replyList = new QWidget;
    m_replyList->setObjectName(QStringLiteral("chatReplies"));
    m_replyLayout = new QVBoxLayout(m_replyList);
    m_replyLayout->setContentsMargins(0, 4, 0, 4);
    m_replyLayout->setSpacing(8);
    m_replyList->hide();

    // The quiet actions under the text.
    auto *actions = new QHBoxLayout;
    actions->setContentsMargins(0, 2, 0, 0);
    actions->setSpacing(2);
    auto *copy = pill(QStringLiteral("Copy"), "chatAction", 24);
    copy->setToolTip(QStringLiteral("Copy this message"));
    connect(copy, &QPushButton::clicked, this, [this] { QApplication::clipboard()->setText(m_text); });
    m_actions.append(copy);
    if (kind == Kind::User) {
        auto *again = pill(QStringLiteral("Ask again"), "chatAction", 24);
        again->setToolTip(QStringLiteral("Send this request again"));
        connect(again, &QPushButton::clicked, this, [this] { emit askAgain(m_text); });
        m_actions.append(again);
    }
    for (QPushButton *action : std::as_const(m_actions)) actions->addWidget(action);
    actions->addStretch(1);
    column->addLayout(actions);
    column->addWidget(m_expander, 0, Qt::AlignLeft);
    column->addWidget(m_replyList);
    refreshTime(time);
}

QString CommentRow::name() const {
    return m_kind == Kind::User ? QStringLiteral("You") : QStringLiteral("Kerf");
}

void CommentRow::setText(const QString &text) {
    m_text = text;
    m_body->setText(bodyMarkup(text));
    m_body->setVisible(!text.isEmpty());
}

void CommentRow::appendText(const QString &text) { setText(m_text + text); }

void CommentRow::refreshTime(const QDateTime &now) { m_timeLabel->setText(relativeTime(m_time, now)); }

void CommentRow::addReply(const QString &text) {
    if (text.isEmpty()) return;
    m_replies.append(text);
    auto *reply = new QWidget;
    reply->setObjectName(QStringLiteral("chatReply"));
    auto *layout = new QHBoxLayout(reply);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(8);
    layout->addWidget(new ChatAvatar(ChatAvatar::Who::Kerf, 20), 0, Qt::AlignTop);
    QLabel *body = label(wrappable(text.simplified()), "chatReplyText");
    body->setWordWrap(true);
    body->setTextInteractionFlags(Qt::TextSelectableByMouse);
    copiesCleanly(body);
    body->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    layout->addWidget(body, 1);
    m_replyLayout->addWidget(reply);
    m_expander->show();
    updateExpander();
}

bool CommentRow::repliesExpanded() const { return m_expander->isChecked(); }

void CommentRow::setRepliesExpanded(bool expanded) { m_expander->setChecked(expanded); }

void CommentRow::updateExpander() {
    const qsizetype count = m_replies.size();
    m_expander->setText(QStringLiteral("%1 step%2").arg(count).arg(count == 1 ? QString() : QStringLiteral("s")));
    m_expander->setToolTip(m_expander->isChecked() ? QStringLiteral("Hide what Kerf did") : QStringLiteral("Show what Kerf did"));
    m_expander->updateGeometry();
    m_expander->update();
}

// ---- RailButton -----------------------------------------------------------

RailButton::RailButton(Glyph glyph, QWidget *parent) : QAbstractButton(parent), m_glyph(glyph) {
    setObjectName(QStringLiteral("railButton"));
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setProperty("bezelRadius", kDisc / 2);
    QFont small = font();
    small.setPixelSize(12);
    small.setWeight(QFont::Medium);
    setFont(small);
}

void RailButton::setGlyph(Glyph glyph) {
    if (glyph == m_glyph) return;
    m_glyph = glyph;
    update();
}

void RailButton::setLabel(const QString &label) {
    if (label == m_label) return;
    m_label = label;
    update();
}

QRect RailButton::discRect() const { return QRect((width() - kDisc) / 2, 0, kDisc, kDisc); }

QSize RailButton::sizeHint() const { return {StageRail::kWidth, kDisc + 4 + 16}; }

void RailButton::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    const QRectF disc = discRect();
    painter.setBrush(isDown() || underMouse() ? theme.railButtonHover : isChecked() ? theme.railButtonActive : theme.railButton);
    painter.drawEllipse(disc);
    // A device switched off (the plain card) keeps its name and its
    // glyph, in secondary ink with a slash through it, as the menu says
    // "Laptop · Off".
    const bool device = m_glyph == Glyph::Phone || m_glyph == Glyph::Laptop;
    const bool off = device && isCheckable() && !isChecked();
    const QColor ink = isChecked() ? theme.railGlyphActive : off ? theme.textSecondary : theme.railGlyph;
    painter.save();
    painter.translate(disc.center() - QPointF(12, 12));
    painter.setPen(QPen(ink, 1.7, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    switch (m_glyph) {
    case Glyph::Chat: {
        // The speech bubble on a 24-point grid: a rounded bubble, its tail
        // at the lower left; filled while the chat is open.
        QPainterPath path;
        path.moveTo(7.5, 4.5); path.lineTo(16.5, 4.5);
        path.quadTo(20.5, 4.5, 20.5, 8.5); path.lineTo(20.5, 12.5);
        path.quadTo(20.5, 16.5, 16.5, 16.5); path.lineTo(11.5, 16.5);
        path.lineTo(6.5, 20); path.lineTo(7.3, 16.5);
        path.quadTo(3.5, 16.3, 3.5, 12.5); path.lineTo(3.5, 8.5);
        path.quadTo(3.5, 4.5, 7.5, 4.5);
        path.closeSubpath();
        painter.setBrush(isChecked() ? QBrush(ink) : QBrush(Qt::NoBrush));
        painter.drawPath(path);
        break;
    }
    case Glyph::Phone:
        // A phone's outline and its camera.
        painter.drawRoundedRect(QRectF(7, 3, 10, 18), 2.5, 2.5);
        painter.drawLine(QPointF(10.8, 5.8), QPointF(13.2, 5.8));
        break;
    case Glyph::Laptop:
        // A laptop's lid, and its wider base under it.
        painter.drawRoundedRect(QRectF(5, 5.5, 14, 10), 1.5, 1.5);
        painter.drawLine(QPointF(3, 18.5), QPointF(21, 18.5));
        break;
    case Glyph::SafeZones: {
        // A frame's corners, and the dashed line inside them.
        const auto corner = [&painter](QPointF at, double dx, double dy) {
            painter.drawPolyline(QPolygonF{at + QPointF(0, 4 * dy), at, at + QPointF(4 * dx, 0)});
        };
        corner({4, 4}, 1, 1);
        corner({20, 4}, -1, 1);
        corner({4, 20}, 1, -1);
        corner({20, 20}, -1, -1);
        QPen dashed(ink, 1.4, Qt::CustomDashLine, Qt::FlatCap);
        dashed.setDashPattern({2.0, 2.0});
        painter.setPen(dashed);
        painter.drawRect(QRectF(8, 8, 8, 8));
        break;
    }
    }
    if (off) {
        // The slash, over a gap of the disc's colour so it reads across the glyph.
        painter.setPen(QPen(isDown() || underMouse() ? theme.railButtonHover : theme.railButton, 3.5, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(QPointF(4.5, 19.5), QPointF(19.5, 4.5));
        painter.setPen(QPen(ink, 1.5, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(QPointF(4.5, 19.5), QPointF(19.5, 4.5));
    }
    painter.restore();
    if (!m_label.isEmpty()) {
        painter.setPen(theme.railLabel);
        painter.setFont(font());
        painter.drawText(QRectF(0, disc.bottom() + 4, width(), 16), Qt::AlignHCenter | Qt::AlignTop, m_label);
    }
}

// ---- StageRail ------------------------------------------------------------

StageRail::StageRail(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageRail"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(12);
    layout->addStretch(1);
    m_device = new RailButton(RailButton::Glyph::Phone);
    m_device->setObjectName(QStringLiteral("railDevice"));
    m_device->setCheckable(true);
    m_device->setCheckedByOwner(true);
    m_device->setLabel(QStringLiteral("Phone"));
    m_safeZones = new RailButton(RailButton::Glyph::SafeZones);
    m_safeZones->setObjectName(QStringLiteral("railSafeZones"));
    m_safeZones->setCheckable(true);
    m_safeZones->setLabel(QStringLiteral("Safe"));
    m_chat = new RailButton(RailButton::Glyph::Chat);
    m_chat->setCheckable(true);
    for (RailButton *button : {m_device, m_safeZones, m_chat}) {
        button->setFixedSize(button->sizeHint());
        layout->addWidget(button, 0, Qt::AlignHCenter);
    }
}

// ---- ChatPanel ------------------------------------------------------------

ChatPanel::ChatPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("chatPanel"));
    setAccessibleName(QStringLiteral("Conversation with Kerf"));
    // The composer and the proposal show their fields' focus themselves.
    setProperty("ringsOwnFields", true);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(0, 0, 0, 0);
    outer->setSpacing(0);
    outer->addWidget(buildHeader());
    // While the pinned proposal's pills are scrolled out of view: a compact
    // bar under the header with its summary, Reject and Accept.
    m_proposalBar = buildProposalBar();
    outer->addWidget(m_proposalBar);

    // A hairline under the header (or the bar) while the view is scrolled
    // away from its top, as a comments panel's header shows: the comments
    // are cut cleanly at the view's edge under it.
    m_scrollEdge = new QFrame;
    m_scrollEdge->setObjectName(QStringLiteral("scrollEdge"));
    m_scrollEdge->setFixedHeight(1);
    outer->addWidget(m_scrollEdge);

    m_scroll = new QScrollArea;
    m_scroll->setObjectName(QStringLiteral("chatScroll"));
    m_scroll->setFrameShape(QFrame::NoFrame);
    m_scroll->setWidgetResizable(true);
    m_scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    // It gives way first when the panel is short, so the composer stays
    // (a minimum of 0 would mean its own hint).
    m_scroll->setMinimumHeight(1);
    m_scroll->viewport()->setAutoFillBackground(false);
    m_content = new QWidget;
    m_content->setObjectName(QStringLiteral("chatContent"));
    m_list = new QVBoxLayout(m_content);
    // The first comment 16 points under the header, as a comments panel's.
    // Its foot is the view's: the view stops kListBottom over the chips or
    // the composer, so rows are cut there at every scroll position.
    m_list->setContentsMargins(kSidePadding, 16, kSidePadding, 0);
    m_list->setSpacing(16);
    // The pinned proposal heads the list and scrolls away with it, as a
    // pinned comment does.
    m_proposal = buildProposal();
    m_list->addWidget(m_proposal);
    m_welcome = buildWelcome();
    m_list->addWidget(m_welcome);
    // While Kerf works: three dots in the avatars' column, its state and
    // its latest step, quietly, after the latest comment.
    m_statusRow = new QWidget;
    m_statusRow->setObjectName(QStringLiteral("chatStatusRow"));
    auto *status = new QHBoxLayout(m_statusRow);
    status->setContentsMargins(0, 0, 0, 0);
    status->setSpacing(12);
    m_typing = new TypingDots;
    status->addWidget(m_typing);
    m_status = label(QStringLiteral("Ready"), "chatStatus");
    status->addWidget(m_status);
    m_activity = label(QString(), "chatActivity");
    m_activity->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    status->addWidget(m_activity, 1);
    m_statusRow->hide();
    m_list->addWidget(m_statusRow);
    m_list->addStretch(1);
    m_scroll->setWidget(m_content);
    m_content->installEventFilter(this);
    outer->addWidget(m_scroll, 1);
    outer->addSpacing(kListBottom);
    // New output keeps the latest in view, unless the reader scrolled away.
    QScrollBar *bar = m_scroll->verticalScrollBar();
    connect(bar, &QScrollBar::rangeChanged, this, [this](int, int maximum) {
        if (m_follow) m_scroll->verticalScrollBar()->setValue(maximum);
        QTimer::singleShot(0, this, &ChatPanel::updateListEdge);
    });
    m_scroll->viewport()->installEventFilter(this);
    connect(bar, &QScrollBar::valueChanged, this, [this](int value) {
        m_follow = value >= m_scroll->verticalScrollBar()->maximum() - 4;
        updateListEdge();
    });
    m_edgeCover = new EdgeCover(m_scroll->viewport());

    // Next steps: one row of chips over the composer that scrolls sideways
    // rather than wrap, as a chip bar does, so the conversation keeps the
    // panel's height.
    m_chips = new QWidget;
    m_chips->setObjectName(QStringLiteral("nextSteps"));
    auto *chips = new FlowLayout(m_chips, 8);
    chips->setSingleLine(true);
    chips->setContentsMargins(kSidePadding, 8, kSidePadding, 4);
    m_chips->setFixedHeight(kChipRowHeight);
    m_chipFadeLeft = new ChipFade(true, m_chips);
    m_chipFadeRight = new ChipFade(false, m_chips);
    // Round buttons over the fades that page the row, as a chip bar's.
    for (const bool back : {true, false}) {
        auto *pager = new ChatIconButton(back ? ChatIconButton::Glyph::Back : ChatIconButton::Glyph::Forward, kChipPager, m_chips);
        pager->setObjectName(back ? QStringLiteral("chipsBack") : QStringLiteral("chipsForward"));
        pager->setFilled(true);
        pager->setToolTip(back ? QStringLiteral("Earlier suggestions") : QStringLiteral("More suggestions"));
        pager->setAccessibleName(pager->toolTip());
        pager->hide();
        connect(pager, &QAbstractButton::clicked, this, [this, back] { pageChips(back ? -1 : 1); });
        (back ? m_chipBack : m_chipForward) = pager;
    }
    m_chips->installEventFilter(this);
    m_chips->hide();
    outer->addWidget(m_chips);
    m_composer = buildComposer();
    outer->addWidget(m_composer);

    m_resizeHandle = new QWidget(this);
    m_resizeHandle->setObjectName(QStringLiteral("chatResizeHandle"));
    m_resizeHandle->setCursor(Qt::SizeHorCursor);
    m_resizeHandle->setToolTip(QStringLiteral("Drag to resize the conversation"));
    m_resizeHandle->installEventFilter(this);

    m_clock = new QTimer(this);
    m_clock->setInterval(30000);
    connect(m_clock, &QTimer::timeout, this, &ChatPanel::refreshTimes);
    m_clock->start();
    updateCount();
    setProposalShown(false);
    updateComposer(false);
}

QWidget *ChatPanel::buildHeader() {
    auto *header = new QWidget;
    header->setObjectName(QStringLiteral("chatHeader"));
    header->setFixedHeight(kHeaderHeight);
    auto *layout = new QHBoxLayout(header);
    layout->setContentsMargins(kSidePadding, 0, 8, 0);
    layout->setSpacing(0);
    m_title = label(QStringLiteral("Kerf"), "chatTitle");
    layout->addWidget(m_title);
    layout->addSpacing(8);
    m_count = label(QString(), "chatCount");
    layout->addWidget(m_count);
    layout->addStretch(1);
    m_more = new ChatIconButton(ChatIconButton::Glyph::More, 36);
    m_more->setObjectName(QStringLiteral("chatMore"));
    m_more->setToolTip(QStringLiteral("More"));
    m_more->setAccessibleName(QStringLiteral("More"));
    layout->addWidget(m_more);
    m_close = new ChatIconButton(ChatIconButton::Glyph::Close, 36);
    m_close->setObjectName(QStringLiteral("chatClose"));
    m_close->setAccessibleName(QStringLiteral("Close Kerf Assistant"));
    layout->addWidget(m_close);
    return header;
}

QFrame *ChatPanel::buildProposal() {
    auto *box = new QFrame;
    box->setObjectName(QStringLiteral("proposalBox"));
    auto *layout = new QVBoxLayout(box);
    // No rule under it: the list's own 16 points under it part them, as
    // threads are parted; the list gives it its side padding.
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(6);
    // "Pinned by Kerf" over the comment, its pin in the avatars' column.
    m_proposalPinLine = new QWidget;
    m_proposalPinLine->setObjectName(QStringLiteral("proposalPinLine"));
    auto *pinned = new QHBoxLayout(m_proposalPinLine);
    pinned->setContentsMargins(kAvatar - 12, 0, 0, 0);
    pinned->setSpacing(12);
    pinned->addWidget(new PinGlyph, 0, Qt::AlignVCenter);
    pinned->addWidget(label(QStringLiteral("Pinned by Kerf"), "chatPinned"));
    pinned->addStretch(1);
    layout->addWidget(m_proposalPinLine);
    auto *row = new QHBoxLayout;
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(12);
    row->addWidget(new ChatAvatar(ChatAvatar::Who::Kerf, kAvatar), 0, Qt::AlignTop);
    auto *column = new QVBoxLayout;
    column->setContentsMargins(0, 0, 0, 0);
    column->setSpacing(4);
    // The name line every comment has, pinned or not: the author, then the time.
    auto *name = new QHBoxLayout;
    name->setContentsMargins(0, 0, 0, 0);
    name->setSpacing(4);
    m_proposalName = label(QStringLiteral("Kerf"), "chatName");
    m_proposalTime = label(QStringLiteral("now"), "chatTime");
    name->addWidget(m_proposalName);
    name->addWidget(m_proposalTime);
    name->addStretch(1);
    column->addLayout(name);
    // What it changes, and the expander for its script.
    auto *line = new QHBoxLayout;
    line->setContentsMargins(0, 0, 0, 0);
    line->setSpacing(8);
    m_proposalSummary = label(QStringLiteral("No pending changes"), "proposalSummary");
    m_proposalSummary->setWordWrap(true);
    m_proposalSummary->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    line->addWidget(m_proposalSummary, 1);
    m_proposalExpander = new ChatExpander;
    m_proposalExpander->setText(QStringLiteral("Script"));
    m_proposalExpander->setToolTip(QStringLiteral("Show the script and the document diff"));
    line->addWidget(m_proposalExpander, 0, Qt::AlignVCenter);
    m_proposalLine = line;
    column->addLayout(line);
    // The first few changes, one row each: glyph, description, detail.
    m_proposalChanges = new QWidget;
    m_proposalChanges->setObjectName(QStringLiteral("proposalChanges"));
    auto *changes = new QVBoxLayout(m_proposalChanges);
    changes->setContentsMargins(0, 2, 0, 4);
    changes->setSpacing(0);
    m_proposalChanges->hide();
    column->addWidget(m_proposalChanges);
    m_proposalDiff = new QTextEdit;
    m_proposalDiff->setReadOnly(true);
    m_proposalDiff->setObjectName(QStringLiteral("proposalDiff"));
    // As tall as its lines, up to ten (then it scrolls), so the request and
    // reply under the pinned proposal stay in view.
    m_proposalDiff->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_proposalDiff->setLineWrapMode(QTextEdit::WidgetWidth);
    m_proposalDiff->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
    m_proposalDiff->setFrameShape(QFrame::NoFrame);
    m_proposalDiff->hide();
    connect(m_proposalDiff->document()->documentLayout(), &QAbstractTextDocumentLayout::documentSizeChanged, this,
            &ChatPanel::fitProposalDiff);
    connect(m_proposalExpander, &QAbstractButton::toggled, this, &ChatPanel::fitProposalDiff);
    // Reject, then Accept, the default, trailing as a comment field's
    // Comment does.
    auto *buttons = new QHBoxLayout;
    buttons->setContentsMargins(0, 0, 0, 0);
    buttons->setSpacing(8);
    buttons->addStretch(1);
    m_proposalButtons = buttons;
    m_reject = pill(QStringLiteral("Reject"), "chatPillTonal", kPillHeight);
    m_accept = pill(QStringLiteral("Accept"), "chatPillFilled", kPillHeight);
    // The compact bar's pills follow these ones' state.
    m_accept->installEventFilter(this);
    m_reject->installEventFilter(this);
    buttons->addWidget(m_reject);
    buttons->addWidget(m_accept);
    column->addLayout(buttons);
    column->addWidget(m_proposalDiff);
    row->addLayout(column, 1);
    layout->addLayout(row);
    // A hairline closes the pinned comment, 16 points under its pills, as
    // a pinned comment is parted from the thread under it.
    layout->addSpacing(16 - layout->spacing());
    m_proposalRule = new QFrame;
    m_proposalRule->setObjectName(QStringLiteral("proposalRule"));
    m_proposalRule->setFixedHeight(1);
    layout->addWidget(m_proposalRule);
    return box;
}

QFrame *ChatPanel::buildProposalBar() {
    auto *bar = new QFrame;
    bar->setObjectName(QStringLiteral("proposalBar"));
    auto *layout = new QHBoxLayout(bar);
    layout->setContentsMargins(kSidePadding, 8, kSidePadding, 8);
    layout->setSpacing(8);
    // The pin, the summary (cut with an ellipsis in a narrow panel), then
    // the pills.
    layout->addWidget(new PinGlyph, 0, Qt::AlignVCenter);
    m_proposalBarSummary = new ElidedLabel;
    m_proposalBarSummary->setObjectName(QStringLiteral("proposalBarSummary"));
    m_proposalBarSummary->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    layout->addWidget(m_proposalBarSummary, 1);
    m_barReject = pill(QStringLiteral("Reject"), "chatPillTonal", kComposerPillHeight);
    m_barAccept = pill(QStringLiteral("Accept"), "chatPillFilled", kComposerPillHeight);
    for (QPushButton *button : {m_barReject, m_barAccept}) layout->addWidget(button);
    // They are the pinned proposal's own: a click is theirs.
    connect(m_barReject, &QPushButton::clicked, this, [this] { m_reject->click(); });
    connect(m_barAccept, &QPushButton::clicked, this, [this] { m_accept->click(); });
    bar->hide();
    return bar;
}

void ChatPanel::setProposalSummary(const QString &summary, const QString &compact, const QString &count) {
    m_summaryFull = summary;
    m_summaryShort = compact;
    m_proposalSummary->setText(summary);
    placeProposalExpander();
    const QString bar = compact.isEmpty() ? summary : compact;
    static_cast<ElidedLabel *>(m_proposalBarSummary)->setFullText(bar, count);
    m_proposalBarSummary->setToolTip(summary);
}

QWidget *ChatPanel::buildWelcome() {
    // Before the first message, Kerf's own first comment: what it does, and
    // two requests to try, which fill the composer.
    auto *welcome = new QWidget;
    welcome->setObjectName(QStringLiteral("chatWelcome"));
    auto *row = new QHBoxLayout(welcome);
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(12);
    row->addWidget(new ChatAvatar(ChatAvatar::Who::Kerf, kAvatar), 0, Qt::AlignTop);
    auto *column = new QVBoxLayout;
    column->setContentsMargins(0, 0, 0, 0);
    column->setSpacing(2);
    // The name line every comment has: the author, then the time.
    auto *line = new QHBoxLayout;
    line->setContentsMargins(0, 0, 0, 0);
    line->setSpacing(4);
    line->addWidget(label(QStringLiteral("Kerf"), "chatName"));
    // It ages as comments do, from when the conversation opened.
    m_welcomeSince = QDateTime::currentDateTime();
    m_welcomeTime = label(relativeTime(m_welcomeSince, m_welcomeSince), "chatTime");
    line->addWidget(m_welcomeTime);
    line->addStretch(1);
    column->addLayout(line);
    QLabel *intro = label(QString(), "chatBody");
    intro->setTextFormat(Qt::RichText);
    intro->setText(bodyMarkup(QStringLiteral("Describe the change you want. I’ll propose it, and you review it before it touches the cut.")));
    intro->setWordWrap(true);
    intro->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    column->addWidget(intro);
    column->addSpacing(10);
    column->addWidget(label(QStringLiteral("Try asking"), "chatSection"));
    column->addSpacing(4);
    // Each as a row of plain words after an arrow, wrapping to two lines,
    // as a list of suggestions: no box, a wash under the pointer.
    auto *examples = new QWidget;
    examples->setObjectName(QStringLiteral("chatExamples"));
    auto *list = new QVBoxLayout(examples);
    list->setContentsMargins(0, 0, 0, 0);
    list->setSpacing(0);
    for (const QString &example : {QStringLiteral("Trim the opening to three seconds."),
                                   QStringLiteral("Lower the music under the dialogue.")}) {
        auto *suggestion = new SuggestionButton(example);
        suggestion->setObjectName(QStringLiteral("chatExample"));
        suggestion->setToolTip(QStringLiteral("Write this in the composer"));
        suggestion->setAccessibleName(example);
        connect(suggestion, &QAbstractButton::clicked, this, [this, example] {
            if (!m_input->isEnabled()) return;
            m_input->setPlainText(example);
            m_input->setFocus(Qt::OtherFocusReason);
            m_input->moveCursor(QTextCursor::End);
        });
        list->addWidget(suggestion);
        m_examples.append(suggestion);
    }
    column->addWidget(examples);
    row->addLayout(column, 1);
    return welcome;
}

QFrame *ChatPanel::buildComposer() {
    auto *composer = new QFrame;
    composer->setObjectName(QStringLiteral("chatComposer"));
    auto *layout = new QVBoxLayout(composer);
    layout->setContentsMargins(kSidePadding, 10, kSidePadding, 10);
    layout->setSpacing(8);
    auto *row = new QHBoxLayout;
    row->setContentsMargins(0, 0, 0, 0);
    row->setSpacing(12);
    row->addWidget(new ChatAvatar(ChatAvatar::Who::User, kComposerAvatar), 0, Qt::AlignTop);
    auto *field = new QVBoxLayout;
    field->setContentsMargins(0, 0, 0, 0);
    field->setSpacing(4);
    m_input = new QTextEdit;
    m_input->setObjectName(QStringLiteral("chatInput"));
    m_input->setAccessibleName(QStringLiteral("Message Kerf"));
    m_input->setAcceptRichText(false);
    m_input->setFrameShape(QFrame::NoFrame);
    m_input->document()->setDocumentMargin(0);
    m_input->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_input->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_input->setTabChangesFocus(true);
    m_input->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    connect(m_input->document()->documentLayout(), &QAbstractTextDocumentLayout::documentSizeChanged, this, &ChatPanel::fitInput);
    field->addWidget(m_input);
    field->addWidget(new ComposerRule);
    row->addLayout(field, 1);
    layout->addLayout(row);

    // Shown while the composer is in use: the model, a hint, Cancel, Send
    // (or Stop while Kerf works), as a comment field shows its buttons.
    m_composerActions = new QWidget;
    m_composerActions->setObjectName(QStringLiteral("chatComposerActions"));
    auto *actions = new QHBoxLayout(m_composerActions);
    actions->setContentsMargins(kComposerAvatar + 12 - 4, 0, 0, 0);
    actions->setSpacing(8);
    m_model = new QComboBox;
    m_model->setEditable(true);
    m_model->setInsertPolicy(QComboBox::NoInsert);
    m_model->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    m_model->setMinimumContentsLength(6);
    m_model->setObjectName(QStringLiteral("agentModel"));
    m_model->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    // It gives way to Cancel and Send in a narrow panel.
    m_model->setMinimumWidth(64);
    actions->addWidget(m_model, 0);
    actions->addStretch(1);
    m_cancel = pill(QStringLiteral("Cancel"), "chatPillText", kComposerPillHeight);
    m_cancel->setToolTip(QStringLiteral("Clear the draft"));
    connect(m_cancel, &QPushButton::clicked, this, [this] {
        m_input->clear();
        m_input->clearFocus();
        updateComposer(m_busy);
    });
    actions->addWidget(m_cancel);
    m_stop = pill(QStringLiteral("Stop"), "chatPillTonal", kComposerPillHeight);
    m_stop->setObjectName(QStringLiteral("abortButton"));
    m_stop->setToolTip(QStringLiteral("Stop the current agent request"));
    actions->addWidget(m_stop);
    m_send = pill(QStringLiteral("Send"), "chatPillFilled", kComposerPillHeight);
    m_send->setObjectName(QStringLiteral("chatSendButton"));
    m_send->setAccessibleName(QStringLiteral("Send message"));
    m_send->setToolTip(QStringLiteral("Send message (Return)"));
    actions->addWidget(m_send);
    layout->addWidget(m_composerActions);
    connect(m_input, &QTextEdit::textChanged, this, [this] { updateComposer(m_busy); });
    return composer;
}

void ChatPanel::fitInput() {
    // One line at rest, growing with the draft up to four, then it scrolls.
    const int line = m_input->fontMetrics().lineSpacing();
    const int content = static_cast<int>(std::ceil(m_input->document()->size().height()));
    const int height = std::clamp(content, line, 4 * line);
    if (m_input->height() != height) m_input->setFixedHeight(height);
    m_input->setVerticalScrollBarPolicy(content > 4 * line ? Qt::ScrollBarAsNeeded : Qt::ScrollBarAlwaysOff);
}

CommentRow *ChatPanel::addComment(CommentRow::Kind kind, const QString &text) {
    // Sending follows the conversation to the latest again.
    if (m_rows.isEmpty() || kind == CommentRow::Kind::User) m_follow = true;
    m_welcome->hide();
    auto *row = new CommentRow(kind, text, QDateTime::currentDateTime());
    connect(row, &CommentRow::askAgain, this, &ChatPanel::askAgain);
    m_list->insertWidget(m_list->indexOf(m_statusRow), row);
    m_rows.append(row);
    updateCount();
    return row;
}

QString ChatPanel::transcript() const {
    QStringList parts;
    for (const CommentRow *row : m_rows) parts << row->name() + QLatin1Char('\n') + row->text();
    return parts.join(QStringLiteral("\n\n"));
}

void ChatPanel::refreshTimes() {
    const QDateTime now = QDateTime::currentDateTime();
    for (CommentRow *row : std::as_const(m_rows)) row->refreshTime(now);
    if (m_welcomeTime) m_welcomeTime->setText(relativeTime(m_welcomeSince, now));
    if (m_proposalSince.isValid()) m_proposalTime->setText(relativeTime(m_proposalSince, now));
}

void ChatPanel::updateCount() {
    // As a comments panel heads itself: the title, then how many.
    m_count->setText(m_rows.isEmpty() ? QString() : QString::number(m_rows.size()));
}

void ChatPanel::setProposalShown(bool shown, const QString &id) {
    m_proposal->setVisible(shown);
    if (!shown) {
        m_proposalExpander->setChecked(false);
        m_proposalId.clear();
        m_proposalSince = QDateTime();
    } else if (!id.isEmpty() && id != m_proposalId) {
        // A new proposal is read from its head: the list scrolls to its top
        // (the proposal 16 points under the header) and stops following the
        // latest comment until the reader scrolls or sends.
        m_proposalId = id;
        // Its time ages from when it came, as a comment's does.
        m_proposalSince = QDateTime::currentDateTime();
        m_proposalTime->setText(relativeTime(m_proposalSince, m_proposalSince));
        m_follow = false;
        m_scroll->verticalScrollBar()->setValue(0);
        QTimer::singleShot(0, this, [this] {
            if (!m_follow) m_scroll->verticalScrollBar()->setValue(0);
        });
    }
    updateListEdge();
}

QList<ProposalChangeRow *> ChatPanel::proposalChangeRows() const {
    return m_proposalChanges->findChildren<ProposalChangeRow *>(QString(), Qt::FindDirectChildrenOnly);
}

void ChatPanel::setProposalChanges(const QList<ProposalChange> &changes) {
    QLayout *layout = m_proposalChanges->layout();
    while (QLayoutItem *item = layout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    const qsizetype shown = std::min<qsizetype>(changes.size(), kProposalChangeLines);
    for (qsizetype index = 0; index < shown; ++index) layout->addWidget(new ProposalChangeRow(changes.at(index), m_proposalChanges));
    if (changes.size() > shown)
        layout->addWidget(label(QStringLiteral("%1 more in the script").arg(changes.size() - shown), "chatChangeMore"));
    m_proposalChanges->setProperty("count", int(changes.size()));
    fitProposalDiff();
}

bool ChatPanel::proposalExpanded() const { return m_proposalExpander->isChecked(); }

void ChatPanel::setProposalExpanded(bool expanded) { m_proposalExpander->setChecked(expanded); }

bool ChatPanel::composerExpanded() const { return m_composerActions->isVisibleTo(m_composer); }

void ChatPanel::updateComposer(bool busy) {
    m_busy = busy;
    const QWidget *focus = QApplication::focusWidget();
    const bool focused = focus && m_composer->isAncestorOf(focus);
    const bool draft = !m_input->toPlainText().isEmpty();
    m_composerActions->setVisible(m_input->isEnabled() ? (focused || draft || busy) : busy);
    m_cancel->setVisible(!busy && (focused || draft));
    m_typing->setProperty("active", busy);
    m_typing->update();
    fitInput();
}

void ChatPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    painter.setBrush(editorTheme().chatSurface);
    painter.drawRoundedRect(QRectF(rect()), kRadius, kRadius);
}

void ChatPanel::resizeEvent(QResizeEvent *) {
    m_resizeHandle->setGeometry(0, kRadius, 6, std::max(0, height() - 2 * kRadius));
    m_resizeHandle->raise();
    placeProposalExpander();
    fitProposalDiff();
}

void ChatPanel::placeProposalExpander() {
    // In a narrow panel (beside a laptop) "Script" would take the end of the
    // summary's line and wrap it: it moves to the start of the pills' row,
    // a text button level with Reject and Accept (which tighten a little),
    // and the summary gets the whole column, without its count (the rows
    // under it count the changes), so it keeps one line.
    // Only once the stage has placed it (an unplaced panel's width means nothing).
    const bool narrow = testAttribute(Qt::WA_Resized) && width() < kNarrowProposal;
    const QString summary = narrow && !m_summaryShort.isEmpty() ? m_summaryShort : m_summaryFull;
    if (m_proposalSummary->text() != summary) m_proposalSummary->setText(summary);
    m_proposalSummary->setToolTip(summary == m_summaryFull ? QString() : m_summaryFull);
    if (m_proposal->property("narrow").toBool() != narrow) {
        m_proposal->setProperty("narrow", narrow);
        for (QPushButton *pill : {m_accept, m_reject}) {
            repolish(pill);
            // Their padding changed: they measure themselves afresh.
            QEvent change(QEvent::StyleChange);
            QCoreApplication::sendEvent(pill, &change);
            pill->updateGeometry();
        }
    }
    const bool inRow = m_proposalButtons->indexOf(m_proposalExpander) >= 0;
    if (inRow == narrow) return;
    if (narrow) {
        // [Script, taking the slack, aligned left][Reject][Accept].
        m_proposalLine->removeWidget(m_proposalExpander);
        delete m_proposalButtons->takeAt(0); // the stretch before the pills
        m_proposalExpander->setCompact(true);
        m_proposalButtons->insertWidget(0, m_proposalExpander, 1, Qt::AlignLeft | Qt::AlignVCenter);
    } else {
        m_proposalButtons->removeWidget(m_proposalExpander);
        m_proposalButtons->insertStretch(0, 1);
        m_proposalExpander->setCompact(false);
        m_proposalLine->addWidget(m_proposalExpander, 0, Qt::AlignVCenter);
    }
}

void ChatPanel::fitProposalDiff() {
    // As tall as its lines, up to ten (then it scrolls); it scrolls with the
    // list it heads. Its frame is in its contents margins (a QFrame keeps it
    // there). The changes are listed until the script and diff, which hold
    // them all, are opened.
    const QMargins frame = m_proposalDiff->contentsMargins();
    const int chrome = frame.top() + frame.bottom();
    const int margins = static_cast<int>(std::ceil(2 * m_proposalDiff->document()->documentMargin()));
    const int lines = m_proposalDiff->fontMetrics().lineSpacing();
    const int least = 2 * lines + margins + chrome;
    const int most = kProposalDiffLines * lines + margins + chrome;
    const bool expanded = m_proposalExpander->isChecked();
    m_proposalChanges->setVisible(m_proposalChanges->property("count").toInt() > 0 && !expanded);
    const int content = static_cast<int>(std::ceil(m_proposalDiff->document()->size().height())) + chrome;
    m_proposalDiff->setFixedHeight(std::clamp(content, least, most));
    m_proposalDiff->setVisible(expanded);
}

void ChatPanel::updateListEdge() {
    const int value = m_scroll->verticalScrollBar()->value();
    // The compact bar while the pinned proposal's head ("Pinned by Kerf",
    // which says what the pills decide) or its pills are out of the view.
    bool sticky = false;
    if (m_proposal->isVisibleTo(m_content)) {
        const QRect pills(m_accept->mapTo(m_content, QPoint()), m_accept->size());
        const int head = m_proposalPinLine->mapTo(m_content, QPoint()).y() + m_proposalPinLine->height();
        sticky = head < value || pills.top() < value || pills.bottom() >= value + m_scroll->viewport()->height();
    }
    if (m_proposalBar->isVisibleTo(this) != sticky) {
        m_proposalBar->setVisible(sticky);
        // The view's height changes with it.
        refitListSoon();
    }
    m_barAccept->setEnabled(m_accept->isEnabled());
    m_barReject->setEnabled(m_reject->isEnabled());
    updateEdgeCover();
    // A hairline under the header (or the bar) while the view is scrolled
    // away from its top.
    const bool scrolled = value > 0;
    if (m_scrollEdge->property("scrolled").toBool() == scrolled) return;
    m_scrollEdge->setProperty("scrolled", scrolled);
    repolish(m_scrollEdge);
}

QList<QRect> ChatPanel::edgeCoverStrips() const { return m_edgeCover->strips(); }

QList<TextLine> labelTextLines(const QLabel *label) {
    // Laid out as the label lays it out: its font, its width when it
    // wraps, its rich text's own line heights; placed as its alignment
    // places the text in its contents.
    const QRect inner = label->contentsRect();
    QTextDocument document;
    document.setDocumentMargin(0);
    document.setDefaultFont(label->font());
    const bool rich = label->textFormat() == Qt::RichText || (label->textFormat() == Qt::AutoText && Qt::mightBeRichText(label->text()));
    if (rich) document.setHtml(label->text());
    else document.setPlainText(label->text());
    document.setTextWidth(label->wordWrap() ? inner.width() : -1);
    const double height = document.size().height();
    const Qt::Alignment align = label->alignment();
    const double offset = (align & Qt::AlignTop) ? 0.0 : (align & Qt::AlignBottom) ? inner.height() - height : (inner.height() - height) / 2.0;
    // Capitals stand this far under a line's ascent; a larger span (a
    // bigger ascent) has taller capitals, in proportion.
    const QFontMetricsF metrics(label->font());
    const double capShare = metrics.ascent() > 0 ? metrics.capHeight() / metrics.ascent() : 0.7;
    QList<TextLine> lines;
    for (QTextBlock block = document.begin(); block.isValid(); block = block.next()) {
        const QRectF bounds = document.documentLayout()->blockBoundingRect(block);
        const QTextLayout *layout = block.layout();
        for (int index = 0; layout && index < layout->lineCount(); ++index) {
            const QTextLine line = layout->lineAt(index);
            const QRectF at(bounds.left() + line.x(), bounds.top() + line.y() + offset, std::max(1.0, line.naturalTextWidth()), line.height());
            const QRect rect(inner.left() + int(std::floor(at.left())), inner.top() + int(std::floor(at.top())), int(std::ceil(at.width())),
                             int(std::round(at.height())));
            const int capTop = inner.top() + int(std::floor(at.top() + line.ascent() * (1.0 - capShare)));
            lines << TextLine{rect, std::clamp(capTop, rect.top(), rect.bottom())};
        }
    }
    return lines;
}

TextLine buttonTextLine(const QAbstractButton *button) {
    const QFontMetricsF metrics(button->font());
    const double line = metrics.lineSpacing();
    const double middle = button->height() / 2.0;
    const QRect rect(0, int(std::round(middle - line / 2.0)), button->width(), int(std::round(line)));
    // The words' box (ascent over descent) is centred; the capitals stand
    // capHeight over its baseline.
    const double textTop = middle - metrics.height() / 2.0;
    const int capTop = int(std::floor(textTop + metrics.ascent() - metrics.capHeight()));
    return {rect, std::clamp(capTop, rect.top(), rect.bottom())};
}

void ChatPanel::updateEdgeCover() {
    QWidget *viewport = m_scroll->viewport();
    m_edgeCover->setGeometry(viewport->rect());
    m_edgeCover->raise();
    const int top = 0;
    const int bottom = viewport->height();
    QList<QRect> strips;
    // `box` (in the view) cut by an edge: covered where less than `least`
    // of it would show (`leastTop` under the top edge, when given).
    const auto cut = [&](const QRect &box, double least, double leastTop = -1.0) {
        if (box.top() < top && box.bottom() >= top && box.bottom() + 1 - top < (leastTop >= 0.0 ? leastTop : least))
            strips << QRect(box.left(), top, box.width(), box.bottom() + 1 - top);
        if (box.top() < bottom && box.bottom() >= bottom && bottom - box.top() < least)
            strips << QRect(box.left(), box.top(), box.width(), bottom - box.top());
    };
    for (QWidget *child : m_content->findChildren<QWidget *>()) {
        if (!child->isVisibleTo(m_content) || child == m_edgeCover) continue;
        const QRect box(child->mapTo(viewport, QPoint()), child->size());
        if (box.bottom() < top || box.top() >= bottom) continue;
        if (box.top() >= top && box.bottom() < bottom) continue;
        // Text: each line by itself, by its line's height; under the top
        // edge, by its capitals: a line shows only with their tops.
        if (auto *label = qobject_cast<QLabel *>(child); label && !label->text().isEmpty()) {
            for (const TextLine &line : labelTextLines(label))
                cut(line.rect.translated(box.topLeft()), kSliverLine * line.rect.height(),
                    std::max(kSliverLine * line.rect.height(), double(line.rect.bottom() + 1 - line.capTop)));
            continue;
        }
        // A button's words, centred in it: as a label's line, by its
        // capitals under the top edge.
        if (auto *button = qobject_cast<QAbstractButton *>(child); button && !button->text().isEmpty() && button->findChildren<QWidget *>().isEmpty()) {
            // Under the top edge the cover runs on to the button's foot, so
            // no descender of the words it hides is left under it (the style
            // may set them a point lower than their centred line).
            const TextLine line = buttonTextLine(button);
            const QRect words = line.rect.translated(box.topLeft());
            const int foot = std::max(0, box.bottom() - words.bottom());
            cut(words.adjusted(0, 0, 0, foot), kSliverLine * words.height(),
                std::max(kSliverLine * words.height(), double(line.rect.bottom() + 1 - line.capTop)) + foot);
            cut(box, kSliver);
            continue;
        }
        // Symbols and painted rows: only leaves (a container's own parts
        // are judged one by one).
        bool leaf = true;
        for (QWidget *grand : child->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
            if (grand->isVisibleTo(child)) leaf = false;
        // An avatar shows only while half of it does: under that, the name
        // line beside it has gone and its arc would stand alone.
        const double share = qobject_cast<ChatAvatar *>(child) ? kSliverAvatar : kSliverSymbol;
        if (leaf) cut(box, std::max<double>(kSliver, share * box.height()));
    }
    m_edgeCover->setStrips(strips);
}

void ChatPanel::refitListSoon() {
    // Once the panel and the list are laid out again, the view measures its
    // content and its range afresh: a scroll area hears of its viewport's
    // resize, not of its content's new size hint (as when the pinned
    // proposal shows, opens or folds), and its range would lag a step.
    QTimer::singleShot(0, m_scroll, [this] {
        QEvent request(QEvent::LayoutRequest);
        QCoreApplication::sendEvent(m_scroll, &request);
    });
}

void ChatPanel::setHovered(bool hovered) {
    if (m_scroll->property("hovered").toBool() == hovered) return;
    m_scroll->setProperty("hovered", hovered);
    repolish(m_scroll->verticalScrollBar());
}

void ChatPanel::enterEvent(QEnterEvent *) { setHovered(true); }

void ChatPanel::leaveEvent(QEvent *) { setHovered(false); }

void ChatPanel::updateChipRow() {
    auto *flow = static_cast<FlowLayout *>(m_chips->layout());
    const int room = std::max(0, m_chips->width() - 2 * kSidePadding);
    const int most = std::max(0, flow->contentWidth() - room);
    flow->setScroll(std::clamp(flow->scroll(), 0, most));
    const bool back = flow->scroll() > 0;
    const bool forward = flow->scroll() < most;
    const int top = 8;
    // Each end: the pager, at the row's padding less 8, over a fade that
    // runs kChipFadeWidth in from it.
    const int inset = kSidePadding - 8;
    const int fadeWidth = inset + kChipPager + kChipFadeWidth;
    // Only chips that fit whole show between the pagers: each end's cover
    // starts at the first chip that does not (8 points clear of the pager),
    // opaque, so no part of a chip shows through it. A chip too wide to fit
    // whole even alone keeps the gradient instead.
    int leftCover = fadeWidth;
    int rightCover = m_chips->width() - fadeWidth;
    int leftRamp = kChipFadeWidth;
    int rightRamp = kChipFadeWidth;
    {
        const int openLeft = back ? inset + kChipPager + 8 : 0;
        const int openRight = forward ? m_chips->width() - inset - kChipPager - 8 : m_chips->width();
        QList<QRect> placed;
        for (int index = 0; index < flow->count(); ++index)
            if (QWidget *chip = flow->itemAt(index)->widget(); chip && !chip->isHidden()) placed << chip->geometry();
        bool whole = false;
        for (const QRect &chip : std::as_const(placed)) whole = whole || (chip.left() >= openLeft && chip.right() < openRight);
        if (whole) {
            if (forward)
                for (const QRect &chip : std::as_const(placed))
                    if (chip.right() >= openRight && chip.left() < m_chips->width()) {
                        rightCover = std::clamp(chip.left(), openLeft, openRight);
                        rightRamp = 0;
                        break;
                    }
            if (back)
                for (auto it = placed.crbegin(); it != placed.crend(); ++it)
                    if (it->left() < openLeft && it->right() >= 0) {
                        leftCover = std::clamp(it->right() + 1, openLeft, openRight);
                        leftRamp = 0;
                        break;
                    }
        }
    }
    static_cast<ChipFade *>(m_chipFadeLeft)->setRamp(leftRamp);
    static_cast<ChipFade *>(m_chipFadeRight)->setRamp(rightRamp);
    m_chipFadeLeft->setGeometry(0, top, leftCover, kChipHeight);
    m_chipFadeRight->setGeometry(rightCover, top, m_chips->width() - rightCover, kChipHeight);
    m_chipBack->move(inset, top + (kChipHeight - kChipPager) / 2);
    m_chipForward->move(m_chips->width() - inset - kChipPager, top + (kChipHeight - kChipPager) / 2);
    m_chipFadeLeft->setVisible(back);
    m_chipFadeRight->setVisible(forward);
    m_chipBack->setVisible(back);
    m_chipForward->setVisible(forward);
    for (QWidget *over : {static_cast<QWidget *>(m_chipFadeLeft), static_cast<QWidget *>(m_chipFadeRight),
                          static_cast<QWidget *>(m_chipBack), static_cast<QWidget *>(m_chipForward)})
        over->raise();
}

void ChatPanel::pageChips(int direction) {
    auto *flow = static_cast<FlowLayout *>(m_chips->layout());
    flow->setScroll(flow->scroll() + direction * kChipPage);
    updateChipRow();
}

bool ChatPanel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_chips) {
        switch (event->type()) {
        case QEvent::Wheel: {
            // A wheel or a trackpad scrolls the row sideways, either way it turns.
            auto *wheel = static_cast<QWheelEvent *>(event);
            const QPoint pixels = wheel->pixelDelta();
            const QPoint angle = wheel->angleDelta() / 4;
            const int delta = !pixels.isNull() ? (pixels.x() ? pixels.x() : pixels.y()) : (angle.x() ? angle.x() : angle.y());
            auto *flow = static_cast<FlowLayout *>(m_chips->layout());
            flow->setScroll(flow->scroll() - delta);
            updateChipRow();
            return true;
        }
        case QEvent::Resize:
        case QEvent::LayoutRequest:
        case QEvent::ChildAdded:
        case QEvent::Show:
            QTimer::singleShot(0, this, &ChatPanel::updateChipRow);
            break;
        default:
            break;
        }
    }
    if (watched == m_scroll->viewport() && event->type() == QEvent::Resize)
        QTimer::singleShot(0, this, &ChatPanel::updateListEdge);
    if (watched == m_content && event->type() == QEvent::LayoutRequest) refitListSoon();
    if (watched == m_content && (event->type() == QEvent::LayoutRequest || event->type() == QEvent::Resize))
        QTimer::singleShot(0, this, &ChatPanel::updateEdgeCover);
    if ((watched == m_accept || watched == m_reject) && event->type() == QEvent::EnabledChange) updateListEdge();
    if (watched == m_resizeHandle) {
        switch (event->type()) {
        case QEvent::MouseButtonPress: {
            auto *mouse = static_cast<QMouseEvent *>(event);
            if (mouse->button() != Qt::LeftButton) break;
            m_dragFromX = int(mouse->globalPosition().x());
            m_dragFromWidth = width();
            return true;
        }
        case QEvent::MouseMove: {
            auto *mouse = static_cast<QMouseEvent *>(event);
            if (!(mouse->buttons() & Qt::LeftButton)) break;
            // The panel grows leftward, towards the picture.
            emit widthDragged(m_dragFromWidth - (int(mouse->globalPosition().x()) - m_dragFromX));
            return true;
        }
        case QEvent::MouseButtonRelease:
            if (static_cast<QMouseEvent *>(event)->button() == Qt::LeftButton) emit widthDragFinished();
            return true;
        default:
            break;
        }
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
