#include "chat_panel.h"
#include "editor_theme.h"

#include <QAbstractTextDocumentLayout>
#include <QApplication>
#include <QClipboard>
#include <QComboBox>
#include <QEnterEvent>
#include <QFrame>
#include <QHBoxLayout>
#include <QLabel>
#include <QLinearGradient>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QStyle>
#include <QStyleOptionButton>
#include <QStylePainter>
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

// A comment's text as the row shows it: wrappable, on 20-point lines for
// 14-point type, as a comments panel sets its text.
QString bodyMarkup(const QString &text) {
    return QStringLiteral("<div style=\"line-height:120%\">%1</div>")
        .arg(wrappable(text).toHtmlEscaped().replace(QLatin1Char('\n'), QStringLiteral("<br>")));
}

// Over the top of a scrolled view: the panel's surface, solid down to where
// the line the view's top edge cuts ends (see ChatPanel::listCover), then
// fading to clear over a few points, so a comment cut off by the view's top
// edge reads as scrolled out of sight and never shows a sliced line. It is
// the scroll area's child, not its viewport's, so it stays put while the
// content scrolls, and is shown only while the view's top edge cuts a
// comment (see ChatPanel::updateScrollFade). Clicks pass through it.
class ScrollTopFade final : public QWidget {
public:
    explicit ScrollTopFade(QScrollArea *area) : QWidget(area), m_area(area) {
        setObjectName(QStringLiteral("scrollFade"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        area->installEventFilter(this);
        connect(EditorAppearance::instance(), &EditorAppearance::changed, this, qOverload<>(&QWidget::update));
        hide();
        follow();
    }
    void setSolid(int solid) {
        solid = std::max(0, solid);
        if (solid == m_solid) return;
        m_solid = solid;
        follow();
        update();
    }
    int solid() const { return m_solid; }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched == m_area && (event->type() == QEvent::Resize || event->type() == QEvent::LayoutRequest)) follow();
        return QWidget::eventFilter(watched, event);
    }
    void paintEvent(QPaintEvent *) override {
        const QColor surface = editorTheme().chatSurface;
        QColor clear = surface;
        clear.setAlpha(0);
        QPainter painter(this);
        painter.fillRect(QRect(0, 0, width(), m_solid), surface);
        QLinearGradient fade(0, m_solid, 0, height());
        fade.setColorAt(0, surface);
        fade.setColorAt(1, clear);
        painter.fillRect(QRect(0, m_solid, width(), height() - m_solid), fade);
    }

private:
    void follow() {
        const QRect viewport = m_area->viewport()->geometry();
        setGeometry(viewport.left(), viewport.top(), viewport.width(),
                    std::min(viewport.height(), m_solid + ChatPanel::kScrollFadeHeight));
        raise();
    }
    QScrollArea *m_area;
    int m_solid = 0;
};

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

protected:
    void paintEvent(QPaintEvent *) override {
        const QColor surface = editorTheme().chatSurface;
        QColor clear = surface;
        clear.setAlpha(0);
        const double ramp = std::clamp(double(ChatPanel::kChipFadeWidth) / std::max(1, width()), 0.0, 1.0);
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
};

// A welcome's example: a chip that, narrower than its words, ends them in
// an ellipsis instead of clipping them, as a chip in a narrow column does.
class ExampleChip final : public QPushButton {
public:
    using QPushButton::QPushButton;

protected:
    void paintEvent(QPaintEvent *) override {
        QStylePainter painter(this);
        QStyleOptionButton option;
        initStyleOption(&option);
        const QRect contents = style()->subElementRect(QStyle::SE_PushButtonContents, &option, this);
        option.text = fontMetrics().elidedText(text(), Qt::ElideRight, std::max(0, contents.width() + 1));
        painter.drawControl(QStyle::CE_PushButton, option);
    }
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

QString relativeTime(const QDateTime &then, const QDateTime &now) {
    const qint64 seconds = std::max<qint64>(0, then.secsTo(now));
    if (seconds < 60) return QStringLiteral("now");
    if (seconds < 3600) return QStringLiteral("%1 min ago").arg(seconds / 60);
    if (seconds < 86400) return QStringLiteral("%1 h ago").arg(seconds / 3600);
    return then.date().toString(QStringLiteral("d MMM"));
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
    // A chevron, 6 points, then the label, in a 28-point pill.
    return {10 + 12 + 6 + fontMetrics().horizontalAdvance(text()) + 12, 28};
}

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
    const QPointF c(10 + 6, height() / 2.0);
    const qreal turn = isChecked() ? -1.0 : 1.0;
    painter.setPen(QPen(ink, 1.6, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.drawPolyline(QPolygonF{c + QPointF(-4.5, -2.25 * turn), c + QPointF(0, 2.25 * turn), c + QPointF(4.5, -2.25 * turn)});
    painter.setFont(font());
    painter.drawText(QRectF(10 + 12 + 6, 0, width() - 28, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
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
    if (kind == Kind::Error) line->addWidget(label(QStringLiteral("· couldn’t complete the edit"), "chatErrorTag"));
    line->addWidget(m_timeLabel);
    line->addStretch(1);
    column->addLayout(line);
    column->addSpacing(2);
    m_body = label(QString(), "chatBody");
    m_body->setTextFormat(Qt::RichText);
    m_body->setText(bodyMarkup(text));
    m_body->setWordWrap(true);
    m_body->setTextInteractionFlags(Qt::TextSelectableByMouse);
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
    const QColor ink = isChecked() ? theme.railGlyphActive : theme.railGlyph;
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
    m_proposal = buildProposal();
    outer->addWidget(m_proposal);

    // A hairline under the header while the view is scrolled, so a line
    // cut off at the top reads as scrolled content and not as a glitch.
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
    m_list->setContentsMargins(kSidePadding, 16, kSidePadding, kListBottom);
    m_list->setSpacing(16);
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
    outer->addWidget(m_scroll, 1);
    m_scrollFade = new ScrollTopFade(m_scroll);
    // New output keeps the latest in view, unless the reader scrolled away.
    QScrollBar *bar = m_scroll->verticalScrollBar();
    connect(bar, &QScrollBar::rangeChanged, this, [this](int, int maximum) {
        if (m_follow) m_scroll->verticalScrollBar()->setValue(maximum);
        QTimer::singleShot(0, this, &ChatPanel::updateScrollFade);
    });
    // A pinned proposal that grows or folds changes the view's height.
    m_scroll->viewport()->installEventFilter(this);
    connect(bar, &QScrollBar::valueChanged, this, [this](int value) {
        m_follow = value >= m_scroll->verticalScrollBar()->maximum() - 4;
        updateScrollFade();
    });

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
    // threads are parted.
    layout->setContentsMargins(kSidePadding, 4, kSidePadding, 0);
    layout->setSpacing(6);
    // "Pinned by Kerf" over the comment, its pin in the avatars' column.
    auto *pinned = new QHBoxLayout;
    pinned->setContentsMargins(kAvatar - 12, 0, 0, 0);
    pinned->setSpacing(12);
    pinned->addWidget(new PinGlyph, 0, Qt::AlignVCenter);
    pinned->addWidget(label(QStringLiteral("Pinned by Kerf"), "chatPinned"));
    pinned->addStretch(1);
    layout->addLayout(pinned);
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
    column->addLayout(line);
    // The first few changes, listed under it as replies are.
    m_proposalChanges = new QWidget;
    m_proposalChanges->setObjectName(QStringLiteral("proposalChanges"));
    auto *changes = new QVBoxLayout(m_proposalChanges);
    changes->setContentsMargins(0, 0, 0, 2);
    changes->setSpacing(2);
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
    m_reject = pill(QStringLiteral("Reject"), "chatPillTonal", kPillHeight);
    m_accept = pill(QStringLiteral("Accept"), "chatPillFilled", kPillHeight);
    buttons->addWidget(m_reject);
    buttons->addWidget(m_accept);
    column->addLayout(buttons);
    column->addWidget(m_proposalDiff);
    row->addLayout(column, 1);
    layout->addLayout(row);
    return box;
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
    line->addWidget(label(QStringLiteral("now"), "chatTime"));
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
    auto *examples = new QWidget;
    auto *flow = new FlowLayout(examples, 8);
    for (const QString &example : {QStringLiteral("Trim the opening to three seconds."),
                                   QStringLiteral("Lower the music under the dialogue.")}) {
        auto *chip = new ExampleChip(QStringLiteral("“%1”").arg(example));
        chip->setObjectName(QStringLiteral("chatExample"));
        chip->setCursor(Qt::PointingHandCursor);
        chip->setProperty("bezelRadius", 8);
        chip->setToolTip(QStringLiteral("Write this in the composer"));
        connect(chip, &QPushButton::clicked, this, [this, example] {
            if (!m_input->isEnabled()) return;
            m_input->setPlainText(example);
            m_input->setFocus(Qt::OtherFocusReason);
            m_input->moveCursor(QTextCursor::End);
        });
        flow->addWidget(chip);
        m_examples.append(chip);
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
    if (m_rows.isEmpty()) m_follow = true;
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
}

void ChatPanel::updateCount() {
    // As a comments panel heads itself: the title, then how many.
    m_count->setText(m_rows.isEmpty() ? QString() : QString::number(m_rows.size()));
}

void ChatPanel::setProposalShown(bool shown) {
    m_proposal->setVisible(shown);
    if (!shown) m_proposalExpander->setChecked(false);
    updateScrollFade();
}

void ChatPanel::setProposalChanges(const QStringList &changes) {
    QLayout *layout = m_proposalChanges->layout();
    while (QLayoutItem *item = layout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    const qsizetype shown = std::min<qsizetype>(changes.size(), kProposalChangeLines);
    for (qsizetype index = 0; index < shown; ++index) {
        QLabel *change = label(wrappable(changes.at(index)), "chatChange");
        change->setWordWrap(true);
        change->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        layout->addWidget(change);
    }
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
    fitProposalDiff();
}

void ChatPanel::fitProposalDiff() {
    // As tall as its lines, up to ten (then it scrolls), and never so tall
    // that the conversation under the pinned proposal has less than its least.
    // Its frame is in its contents margins (a QFrame keeps it there).
    const QMargins frame = m_proposalDiff->contentsMargins();
    const int chrome = frame.top() + frame.bottom();
    const int margins = static_cast<int>(std::ceil(2 * m_proposalDiff->document()->documentMargin()));
    const int lines = m_proposalDiff->fontMetrics().lineSpacing();
    const int least = 2 * lines + margins + chrome;
    int most = kProposalDiffLines * lines + margins + chrome;
    bool fits = true;
    // The changes are listed until the script and diff, which hold them all, are opened.
    const bool listsChanges = m_proposalChanges->property("count").toInt() > 0 && !m_proposalExpander->isChecked();
    if (height() > 0) {
        const auto tall = [this](const QWidget *part) {
            return part->hasHeightForWidth() ? part->heightForWidth(width()) : part->sizeHint().height();
        };
        const int shown = m_proposalDiff->isVisibleTo(m_proposal) ? m_proposalDiff->height() + 2 : 0;
        // The changes' height in the comment's text column.
        const int column = std::max(1, width() - 2 * kSidePadding - kAvatar - 12);
        const int changesHeight = m_proposalChanges->layout()->hasHeightForWidth() ? m_proposalChanges->layout()->totalHeightForWidth(column)
                                                                                     : m_proposalChanges->sizeHint().height();
        const int listed = m_proposalChanges->isVisibleTo(m_proposal) ? changesHeight + 4 : 0;
        const int base = kHeaderHeight + (tall(m_proposal) - shown - listed) + m_scrollEdge->height() + tall(m_composer) +
                         (m_chips->isVisibleTo(this) ? m_chips->height() : 0) + kMinListHeight;
        // The changes stay listed unless the conversation would fall under its least.
        const int changes = listsChanges ? changesHeight + 4 : 0;
        const bool changesFit = height() - base - changes >= 0;
        m_proposalChanges->setVisible(listsChanges && changesFit);
        const int others = base + (changesFit ? changes : 0);
        // In a panel too short for even two lines of it, it stays folded.
        fits = height() - others >= least;
        most = std::min(most, std::max(least, height() - others));
    }
    if (height() <= 0) m_proposalChanges->setVisible(listsChanges);
    const int content = static_cast<int>(std::ceil(m_proposalDiff->document()->size().height())) + chrome;
    m_proposalDiff->setFixedHeight(std::clamp(content, least, most));
    m_proposalDiff->setVisible(m_proposalExpander->isChecked() && fits);
}

QWidget *ChatPanel::itemAcross(int y) const {
    for (int index = 0; index < m_list->count(); ++index) {
        QWidget *item = m_list->itemAt(index)->widget();
        if (!item || !item->isVisible()) continue;
        const QRect area = item->geometry();
        if (area.top() < y && area.bottom() >= y) return item;
    }
    return nullptr;
}

namespace {

// A band a comment's part takes down the content, and whether it is text
// (a name, a line) or a control (an avatar, a button).
struct Band {
    int top;
    int bottom;
    bool text;
    int label = -1; // The wrapped text it is a line of, if any.
};

// The bands a comment's parts take down the content: each leaf widget (an
// avatar, a name, a button) whole, and a wrapped text one line at a time,
// each line down to where the next starts.
void collectBands(const QWidget *widget, const QWidget *content, QList<Band> &bands) {
    bool parent = false;
    for (const QWidget *child : widget->findChildren<QWidget *>(Qt::FindDirectChildrenOnly)) {
        if (!child->isVisibleTo(content)) continue;
        parent = true;
        collectBands(child, content, bands);
    }
    if (parent) return;
    const int top = widget->mapTo(content, QPoint()).y();
    const auto *text = qobject_cast<const QLabel *>(widget);
    if (!text || !text->wordWrap() || text->text().isEmpty()) {
        bands.append({top, top + widget->height(), text != nullptr});
        return;
    }
    // The label's own lines: its text laid out again at its width, in its
    // font, as it lays it out.
    QTextDocument document;
    document.setDefaultFont(text->font());
    document.setDocumentMargin(0);
    if (text->textFormat() == Qt::RichText || (text->textFormat() == Qt::AutoText && Qt::mightBeRichText(text->text())))
        document.setHtml(text->text());
    else
        document.setPlainText(text->text());
    const QRect contents = text->contentsRect();
    document.setTextWidth(std::max(1, contents.width()));
    const double height = document.size().height();
    const double offset = top + contents.top() + ((text->alignment() & Qt::AlignVCenter) ? std::max(0.0, (contents.height() - height) / 2.0) : 0.0);
    QList<double> tops;
    for (QTextBlock block = document.begin(); block.isValid(); block = block.next()) {
        const QRectF area = document.documentLayout()->blockBoundingRect(block);
        const QTextLayout *layout = block.layout();
        for (int line = 0; layout && line < layout->lineCount(); ++line) tops.append(area.top() + layout->lineAt(line).y());
    }
    if (tops.isEmpty()) {
        bands.append({top, top + widget->height(), true});
        return;
    }
    tops.append(std::max(height, tops.constLast() + 1.0));
    const int label = int(bands.size());
    for (qsizetype line = 0; line + 1 < tops.size(); ++line)
        bands.append({qRound(offset + tops.at(line)), qRound(offset + tops.at(line + 1)), true, label});
}

} // namespace

int ChatPanel::coverEnd(int y) const {
    const QWidget *item = itemAcross(y);
    if (!item) return y;
    QList<Band> bands;
    collectBands(item, m_content, bands);
    // Down past every part the edge cuts, and every part that cuts where
    // that ends (the first line beside an avatar), until nothing is cut.
    int end = y;
    for (bool moved = true; moved;) {
        moved = false;
        for (const Band &band : std::as_const(bands)) {
            if (band.top < end && band.bottom > end) {
                end = band.bottom;
                moved = true;
            }
        }
    }
    // A paragraph's last line alone reads as a fragment: it goes with the
    // lines over it.
    for (const Band &cut : std::as_const(bands)) {
        if (cut.label < 0 || cut.bottom != end) continue;
        QList<const Band *> rest;
        for (const Band &band : std::as_const(bands))
            if (band.label == cut.label && band.top >= end) rest.append(&band);
        if (rest.size() == 1) end = rest.constFirst()->bottom;
        break;
    }
    // A comment's foot alone (its actions, its expander) is no comment:
    // with no text of it left in view, the cover takes the rest of it.
    const bool text = std::any_of(bands.cbegin(), bands.cend(), [end](const Band &band) { return band.text && band.top >= end; });
    return text ? end : std::max(end, item->geometry().bottom() + 1);
}

int ChatPanel::listCover() const {
    const int value = m_scroll->verticalScrollBar()->value();
    if (value <= 0 || !itemAcross(value)) return 0;
    return std::min(coverEnd(value) - value, m_scroll->viewport()->height());
}

void ChatPanel::updateScrollFade() {
    auto *fade = static_cast<ScrollTopFade *>(m_scrollFade);
    const int value = m_scroll->verticalScrollBar()->value();
    const bool cut = value > 0 && itemAcross(value);
    fade->setSolid(cut ? listCover() : 0);
    fade->setVisible(cut);
    // A hairline under the header while the view is scrolled; none under a
    // pinned proposal, which the list's own room parts from it.
    const bool scrolled = value > 0 && m_proposal->isHidden();
    if (m_scrollEdge->property("scrolled").toBool() == scrolled) return;
    m_scrollEdge->setProperty("scrolled", scrolled);
    repolish(m_scrollEdge);
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
    m_chipFadeLeft->setGeometry(0, top, fadeWidth, kChipHeight);
    m_chipFadeRight->setGeometry(m_chips->width() - fadeWidth, top, fadeWidth, kChipHeight);
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
        QTimer::singleShot(0, this, &ChatPanel::updateScrollFade);
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
