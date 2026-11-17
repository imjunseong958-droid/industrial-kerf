#include "composer.h"

#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QAbstractTextDocumentLayout>
#include <QAction>
#include <QApplication>
#include <QFontMetrics>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
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

// ------------------------------------------------------------------ permission

namespace permission {

QString name(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::AcceptEdits: return QStringLiteral("accept-edits");
    case PermissionMode::Auto: return QStringLiteral("auto");
    case PermissionMode::Ask: break;
    }
    return QStringLiteral("ask");
}

PermissionMode named(const QString &name) {
    if (name == QLatin1String("accept-edits")) return PermissionMode::AcceptEdits;
    if (name == QLatin1String("auto")) return PermissionMode::Auto;
    return PermissionMode::Ask;
}

QString label(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::AcceptEdits: return QStringLiteral("Accept edits");
    case PermissionMode::Auto: return QStringLiteral("Auto");
    case PermissionMode::Ask: break;
    }
    return QStringLiteral("Ask before edits");
}

QString summary(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::AcceptEdits: return QStringLiteral("Edits apply as Kerf makes them");
    case PermissionMode::Auto: return QStringLiteral("Edits apply, and Kerf makes its shots");
    case PermissionMode::Ask: break;
    }
    return QStringLiteral("Each edit waits for you");
}

PermissionMode next(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::Ask: return PermissionMode::AcceptEdits;
    case PermissionMode::AcceptEdits: return PermissionMode::Auto;
    case PermissionMode::Auto: break;
    }
    return PermissionMode::Ask;
}

} // namespace permission

namespace {

QString modeIcon(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::AcceptEdits: return QStringLiteral("g-check");
    case PermissionMode::Auto: return QStringLiteral("g-kerf");
    case PermissionMode::Ask: break;
    }
    return QStringLiteral("g-lock");
}

// What the chip is drawn in: the more Kerf does on its own, the more it says so.
QColor modeTone(PermissionMode mode) {
    switch (mode) {
    case PermissionMode::AcceptEdits: return color(accentInk);
    case PermissionMode::Auto: return color(activity::gpu);
    case PermissionMode::Ask: break;
    }
    return QColor();
}

// "68k", "1.2M": a count as a glance reads it.
QString shortCount(qint64 count) {
    if (count >= 1'000'000) return QStringLiteral("%1M").arg(QString::number(count / 1'000'000.0, 'f', count >= 10'000'000 ? 0 : 1));
    if (count >= 1000) return QStringLiteral("%1k").arg(qRound64(count / 1000.0));
    return QString::number(count);
}

} // namespace

// ------------------------------------------------------------------ ComposerChip

ComposerChip::ComposerChip(QWidget *parent) : ui::KitButton(parent) {
    setFixedHeight(24);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
}

void ComposerChip::setIconName(const QString &icon) {
    if (icon == m_icon) return;
    m_icon = icon;
    setFixedWidth(sizeHint().width());
    update();
}

void ComposerChip::setLabel(const QString &text) {
    if (text == this->text()) return;
    setText(text);
    setFixedWidth(sizeHint().width());
    update();
}

void ComposerChip::setTone(const QColor &tone) {
    if (tone == m_tone) return;
    m_tone = tone;
    update();
}

void ComposerChip::setChevron(bool chevron) {
    if (chevron == m_chevron) return;
    m_chevron = chevron;
    setCursor(chevron ? Qt::PointingHandCursor : Qt::ArrowCursor);
    setFixedWidth(sizeHint().width());
    update();
}

void ComposerChip::setWidestLabel(int width) {
    if (width == m_widest) return;
    m_widest = width;
    setFixedWidth(sizeHint().width());
    update();
}

QSize ComposerChip::sizeHint() const {
    int width = 6;
    if (!m_icon.isEmpty()) width += 14 + 4;
    width += std::min(m_widest, ui::snapWidth(control, text()));
    if (m_chevron) width += 2 + 12;
    return QSize(width + 6, 24);
}

void ComposerChip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF r = rect();
    if (isEnabled() && m_chevron && hoverProgress() > 0) {
        QColor fill = color(hover2);
        fill.setAlphaF(float(hoverProgress()));
        p.setPen(Qt::NoPen);
        p.setBrush(fill);
        p.drawRoundedRect(r, radius::control, radius::control);
    }
    const bool lit = isEnabled() && m_chevron && isHovered();
    const QColor ink_ = !isEnabled() ? color(dis) : m_tone.isValid() ? m_tone : color(lit ? ink : ink2);
    qreal x = 6;
    if (!m_icon.isEmpty()) {
        icons::paint(p, m_icon, QRectF(x, (height() - 14) / 2.0, 14, 14), ink_, 14);
        x += 14 + 4;
    }
    const QFont face = tokens::font(control);
    const QFontMetricsF metrics(face);
    const qreal room = std::min<qreal>(m_widest, width() - x - 6 - (m_chevron ? 14 : 0));
    p.setFont(face);
    p.setPen(ink_);
    p.drawText(QRectF(x, 0, room + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, metrics.elidedText(text(), Qt::ElideMiddle, room + 1));
    if (m_chevron)
        icons::paint(p, QStringLiteral("g-chev"), QRectF(width() - 6 - 12, (height() - 12) / 2.0, 12, 12),
                     color(lit ? ink2 : ink3), 12);
    if (focusVisible()) ui::paintFocusRing(p, r, radius::control);
}

// ------------------------------------------------------------------ ContextMeter

ContextMeter::ContextMeter(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("composerContext"));
    setFixedSize(24, 24);
    setAccessibleName(QStringLiteral("Context"));
    hide();
}

void ContextMeter::setUse(qint64 used, qint64 limit, bool tokens) {
    used = std::max<qint64>(0, used);
    limit = std::max<qint64>(0, limit);
    if (used == m_used && limit == m_limit && tokens == m_tokens) return;
    m_used = used;
    m_limit = limit;
    m_tokens = tokens;
    const QString said = describe();
    setToolTip(said);
    setAccessibleDescription(said);
    update();
}

qreal ContextMeter::fraction() const { return m_limit > 0 ? std::clamp(qreal(m_used) / qreal(m_limit), 0.0, 1.0) : 0.0; }

QColor ContextMeter::ringColor() const {
    const qreal full = fraction();
    return color(full >= 0.9 ? neg : full >= 0.75 ? warn : ink2);
}

QString ContextMeter::describe() const {
    if (m_limit <= 0) return {};
    const int percent = int(std::lround(fraction() * 100.0));
    // Kerf's own agent keeps what fits and puts older steps away; a model's
    // window, counted in its tokens, says how much of it is taken.
    return m_tokens ? QStringLiteral("Context %1% full · %2 of %3 tokens").arg(percent).arg(shortCount(m_used), shortCount(m_limit))
                    : QStringLiteral("Context %1% full · Kerf puts older steps away as it fills").arg(percent);
}

void ContextMeter::paintEvent(QPaintEvent *) {
    if (m_limit <= 0) return;
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    constexpr qreal across = 14;
    constexpr qreal stroke = 2;
    const QRectF ring((width() - across) / 2.0, (height() - across) / 2.0, across, across);
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(color(track), stroke));
    p.drawEllipse(ring);
    const qreal full = fraction();
    if (full <= 0.0) return;
    // From the top, clockwise; the least of it still shows.
    p.setPen(QPen(ringColor(), stroke, Qt::SolidLine, Qt::RoundCap));
    p.drawArc(ring, 90 * 16, -int(std::lround(std::max(full, 0.03) * 360 * 16)));
}

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
    // 18 either side, as the conversation's text keeps 22 in; 12 over and under.
    outer->setContentsMargins(18, 12, 18, 12);
    outer->setSpacing(0);
    // What waits for Kerf to finish: "Next: …" and Cancel, over the box.
    m_queuedRow = new QWidget;
    m_queuedRow->setObjectName(QStringLiteral("composerQueued"));
    {
        auto *row = new QHBoxLayout(m_queuedRow);
        row->setContentsMargins(4, 0, 0, 6);
        row->setSpacing(8);
        m_queuedLabel = new QLabel;
        m_queuedLabel->setFont(tokens::font(meta));
        QPalette palette = m_queuedLabel->palette();
        palette.setColor(QPalette::WindowText, color(ink2));
        m_queuedLabel->setPalette(palette);
        m_queuedLabel->setTextFormat(Qt::PlainText);
        row->addWidget(m_queuedLabel, 1);
        auto *cancel = new ui::GhostButton(QStringLiteral("Cancel"));
        cancel->setObjectName(QStringLiteral("composerQueuedCancel"));
        cancel->setMeta(true);
        connect(cancel, &QAbstractButton::clicked, this, [this] {
            setQueued(QString());
            emit queuedCancelled();
        });
        m_queuedCancel = cancel;
        row->addWidget(cancel);
    }
    m_queuedRow->hide();
    outer->addWidget(m_queuedRow);
    auto *box = new ComposerBox;
    m_box = box;
    m_box->setObjectName(QStringLiteral("composerBox"));
    // One row: what to ask, then how full Kerf's context is and Send, which
    // keep to the foot as the text grows. Nothing else is in the box.
    auto *boxLayout = new QHBoxLayout(m_box);
    boxLayout->setContentsMargins(16, 10, 8, 10);
    boxLayout->setSpacing(8);
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
    // Its sheet, with the size of the face chosen for reading (refreshFaces).
    refreshFaces();
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
    boxLayout->addWidget(m_input, 1, Qt::AlignVCenter);
    m_context = new ContextMeter;
    boxLayout->addWidget(m_context, 0, Qt::AlignBottom);
    m_send = new SendButton(SendButton::Mode::Send);
    m_stop = new SendButton(SendButton::Mode::Stop);
    m_stop->hide();
    connect(m_send, &QAbstractButton::clicked, this, &Composer::submitRequested);
    connect(m_stop, &QAbstractButton::clicked, this, &Composer::stopRequested);
    boxLayout->addWidget(m_send, 0, Qt::AlignBottom);
    boxLayout->addWidget(m_stop, 0, Qt::AlignBottom);
    outer->addWidget(m_box);

    // Under the box, what the request goes with: where (the owner's folder
    // chip, first) and when at the start, who answers and how far Kerf may go
    // at the end.
    auto *row = new QWidget;
    row->setObjectName(QStringLiteral("composerTools"));
    auto *rowLayout = new QHBoxLayout(row);
    m_toolRow = rowLayout;
    rowLayout->setContentsMargins(4, 6, 4, 0);
    rowLayout->setSpacing(4);
    m_stamp = new StampButton;
    m_stamp->setTimecode(QStringLiteral("00:00:00:00"));
    rowLayout->addWidget(m_stamp);
    rowLayout->addStretch(1);
    m_model = new ComposerChip;
    m_model->setObjectName(QStringLiteral("composerModel"));
    m_model->setAccessibleName(QStringLiteral("Model"));
    connect(m_model, &QAbstractButton::clicked, this, [this] {
        if (m_model->hasChevron()) ui::popupMenu(buildModelMenu(), m_model, ui::Side::Above);
    });
    rowLayout->addWidget(m_model);
    m_mode = new ComposerChip;
    m_mode->setObjectName(QStringLiteral("composerMode"));
    connect(m_mode, &QAbstractButton::clicked, this, [this] { ui::popupMenu(buildModeMenu(), m_mode, ui::Side::Above); });
    rowLayout->addWidget(m_mode);
    m_mode->hide();
    m_model->hide();
    outer->addWidget(row);

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
    setPermissionMode(m_permission);
    setModels({}, QString(), false);
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

void Composer::setAgentControls(bool shown) {
    m_agentControls = shown;
    m_mode->setVisible(shown);
    m_model->setVisible(shown && !m_currentModel.isEmpty());
    if (!shown) m_context->hide();
}

void Composer::setPermissionMode(PermissionMode mode) {
    m_permission = mode;
    m_mode->setIconName(modeIcon(mode));
    m_mode->setLabel(permission::label(mode));
    m_mode->setTone(modeTone(mode));
    m_mode->setAccessibleName(QStringLiteral("Permissions: %1").arg(permission::label(mode)));
    ui::setTip(m_mode, QStringLiteral("%1 · %2").arg(permission::label(mode), permission::summary(mode)),
               ui::keyText(QKeySequence(Qt::SHIFT | Qt::Key_Tab)));
}

QMenu *Composer::buildModeMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("composerModeMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    ui::setMenuMinimumCardWidth(menu, 340);
    ui::addMenuHeader(menu, QStringLiteral("What Kerf does without asking"));
    for (const PermissionMode mode : {PermissionMode::Ask, PermissionMode::AcceptEdits, PermissionMode::Auto}) {
        QAction *action = menu->addAction(permission::label(mode));
        action->setObjectName(QStringLiteral("composerMode.%1").arg(permission::name(mode)));
        action->setCheckable(true);
        action->setChecked(mode == m_permission);
        ui::setMenuKey(action, permission::summary(mode), true);
        connect(action, &QAction::triggered, this, [this, mode] {
            if (mode == m_permission) return;
            setPermissionMode(mode);
            emit permissionModeChosen(mode);
        });
    }
    return menu;
}

void Composer::setModels(const QStringList &models, const QString &current, bool choosable) {
    m_models = models;
    m_currentModel = current;
    m_modelsChoosable = choosable && models.size() > 1;
    m_model->setLabel(current);
    m_model->setChevron(m_modelsChoosable);
    m_model->setVisible(m_agentControls && !current.isEmpty());
    ui::setTip(m_model, m_modelsChoosable ? QStringLiteral("Model · %1").arg(current) : QStringLiteral("Answers come from %1").arg(current));
}

QMenu *Composer::buildModelMenu() {
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("composerModelMenu"));
    menu->setAttribute(Qt::WA_DeleteOnClose);
    ui::styleMenu(menu);
    ui::addMenuHeader(menu, QStringLiteral("Model"));
    for (const QString &model : std::as_const(m_models)) {
        QAction *action = menu->addAction(model);
        action->setCheckable(true);
        action->setChecked(model == m_currentModel);
        connect(action, &QAction::triggered, this, [this, model] { emit modelChosen(model); });
    }
    menu->addSeparator();
    QAction *settings = menu->addAction(QStringLiteral("Agent settings…"));
    settings->setObjectName(QStringLiteral("composerModelSettings"));
    connect(settings, &QAction::triggered, this, &Composer::agentSettingsRequested);
    return menu;
}

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

void Composer::setQueued(const QString &text) {
    m_queuedText = text.trimmed();
    const QString line = m_queuedText.simplified();
    m_queuedLabel->setText(QStringLiteral("Next: %1").arg(line));
    m_queuedLabel->setToolTip(m_queuedText);
    // One line, cut where it runs out of room.
    m_queuedLabel->setText(QFontMetrics(m_queuedLabel->font()).elidedText(m_queuedLabel->text(), Qt::ElideRight, std::max(80, width() - 120)));
    m_queuedRow->setVisible(!m_queuedText.isEmpty());
    updateGeometry();
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
    m_roomy = true;
    static_cast<ComposerBox *>(m_box)->setRadius(16);
    refreshFaces();
}

void Composer::refreshFaces() {
    QFont font = tokens::font(tokens::reading);
    // A point larger where the composer is the whole of the window's work.
    if (m_roomy) font.setPixelSize(font.pixelSize() + 1);
    m_input->setFont(font);
    // The app's sheet gives #chatInput a size of its own; this one says the size in use.
    m_input->setStyleSheet(QStringLiteral("QTextEdit#chatInput, QTextEdit#chatInput:focus, QTextEdit#chatInput:hover, "
                                          "QTextEdit#chatInput:disabled { background: transparent; border: 0; padding: 0; "
                                          "color: %1; font-size: %2px; }")
                               .arg(color(ink).name(QColor::HexRgb))
                               .arg(font.pixelSize()));
    fitInput();
}


void Composer::fitInput() {
    m_input->document()->setTextWidth(m_input->viewport()->width());
    const int lines = std::clamp(int(std::ceil(m_input->document()->size().height())), m_leastInput, 110);
    if (m_input->height() != lines) m_input->setFixedHeight(lines);
    m_input->setVerticalScrollBarPolicy(m_input->document()->size().height() > 100 ? Qt::ScrollBarAsNeeded : Qt::ScrollBarAlwaysOff);
}

bool Composer::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_input && event->type() == QEvent::KeyPress) {
        auto *key = static_cast<QKeyEvent *>(event);
        const bool enter = key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter;
        if (enter && !(key->modifiers() & Qt::ShiftModifier)) {
            // Return sends; while Kerf works it waits for the turn to end; a
            // draft stays while neither is possible.
            if (!m_busy && m_send->isEnabled()) emit submitRequested();
            else if (m_busy && !text().isEmpty()) emit queueRequested();
            return true;
        }
        if (key->key() == Qt::Key_Escape) {
            emit escapePressed();
            return true;
        }
        // Shift-Tab goes round how far Kerf may go, as it does in a coding
        // agent's prompt; Tab still leaves the text.
        if (key->key() == Qt::Key_Backtab && m_agentControls) {
            setPermissionMode(permission::next(m_permission));
            emit permissionModeChosen(m_permission);
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
