#include "composer.h"

#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QAbstractTextDocumentLayout>
#include <QAction>
#include <QApplication>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QMouseEvent>
#include <QPainter>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextDocument>
#include <QTextEdit>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <QSignalBlocker>

#include <algorithm>
#include <cmath>
#include <utility>

namespace kerf {

using namespace tokens;

namespace {

// The composer's box (.cbox): radius 8, raised, an inset line2 that turns
// accent while the input has focus; pad 8 8 8 12.
class ComposerBox final : public QWidget {
public:
    using QWidget::QWidget;
    void setFocused(bool focused) {
        m_focused = focused;
        update();
    }
    void setRadius(qreal radius) {
        m_radius = radius;
        update();
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), m_radius, m_radius);
        ui::paintInsetLine(p, QRectF(rect()), m_radius, color(m_focused ? accent : line2));
    }

private:
    qreal m_radius = radius::panel;
    bool m_focused = false;
};

// The first word of a name, as an @mention writes it: "Mina Park" → "Mina".
QString mentionToken(const QString &name) { return name.section(QLatin1Char(' '), 0, 0, QString::SectionSkipEmpty); }

} // namespace

// ------------------------------------------------------------------ SendButton

SendButton::SendButton(Mode mode, QWidget *parent) : QPushButton(parent), m_mode(mode) {
    setObjectName(mode == Mode::Send ? QStringLiteral("composerSend") : QStringLiteral("composerStop"));
    setFixedSize(24, 24);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setFlat(true);
    // The app's push-button sheet has no say over how it looks.
    setStyleSheet(QStringLiteral("QPushButton { border: 0; background: transparent; padding: 0; min-width: 0; }"));
    if (mode == Mode::Send) {
        ui::setTip(this, QStringLiteral("Send"), ui::keyText(QKeySequence(Qt::Key_Return)));
        setAccessibleName(QStringLiteral("Send"));
    } else {
        ui::setTip(this, QStringLiteral("Stop"), QStringLiteral("Esc"));
        setAccessibleName(QStringLiteral("Stop"));
    }
}

QColor SendButton::background() const {
    if (m_mode == Mode::Stop) return color(on);
    if (!isEnabled()) return color(on);
    return underMouse() ? color(accentHover) : color(accent);
}

QColor SendButton::glyphColor() const {
    if (m_mode == Mode::Stop) return isEnabled() ? color(ink) : color(dis);
    return isEnabled() ? color(accentText) : color(ink3);
}

void SendButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(background());
    p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    icons::paint(p, m_mode == Mode::Send ? QStringLiteral("g-send") : QStringLiteral("g-stop"), QRectF(rect()), glyphColor(), 16);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ StampButton

StampButton::StampButton(QWidget *parent) : QAbstractButton(parent) {
    setObjectName(QStringLiteral("composerStamp"));
    setCheckable(true);
    setFont(tokens::font(monoSmall));
    setFixedHeight(24);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    ui::setTip(this, QStringLiteral("Stamp timecode · click to detach"));
    connect(this, &QAbstractButton::toggled, this, [this](bool detached) {
        ui::setTip(this, detached ? QStringLiteral("Stamp timecode · click to attach") : QStringLiteral("Stamp timecode · click to detach"));
        update();
    });
}

void StampButton::setTimecode(const QString &timecode) {
    if (timecode == text()) return;
    setText(timecode);
    setFixedWidth(sizeHint().width());
    update();
}

QSize StampButton::sizeHint() const { return QSize(4 + 14 + 4 + ui::snapWidth(QFontMetricsF(font()), text()) + 6, 24); }

void StampButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    if (underMouse()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(hover2));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    const QColor ink = isChecked() ? color(ink3) : color(ink2);
    icons::paint(p, QStringLiteral("g-clock"), QRectF(4, 5, 14, 14), ink, 14);
    QFont face = font();
    face.setStrikeOut(isChecked());
    p.setFont(face);
    p.setPen(ink);
    const QFontMetricsF metrics(face);
    p.drawText(QPointF(4 + 14 + 4, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ ReplyBar

ReplyBar::ReplyBar(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("replyBar"));
    setFixedHeight(0);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(16, 0, 12, 0);
    layout->setSpacing(6);
    auto *glyph = new QLabel;
    glyph->setFixedSize(14, 14);
    glyph->setPixmap(icons::pixmap(QStringLiteral("g-reply"), color(ink3), 14, 2));
    layout->addWidget(glyph);
    m_label = new QLabel;
    m_label->setFont(tokens::font(meta));
    m_label->setTextFormat(Qt::RichText);
    QPalette palette = m_label->palette();
    palette.setColor(QPalette::WindowText, color(ink3));
    m_label->setPalette(palette);
    layout->addWidget(m_label, 1);
    m_close = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S20);
    m_close->setTip(QStringLiteral("Cancel reply"), QStringLiteral("Esc"));
    connect(m_close, &QAbstractButton::clicked, this, [this] {
        setTarget({}, {});
        emit cancelled();
    });
    layout->addWidget(m_close);
    m_grow = new QVariantAnimation(this);
    m_grow->setEasingCurve(easing());
    connect(m_grow, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) { setFixedHeight(value.toInt()); });
}

void ReplyBar::setTarget(const QString &prefix, const QString &name) {
    m_open = !name.isEmpty();
    if (m_open)
        m_label->setText(QStringLiteral("%1 <span style=\"color:%2;\">%3</span>")
                             .arg(prefix.toHtmlEscaped(), color(ink2).name(QColor::HexRgb), name.toHtmlEscaped()));
    animateTo(m_open ? 24 : 0);
}

void ReplyBar::animateTo(int height) {
    m_grow->stop();
    const int ms = ui::motionDuration(motion::fade);
    if (ms <= 0 || !isVisible()) {
        setFixedHeight(height);
        return;
    }
    m_grow->setDuration(ms);
    m_grow->setStartValue(this->height());
    m_grow->setEndValue(height);
    m_grow->start();
}

// ------------------------------------------------------------------ FloatingList

FloatingList::FloatingList(const QString &icon, QWidget *parent) : QWidget(parent), m_icon(icon) {
    setMouseTracking(true);
    setAttribute(Qt::WA_TranslucentBackground);
    hide();
}

void FloatingList::setItems(const QStringList &items) {
    m_items = items;
    m_current = items.isEmpty() ? -1 : std::clamp(m_current, -1, int(items.size()) - 1);
    update();
}

void FloatingList::setCurrent(int index) {
    m_current = m_items.isEmpty() ? -1 : std::clamp(index, 0, int(m_items.size()) - 1);
    update();
}

void FloatingList::moveCurrent(int delta) {
    if (m_items.isEmpty()) return;
    const int n = int(m_items.size());
    setCurrent(m_current < 0 ? (delta > 0 ? 0 : n - 1) : (m_current + delta + n) % n);
}

int FloatingList::cardHeight() const { return 4 + 28 * int(m_items.size()) + 4; }

void FloatingList::placeAbove(int left, int right, int bottom) {
    const QMargins shadow = ui::shadowMargins();
    const QRect card(left, bottom - cardHeight(), std::max(0, right - left), cardHeight());
    setGeometry(card.marginsAdded(shadow));
}

QRect FloatingList::cardRect() const { return rect().marginsRemoved(ui::shadowMargins()); }

QRect FloatingList::itemRect(int index) const {
    const QRect card = cardRect();
    return QRect(card.left() + 4, card.top() + 4 + 28 * index, card.width() - 8, 28);
}

int FloatingList::itemAt(const QPoint &point) const {
    for (int index = 0; index < m_items.size(); ++index)
        if (itemRect(index).contains(point)) return index;
    return -1;
}

void FloatingList::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF card(cardRect());
    ui::paintShadow(p, card, radius::card);
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::card));
    p.drawRoundedRect(card, radius::card, radius::card);
    ui::paintInsetLine(p, card, radius::card, color(line2));
    const QFont face = tokens::font(control);
    const QFontMetricsF metrics(face);
    p.setFont(face);
    for (int index = 0; index < m_items.size(); ++index) {
        const QRectF item(itemRect(index));
        if (index == m_hover || index == m_current) {
            p.setPen(Qt::NoPen);
            p.setBrush(color(raised));
            p.drawRoundedRect(item, radius::control, radius::control);
        }
        qreal x = item.left() + 8;
        if (!m_icon.isEmpty()) {
            icons::paint(p, m_icon, QRectF(x, item.top() + 7, 14, 14), color(ink3), 14);
            x += 14 + 8;
        }
        p.setPen(color(ink));
        const QString shown = metrics.elidedText(m_items.at(index), Qt::ElideRight, item.right() - 8 - x);
        p.drawText(QPointF(x, item.top() + (28 - metrics.height()) / 2 + metrics.ascent()), shown);
    }
}

void FloatingList::mouseMoveEvent(QMouseEvent *event) {
    const int index = itemAt(event->position().toPoint());
    if (index != m_hover) {
        m_hover = index;
        update();
    }
}

void FloatingList::mousePressEvent(QMouseEvent *event) {
    const int index = itemAt(event->position().toPoint());
    if (event->button() == Qt::LeftButton && index >= 0) emit chosen(index);
}

void FloatingList::leaveEvent(QEvent *) {
    m_hover = -1;
    update();
}

// ------------------------------------------------------------------ Composer

Composer::Composer(QWidget *parent) : QFrame(parent) {
    setObjectName(QStringLiteral("chatComposer"));
    // The box shows its focus; a ring on the text inside would be cut off by it.
    setProperty("ringsOwnFields", true);
    // The app's sheet draws #chatComposer's old divider; the composer paints its own.
    setStyleSheet(QStringLiteral("QFrame#chatComposer { border: 0; background: transparent; }"));
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    auto *outer = new QVBoxLayout(this);
    outer->setContentsMargins(16, 8, 12, 8);
    outer->setSpacing(0);
    auto *box = new ComposerBox;
    m_box = box;
    m_box->setObjectName(QStringLiteral("composerBox"));
    auto *boxLayout = new QVBoxLayout(m_box);
    // pad 8 8 8 12; the stamp's margin-left −4 puts its clock 8 in.
    boxLayout->setContentsMargins(8, 8, 8, 8);
    boxLayout->setSpacing(0);
    m_input = new QTextEdit;
    m_input->setObjectName(QStringLiteral("chatInput"));
    m_input->setAcceptRichText(false);
    m_input->setTabChangesFocus(true);
    m_input->setFrameShape(QFrame::NoFrame);
    m_input->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_input->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_input->setFont(tokens::font(tokens::reading));
    m_input->document()->setDocumentMargin(0);
    m_input->setPlaceholderText(QStringLiteral("Ask Kerf, or leave a note"));
    m_input->setStyleSheet(QStringLiteral("QTextEdit#chatInput, QTextEdit#chatInput:focus, QTextEdit#chatInput:hover, "
                                          "QTextEdit#chatInput:disabled { background: transparent; border: 0; padding: 0; "
                                          "color: %1; font-size: 14px; }")
                               .arg(color(ink).name(QColor::HexRgb)));
    {
        QPalette palette = m_input->palette();
        palette.setColor(QPalette::PlaceholderText, color(ink3));
        palette.setColor(QPalette::Text, color(ink));
        m_input->setPalette(palette);
    }
    m_input->setFixedHeight(22);
    m_input->installEventFilter(this);
    connect(m_input->document(), &QTextDocument::contentsChanged, this, [this] {
        fitInput();
        updatePopovers();
    });
    connect(m_input, &QTextEdit::cursorPositionChanged, this, &Composer::updatePopovers);
    {
        auto *line = new QWidget;
        auto *lineLayout = new QHBoxLayout(line);
        lineLayout->setContentsMargins(4, 0, 0, 0);
        lineLayout->setSpacing(0);
        lineLayout->addWidget(m_input);
        boxLayout->addWidget(line);
    }

    auto *row = new QWidget;
    row->setFixedHeight(28);
    auto *rowLayout = new QHBoxLayout(row);
    m_toolRow = rowLayout;
    rowLayout->setContentsMargins(0, 0, 0, 0);
    rowLayout->setSpacing(4);
    m_stamp = new StampButton;
    m_stamp->setTimecode(QStringLiteral("00:00:00:00"));
    rowLayout->addWidget(m_stamp);
    const auto tool = [rowLayout](const QString &icon, const QString &tip, const QString &key) {
        auto *button = new ui::IconButton(icon, ui::IconButton::Size::S24);
        button->setTip(tip, key);
        button->setAccessibleName(tip);
        rowLayout->addWidget(button);
        return button;
    };
    m_mention = tool(QStringLiteral("g-at"), QStringLiteral("Mention"), QStringLiteral("@"));
    m_draw = tool(QStringLiteral("g-annot"), QStringLiteral("Draw on frame"), QStringLiteral("D"));
    m_draw->setCheckable(true);
    connect(m_mention, &QAbstractButton::clicked, this, &Composer::startMention);
    connect(m_draw, &QAbstractButton::clicked, this, [this] {
        // The button shows the state; the click asks for the change.
        const QSignalBlocker blocker(m_draw);
        m_draw->setChecked(m_drawState != Draw::None);
        emit drawClicked();
    });
    rowLayout->addStretch(1);
    m_send = new SendButton(SendButton::Mode::Send);
    m_stop = new SendButton(SendButton::Mode::Stop);
    m_stop->hide();
    connect(m_send, &QAbstractButton::clicked, this, &Composer::submitRequested);
    connect(m_stop, &QAbstractButton::clicked, this, &Composer::stopRequested);
    rowLayout->addWidget(m_send);
    rowLayout->addWidget(m_stop);
    boxLayout->addWidget(row);
    outer->addWidget(m_box);

    m_error = new QLabel;
    m_error->setObjectName(QStringLiteral("composerError"));
    m_error->setFont(tokens::font(meta));
    m_error->setWordWrap(true);
    {
        QPalette palette = m_error->palette();
        palette.setColor(QPalette::WindowText, color(neg));
        m_error->setPalette(palette);
    }
    m_error->setContentsMargins(0, 4, 0, 0);
    m_error->hide();
    outer->addWidget(m_error);

    m_suggestions = new FloatingList(QStringLiteral("g-cdr"), this);
    m_suggestions->setObjectName(QStringLiteral("suggestionsPopover"));
    connect(m_suggestions, &FloatingList::chosen, this, [this](int index) {
        m_suggestions->hide();
        emit suggestionChosen(index);
    });
    m_mentions = new FloatingList(QString(), this);
    m_mentions->setObjectName(QStringLiteral("mentionList"));
    connect(m_mentions, &FloatingList::chosen, this, [this](int index) {
        if (index >= 0 && index < m_mentionShown.size()) insertMention(m_mentionShown.at(index));
    });
    connect(qApp, &QApplication::focusChanged, this, [this](QWidget *, QWidget *now) {
        const bool focused = now && (now == m_input || isAncestorOf(now));
        if (focused == m_focused) return;
        m_focused = focused;
        static_cast<ComposerBox *>(m_box)->setFocused(now == m_input);
        updatePopovers();
        emit focusChanged(focused);
    });
}

QString Composer::text() const { return m_input->toPlainText(); }

void Composer::setOverlayParent(QWidget *parent) {
    for (FloatingList *list : {m_suggestions, m_mentions}) {
        list->setParent(parent);
        list->hide();
    }
}

void Composer::setTimecode(const QString &timecode) { m_stamp->setTimecode(timecode); }

void Composer::setStamped(bool stamped) { m_stamp->setChecked(!stamped); }

void Composer::setBusy(bool busy) {
    m_busy = busy;
    m_send->setVisible(!busy);
    m_stop->setVisible(busy);
    updatePopovers();
}

void Composer::setDraw(Draw state, const QJsonArray &strokes) {
    m_drawState = state;
    m_drawing = state == Draw::Attached ? strokes : QJsonArray{};
    m_draw->setChecked(state != Draw::None);
    switch (state) {
    case Draw::None: m_draw->setTip(QStringLiteral("Draw on frame"), drawKey()); break;
    case Draw::Drawing: m_draw->setTip(QStringLiteral("Drawing on the frame · click to cancel"), QStringLiteral("Esc")); break;
    case Draw::Attached:
        m_draw->setTip(QStringLiteral("Drawing attached · %1 · click to remove")
                           .arg(strokes.size() == 1 ? QStringLiteral("1 stroke") : QStringLiteral("%1 strokes").arg(strokes.size())));
        break;
    }
}

void Composer::requestDraw() {
    if (m_draw->isEnabled()) m_draw->click();
}

void Composer::setDrawAction(QAction *action) {
    if (m_drawAction) {
        removeAction(m_drawAction);
        m_drawAction->disconnect(this);
    }
    m_drawAction = action;
    if (action) {
        action->setShortcutContext(Qt::WidgetWithChildrenShortcut);
        addAction(action);
        connect(action, &QAction::changed, this, [this] {
            if (m_drawState == Draw::None) m_draw->setTip(QStringLiteral("Draw on frame"), drawKey());
        });
    }
    if (m_drawState == Draw::None) m_draw->setTip(QStringLiteral("Draw on frame"), drawKey());
}

QString Composer::drawKey() const {
    return m_drawAction ? m_drawAction->shortcut().toString(QKeySequence::NativeText) : QStringLiteral("D");
}

bool Composer::hasFocusWithin() const {
    const QWidget *focus = QApplication::focusWidget();
    return focus && (focus == this || isAncestorOf(focus));
}

void Composer::setError(const QString &message) {
    m_error->setText(message);
    m_error->setVisible(!message.isEmpty());
}

void Composer::setSuggestions(const QStringList &labels) {
    m_suggestions->setItems(labels);
    updatePopovers();
}

QStringList Composer::suggestions() const { return m_suggestions->items(); }

void Composer::setMentionCandidates(const QStringList &names) {
    m_candidates = names;
    updatePopovers();
}

void Composer::startMention() {
    QTextCursor cursor = m_input->textCursor();
    const QString before = cursor.block().text().left(cursor.positionInBlock());
    if (!before.isEmpty() && !before.endsWith(QLatin1Char(' '))) cursor.insertText(QStringLiteral(" "));
    cursor.insertText(QStringLiteral("@"));
    m_input->setTextCursor(cursor);
    m_input->setFocus(Qt::OtherFocusReason);
    updatePopovers();
}

void Composer::fill(const QString &text) {
    m_input->setPlainText(text);
    QTextCursor cursor = m_input->textCursor();
    cursor.movePosition(QTextCursor::End);
    m_input->setTextCursor(cursor);
    m_input->setFocus(Qt::OtherFocusReason);
}

void Composer::clear() {
    m_input->clear();
    setDraw(Draw::None);
    setError({});
}

QString Composer::mentionPrefix(int *start) const {
    const QTextCursor cursor = m_input->textCursor();
    const QString line = cursor.block().text();
    const int at = cursor.positionInBlock();
    int index = at - 1;
    while (index >= 0 && !line.at(index).isSpace() && line.at(index) != QLatin1Char('@')) --index;
    if (index < 0 || line.at(index) != QLatin1Char('@')) return QString();
    if (index > 0 && (line.at(index - 1).isLetterOrNumber() || line.at(index - 1) == QLatin1Char('_'))) return QString();
    if (start) *start = cursor.block().position() + index;
    const QString prefix = line.mid(index + 1, at - index - 1);
    return prefix.isNull() ? QStringLiteral("") : prefix;
}

void Composer::insertMention(const QString &name) {
    int start = -1;
    if (mentionPrefix(&start).isNull() || start < 0) return;
    QTextCursor cursor = m_input->textCursor();
    cursor.setPosition(start, QTextCursor::KeepAnchor);
    cursor.insertText(QStringLiteral("@%1 ").arg(mentionToken(name)));
    m_input->setTextCursor(cursor);
    m_mentions->hide();
    m_input->setFocus(Qt::OtherFocusReason);
}

void Composer::updatePopovers() {
    const bool focused = m_input->hasFocus();
    const QString prefix = focused ? mentionPrefix() : QString();
    m_mentionShown.clear();
    if (!prefix.isNull())
        for (const QString &name : std::as_const(m_candidates))
            if (prefix.isEmpty() || name.startsWith(prefix, Qt::CaseInsensitive) ||
                mentionToken(name).startsWith(prefix, Qt::CaseInsensitive))
                m_mentionShown << name;
    const bool mention = !m_mentionShown.isEmpty();
    if (mention) {
        const QString before = m_mentions->items().value(m_mentions->current());
        m_mentions->setItems(m_mentionShown);
        const int keep = int(m_mentionShown.indexOf(before));
        m_mentions->setCurrent(keep >= 0 ? keep : 0);
    }
    const bool empty = m_input->toPlainText().trimmed().isEmpty();
    const bool suggest = focused && empty && !mention && !m_busy && !m_suggestions->items().isEmpty();
    m_mentions->setVisible(mention && m_mentions->parentWidget());
    m_suggestions->setVisible(suggest && m_suggestions->parentWidget());
    placePopovers();
}

void Composer::placePopovers() {
    for (FloatingList *list : {m_suggestions, m_mentions}) {
        QWidget *host = list->parentWidget();
        if (!host || !list->isVisible()) continue;
        const QPoint origin = host == this ? QPoint() : mapTo(host, QPoint());
        const QMargins margins = contentsMargins();
        list->placeAbove(origin.x() + margins.left() + 16, origin.x() + width() - margins.right() - 12, origin.y() + 4);
        list->raise();
    }
}

void Composer::setRoomy() {
    m_leastInput = 52;
    static_cast<ComposerBox *>(m_box)->setRadius(16);
    m_input->setStyleSheet(m_input->styleSheet().replace(QStringLiteral("font-size: 14px"), QStringLiteral("font-size: 15px")));
    QFont font = m_input->font();
    font.setPixelSize(15);
    m_input->setFont(font);
    fitInput();
}


void Composer::fitInput() {
    m_input->document()->setTextWidth(m_input->viewport()->width());
    const int lines = std::clamp(int(std::ceil(m_input->document()->size().height())), m_leastInput, 110);
    if (m_input->height() != lines) m_input->setFixedHeight(lines);
    m_input->setVerticalScrollBarPolicy(m_input->document()->size().height() > 100 ? Qt::ScrollBarAsNeeded : Qt::ScrollBarAlwaysOff);
}

void Composer::paintEvent(QPaintEvent *) {
    if (!m_divider) return;
    QPainter p(this);
    p.fillRect(QRect(0, 0, width(), 1), color(line));
}

bool Composer::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_input && event->type() == QEvent::KeyPress) {
        auto *key = static_cast<QKeyEvent *>(event);
        const bool enter = key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter;
        if (m_mentions->isVisible()) {
            if (key->key() == Qt::Key_Down || key->key() == Qt::Key_Up) {
                m_mentions->moveCurrent(key->key() == Qt::Key_Down ? 1 : -1);
                return true;
            }
            if ((enter && !(key->modifiers() & Qt::ShiftModifier)) || key->key() == Qt::Key_Tab) {
                insertMention(m_mentions->items().value(std::max(0, m_mentions->current())));
                return true;
            }
            if (key->key() == Qt::Key_Escape) {
                m_mentions->hide();
                return true;
            }
        }
        if (enter && !(key->modifiers() & Qt::ShiftModifier)) {
            // Return sends; a draft stays while sending is not possible.
            if (!m_busy && m_send->isEnabled()) emit submitRequested();
            return true;
        }
        if (key->key() == Qt::Key_Escape) {
            emit escapePressed();
            return true;
        }
    }
    if (watched == m_input && event->type() == QEvent::Resize) fitInput();
    return QFrame::eventFilter(watched, event);
}

void Composer::resizeEvent(QResizeEvent *event) {
    QFrame::resizeEvent(event);
    placePopovers();
}

void Composer::moveEvent(QMoveEvent *event) {
    QFrame::moveEvent(event);
    placePopovers();
}

} // namespace kerf
