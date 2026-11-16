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
    // 18 either side, as the conversation's text keeps 22 in; 12 over, 16 under.
    outer->setContentsMargins(18, 12, 18, 16);
    outer->setSpacing(0);
    auto *box = new ComposerBox;
    m_box = box;
    m_box->setObjectName(QStringLiteral("composerBox"));
    auto *boxLayout = new QVBoxLayout(m_box);
    // pad 12 12 10; the stamp's margin-left −4 puts its clock 8 in.
    boxLayout->setContentsMargins(12, 12, 12, 10);
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
    m_input->setPlaceholderText(QStringLiteral("Ask Kerf to make or change something"));
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
    m_suggestions->setParent(parent);
    m_suggestions->hide();
}

void Composer::setTimecode(const QString &timecode) { m_stamp->setTimecode(timecode); }

void Composer::setStamped(bool stamped) { m_stamp->setChecked(!stamped); }

void Composer::setBusy(bool busy) {
    m_busy = busy;
    m_send->setVisible(!busy);
    m_stop->setVisible(busy);
    updatePopovers();
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

void Composer::fill(const QString &text) {
    m_input->setPlainText(text);
    QTextCursor cursor = m_input->textCursor();
    cursor.movePosition(QTextCursor::End);
    m_input->setTextCursor(cursor);
    m_input->setFocus(Qt::OtherFocusReason);
}

void Composer::clear() {
    m_input->clear();
    setError({});
}

void Composer::updatePopovers() {
    const bool focused = m_input->hasFocus();
    const bool empty = m_input->toPlainText().trimmed().isEmpty();
    const bool suggest = focused && empty && !m_busy && !m_suggestions->items().isEmpty();
    m_suggestions->setVisible(suggest && m_suggestions->parentWidget());
    placePopovers();
}

void Composer::placePopovers() {
    QWidget *host = m_suggestions->parentWidget();
    if (!host || !m_suggestions->isVisible()) return;
    const QPoint origin = host == this ? QPoint() : mapTo(host, QPoint());
    const QMargins margins = contentsMargins();
    m_suggestions->placeAbove(origin.x() + margins.left() + 16, origin.x() + width() - margins.right() - 12, origin.y() + 4);
    m_suggestions->raise();
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
