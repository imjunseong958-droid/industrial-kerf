#include "inspector_widgets.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "icons.h"

#include <QAction>
#include <QApplication>
#include <QCursor>
#include <QEnterEvent>
#include <QFocusEvent>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLineEdit>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QRadialGradient>
#include <QScreen>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

namespace {

// Pixels of drag per step.
constexpr int kScrubPixels = 2;
// A press that travels less than this is a click.
constexpr int kClickSlop = 3;

QColor col(QRgb rgba) {
    return tokens::color(rgba);
}

QString css(QRgb rgba) {
    const QColor c = col(rgba);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

// The editor's typing field, over a value or a name: the raised fill, an
// accent ring (the design's focus colour), the figures as the value showed them.
QString editorSheet() {
    return QStringLiteral("QLineEdit { background: %1; color: %2; border: 1px solid %3; border-radius: 4px; padding: 0px 3px; "
                          "selection-background-color: %4; selection-color: %2; }")
        .arg(css(raised), css(tokens::ink), css(accent), css(accentHover));
}

// The knob shadow (0 1px 2px rgba(0,0,0,.6)) as a soft disc under a circle.
void paintKnobShadow(QPainter &painter, const QPointF &centre, qreal radius) {
    const Shadow &shadow = tokens::knob;
    const QColor shade = col(shadow.color);
    QColor clear = shade;
    clear.setAlpha(0);
    QRadialGradient gradient(centre + QPointF(shadow.dx, shadow.dy), radius + shadow.blur);
    gradient.setColorAt(0, shade);
    gradient.setColorAt((radius - shadow.blur / 2) / (radius + shadow.blur), shade);
    gradient.setColorAt(1, clear);
    painter.setPen(Qt::NoPen);
    painter.setBrush(gradient);
    painter.drawEllipse(centre + QPointF(shadow.dx, shadow.dy), radius + shadow.blur, radius + shadow.blur);
}

// The 14-point check box of the kit (.cbx) in `box`.
void paintCheck(QPainter &painter, const QRectF &box, bool checked, bool enabled) {
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    if (!enabled) painter.setOpacity(0.5);
    if (checked) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(col(on));
        painter.drawRoundedRect(box, radius::small, radius::small);
    }
    ui::paintInsetLine(painter, box, radius::small, col(checkboxOutline));
    if (checked) icons::paint(painter, QStringLiteral("checkbox-check"), box, col(ink2), 14);
    painter.restore();
}

} // namespace

QString inspectorFontCss(const tokens::TypeStyle &style) {
    return QStringLiteral("font-family: \"%1\"; font-size: %2px; font-weight: %3;")
        .arg(tokens::font(style).family()).arg(style.px).arg(style.weight);
}

QColor changedValueInk() {
    return col(tokens::ink);
}

// ---------------------------------------------------------------- ScrubField

ScrubField::ScrubField(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("scrubField"));
    // Tab reaches it; a click or a drag does not take the keys from the
    // timeline (typing gives them to its editor).
    setFocusPolicy(Qt::TabFocus);
    setAttribute(Qt::WA_Hover);
    setCursor(Qt::SizeHorCursor);
    setFixedHeight(kHeight);
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    m_flash = new QTimer(this);
    m_flash->setSingleShot(true);
    m_flash->setInterval(600);
    connect(m_flash, &QTimer::timeout, this, qOverload<>(&QWidget::update));
    m_stepRun = new QTimer(this);
    m_stepRun->setSingleShot(true);
    m_stepRun->setInterval(kStepRunMs);
    connect(m_stepRun, &QTimer::timeout, this, &ScrubField::flushSteps);
}

void ScrubField::flushSteps() {
    m_stepRun->stop();
    if (!m_stepping) return;
    m_stepping = false;
    if (m_mixed || formatNumber(m_format, m_value) != formatNumber(m_format, m_stepStart))
        emit committed(m_value, m_value - m_stepStart, true);
}

void ScrubField::setTrailingInset(int inset) {
    m_trailingInset = inset;
    if (m_editor) m_editor->setGeometry(rect().adjusted(0, 0, -m_trailingInset, 0));
    update();
}

void ScrubField::setFormat(const NumberFormat &format) {
    m_format = format;
    updateGeometry();
    update();
}

void ScrubField::setValue(double display) {
    // A snapshot arriving mid-gesture never pulls the value back.
    if (m_dragging || m_stepping) return;
    m_value = display;
    update();
}

void ScrubField::setDefaultValue(double display) {
    m_default = display;
    m_hasDefault = true;
    update();
}

bool ScrubField::isDefault() const {
    return m_hasDefault && !m_mixed && formatNumber(m_format, m_value) == formatNumber(m_format, m_default);
}

void ScrubField::setDimmed(bool dimmed) {
    if (m_dimmed == dimmed) return;
    m_dimmed = dimmed;
    update();
}

void ScrubField::setGlass(bool glass) {
    m_glass = glass;
}

void ScrubField::setSmall(bool small) {
    m_small = small;
    updateGeometry();
    update();
}

QColor ScrubField::valueInk() const {
    // Values are in the primary ink (mono 12 ink); a switched-off effect's,
    // a mixed value and a flash after a refused entry in the secondary;
    // the small value under a wheel in ink3; disabled in dis.
    if (!isEnabled()) return col(dis);
    if (m_small) return col(ink3);
    if (m_dimmed || m_mixed || m_flash->isActive()) return col(ink2);
    return col(tokens::ink);
}

QFont ScrubField::valueFont() const {
    return tokens::font(m_small ? monoSmall : timecode);
}

void ScrubField::setMixed(bool mixed) {
    m_mixed = mixed;
    update();
}

void ScrubField::setReadOnly(bool readOnly) {
    m_readOnly = readOnly;
    setCursor(readOnly ? Qt::ArrowCursor : Qt::SizeHorCursor);
    setFocusPolicy(readOnly ? Qt::NoFocus : Qt::TabFocus);
    update();
}

QString ScrubField::text() const {
    return m_mixed ? QStringLiteral("Mixed") : formatNumber(m_format, m_value);
}

QString ScrubField::unitText() const {
    if (m_mixed) return {};
    if (m_format.kind == NumberFormat::Kind::Pan || m_format.kind == NumberFormat::Kind::Timecode) return {};
    if (m_format.kind == NumberFormat::Kind::Gain && m_value <= m_format.min + 1e-6) return {};
    return m_format.kind == NumberFormat::Kind::Speed ? QStringLiteral("%") : m_format.unit;
}

int ScrubField::neededWidth() const {
    const QFontMetricsF value(valueFont());
    const QFontMetricsF small(tokens::font(meta));
    double needed = value.horizontalAdvance(text());
    const QString unit = unitText();
    // The unit's slot is 14 wide, wider for a long unit ("stops").
    if (!unit.isEmpty() || (!m_small && m_format.kind != NumberFormat::Kind::Timecode && m_format.kind != NumberFormat::Kind::Pan))
        needed += kUnitGap + std::max<double>(kUnitSlot, small.horizontalAdvance(unit));
    if (!m_format.prefix.isEmpty()) needed += small.horizontalAdvance(m_format.prefix) + 4.0;
    return int(std::ceil(needed)) + 4 + m_trailingInset;
}

QSize ScrubField::sizeHint() const {
    return {std::max(InspectorRow::kValueField, neededWidth()), kHeight};
}

double ScrubField::clamped(double display, bool *wasClamped) const {
    const double value = std::clamp(display, m_format.min, m_format.max);
    if (wasClamped) *wasClamped = value != display;
    return value;
}

double ScrubField::stepFor(Qt::KeyboardModifiers modifiers) const {
    if (modifiers & Qt::ShiftModifier) return m_format.bigStep;
    if (modifiers & Qt::AltModifier) return m_format.smallStep;
    return m_format.step;
}

void ScrubField::flash() {
    m_flash->start();
    update();
}

Qt::Alignment ScrubField::valueAlignment() const {
    return m_small ? Qt::AlignHCenter : Qt::AlignRight;
}

bool ScrubField::showsWhole() const {
    return neededWidth() - 4 <= width();
}

void ScrubField::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const bool hovered = m_hovered && isEnabled() && !m_readOnly && !isTyping();
    // Under the pointer (or while dragging) a raised wash says it scrubs.
    if (hovered || m_dragging) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(col(m_dragging ? hover2 : raised));
        painter.drawRoundedRect(QRectF(rect()).adjusted(0, 2, 0, -2), radius::small, radius::small);
    }
    if (isTyping()) return;
    const QRectF inner = QRectF(rect()).adjusted(2, 0, -2 - m_trailingInset, 0);
    const QFont valueFont = this->valueFont();
    const QFont unitFont = tokens::font(meta);
    const QColor unitInk = col(isEnabled() ? ink3 : dis);
    double right = inner.right();
    const QString unit = unitText();
    // The unit in its 14-point slot after the figures, left-aligned in it.
    if (!m_small && m_format.kind != NumberFormat::Kind::Timecode && m_format.kind != NumberFormat::Kind::Pan && !m_mixed) {
        const double slot = std::max<double>(kUnitSlot, QFontMetricsF(unitFont).horizontalAdvance(unit));
        if (!unit.isEmpty()) {
            painter.setFont(unitFont);
            painter.setPen(unitInk);
            painter.drawText(QRectF(right - slot, inner.top(), slot, inner.height()), Qt::AlignLeft | Qt::AlignVCenter, unit);
        }
        right -= slot + kUnitGap;
    } else if (!unit.isEmpty()) {
        const double width = QFontMetricsF(unitFont).horizontalAdvance(unit);
        painter.setFont(unitFont);
        painter.setPen(unitInk);
        painter.drawText(QRectF(right - width, inner.top(), width, inner.height()), Qt::AlignRight | Qt::AlignVCenter, unit);
        right -= width + kUnitGap;
    }
    const QFontMetricsF metrics(valueFont);
    double left = inner.left();
    if (!m_format.prefix.isEmpty()) {
        // The axis label (X, Y) 4 before the figures, which keep right.
        const double prefixWidth = QFontMetricsF(unitFont).horizontalAdvance(m_format.prefix);
        const double figures = std::min(metrics.horizontalAdvance(text()), right - left - prefixWidth - 4.0);
        painter.setFont(unitFont);
        painter.setPen(unitInk);
        painter.drawText(QRectF(right - figures - 4.0 - prefixWidth, inner.top(), prefixWidth + 1, inner.height()),
                         Qt::AlignLeft | Qt::AlignVCenter, m_format.prefix);
        left = right - figures;
    }
    const QRectF valueRect(left, inner.top(), std::max(0.0, right - left), inner.height());
    painter.setFont(valueFont);
    painter.setPen(valueInk());
    painter.drawText(valueRect, valueAlignment() | Qt::AlignVCenter, metrics.elidedText(text(), Qt::ElideLeft, valueRect.width()));
    if (hasFocus() && !m_readOnly) ui::paintFocusRing(painter, QRectF(rect()).adjusted(1, 3, -1, -3), radius::small);
}

void ScrubField::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || m_readOnly || !isEnabled()) return QWidget::mousePressEvent(event);
    scrubPress(event->globalPosition().toPoint());
    event->accept();
}

void ScrubField::mouseMoveEvent(QMouseEvent *event) {
    if (!m_pressed) return QWidget::mouseMoveEvent(event);
    scrubMove(event->globalPosition().toPoint(), event->modifiers());
}

void ScrubField::mouseReleaseEvent(QMouseEvent *event) {
    if (!m_pressed) return QWidget::mouseReleaseEvent(event);
    const bool clicked = !m_dragging;
    scrubRelease();
    if (clicked) beginTyping();
}

void ScrubField::scrubPress(const QPoint &global) {
    if (m_readOnly || !isEnabled()) return;
    m_pressed = true;
    m_dragging = false;
    m_pressX = global.x();
    m_travel = 0;
    m_startValue = m_value;
}

void ScrubField::scrubMove(const QPoint &global, Qt::KeyboardModifiers modifiers) {
    if (!m_pressed) return;
    const int dx = global.x() - m_pressX;
    if (!m_dragging && std::abs(dx) < kClickSlop) return;
    if (!m_dragging) {
        m_dragging = true;
        setCursor(Qt::BlankCursor);
    }
    // The pointer wraps at the screen's edge, so a long drag never runs out.
    int travel = m_travel + dx;
    if (QScreen *screen = QGuiApplication::screenAt(global); screen && QGuiApplication::platformName() != QLatin1String("offscreen")) {
        const QRect area = screen->geometry();
        if (global.x() <= area.left() + 1 || global.x() >= area.right() - 1) {
            const QPoint wrapped(global.x() <= area.left() + 1 ? area.right() - 2 : area.left() + 2, global.y());
            m_travel = travel;
            m_pressX = wrapped.x();
            QCursor::setPos(wrapped);
        }
    }
    const double steps = std::trunc(double(travel) / kScrubPixels);
    const double value = clamped(m_startValue + steps * stepFor(modifiers));
    m_value = value;
    update();
    emit previewed(value);
}

void ScrubField::scrubRelease() {
    if (!m_pressed) return;
    m_pressed = false;
    const bool dragged = m_dragging;
    m_dragging = false;
    setCursor(m_readOnly ? Qt::ArrowCursor : Qt::SizeHorCursor);
    update();
    if (dragged && formatNumber(m_format, m_value) != formatNumber(m_format, m_startValue))
        emit committed(m_value, m_value - m_startValue, true);
    else if (dragged)
        m_value = m_startValue;
}

void ScrubField::beginTyping() {
    if (m_readOnly || !isEnabled()) return;
    if (!m_editor) {
        m_editor = new QLineEdit(this);
        m_editor->setObjectName(QStringLiteral("scrubEditor"));
        m_editor->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
        m_editor->setStyleSheet(editorSheet());
        m_editor->installEventFilter(this);
        connect(m_editor, &QLineEdit::editingFinished, this, [this] {
            if (isTyping()) endTyping(true);
        });
    }
    m_editor->setFont(valueFont());
    m_editor->setGeometry(rect().adjusted(0, 1, -m_trailingInset, -1));
    m_editor->setText(m_mixed ? QString() : formatNumber(m_format, m_value));
    m_editor->show();
    m_editor->setFocus(Qt::MouseFocusReason);
    m_editor->selectAll();
    update();
}

bool ScrubField::isTyping() const {
    return m_editor && !m_editor->isHidden();
}

void ScrubField::commitText(const QString &text) {
    const std::optional<double> parsed = parseNumber(m_format, text, m_value);
    if (!parsed) {
        flash();
        return;
    }
    bool wasClamped = false;
    const double value = clamped(*parsed, &wasClamped);
    if (wasClamped) flash();
    QString typed = text.trimmed();
    typed.replace(QChar(0x2212), QLatin1Char('-'));
    const bool relative = typed.startsWith(QLatin1Char('+')) || typed.startsWith(QLatin1Char('*')) ||
                          (m_format.kind == NumberFormat::Kind::Timecode && typed.startsWith(QLatin1Char('-')));
    const double before = m_value;
    if (!m_mixed && formatNumber(m_format, value) == formatNumber(m_format, before)) return;
    m_value = value;
    update();
    emit committed(value, value - before, relative);
}

void ScrubField::endTyping(bool commit) {
    if (!m_editor) return;
    const QString text = m_editor->text();
    m_editor->hide();
    if (commit) {
        // A run of ↑↓ steps first, as its one edit; the text then says the
        // value the run reached, so committing it changes nothing more
        // unless something else was typed after the steps.
        flushSteps();
        commitText(text);
    } else if (m_stepping) {
        // Esc takes back a run that has not been sent yet.
        m_stepRun->stop();
        m_stepping = false;
        m_value = m_stepStart;
        emit previewed(m_value);
    }
    update();
}

void ScrubField::stepEditor(Qt::KeyboardModifiers modifiers, bool up) {
    const double base = parseNumber(m_format, m_editor->text(), m_value).value_or(m_value);
    bool wasClamped = false;
    const double value = clamped(base + stepFor(modifiers) * (up ? 1.0 : -1.0), &wasClamped);
    if (wasClamped) flash();
    if (!m_stepping) {
        m_stepping = true;
        m_stepStart = m_value;
    }
    m_value = value;
    m_editor->setText(formatNumber(m_format, value));
    m_editor->selectAll();
    update();
    emit previewed(value);
    m_stepRun->start();
}

bool ScrubField::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_editor && event->type() == QEvent::KeyPress) {
        auto *key = static_cast<QKeyEvent *>(event);
        // ↑↓ step the value being typed in (a line edit would only move its
        // caret): this is where the keys are after a click on the field.
        if (key->key() == Qt::Key_Up || key->key() == Qt::Key_Down) {
            stepEditor(key->modifiers(), key->key() == Qt::Key_Up);
            return true;
        }
        if (key->key() == Qt::Key_Escape) {
            endTyping(false);
            setFocus(Qt::OtherFocusReason);
            emit doneEditing();
            return true;
        }
        if (key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) {
            endTyping(true);
            setFocus(Qt::OtherFocusReason);
            emit doneEditing();
            return true;
        }
        if (key->key() == Qt::Key_Tab || key->key() == Qt::Key_Backtab) {
            endTyping(true);
            if (key->key() == Qt::Key_Tab) focusNextChild();
            else focusPreviousChild();
            return true;
        }
    }
    if (watched == m_editor && event->type() == QEvent::FocusOut && isTyping()) endTyping(true);
    return QWidget::eventFilter(watched, event);
}

void ScrubField::keyPressEvent(QKeyEvent *event) {
    if (m_readOnly) return QWidget::keyPressEvent(event);
    switch (event->key()) {
    case Qt::Key_Up:
    case Qt::Key_Down: {
        // One edit per run of presses: the value follows each, and the run
        // commits once it pauses (or the field loses focus).
        const double step = stepFor(event->modifiers()) * (event->key() == Qt::Key_Up ? 1.0 : -1.0);
        if (!m_stepping) {
            m_stepping = true;
            m_stepStart = m_value;
        }
        bool wasClamped = false;
        const double value = clamped(m_value + step, &wasClamped);
        if (wasClamped) flash();
        m_value = value;
        update();
        emit previewed(value);
        m_stepRun->start();
        return;
    }
    case Qt::Key_Return:
    case Qt::Key_Enter:
        beginTyping();
        return;
    default:
        break;
    }
    // Typing a digit or sign starts typing with it.
    const QString text = event->text();
    if (!text.isEmpty() && (text.at(0).isDigit() || QStringLiteral("+-*.").contains(text.at(0)))) {
        beginTyping();
        m_editor->setText(text);
        return;
    }
    QWidget::keyPressEvent(event);
}

void ScrubField::focusInEvent(QFocusEvent *event) {
    QWidget::focusInEvent(event);
    if (event->reason() == Qt::TabFocusReason || event->reason() == Qt::BacktabFocusReason) beginTyping();
    update();
}

void ScrubField::focusOutEvent(QFocusEvent *event) {
    QWidget::focusOutEvent(event);
    flushSteps();
    update();
}

void ScrubField::enterEvent(QEnterEvent *event) {
    m_hovered = true;
    update();
    QWidget::enterEvent(event);
}

void ScrubField::leaveEvent(QEvent *event) {
    m_hovered = false;
    update();
    QWidget::leaveEvent(event);
}

void ScrubField::resizeEvent(QResizeEvent *event) {
    if (m_editor) m_editor->setGeometry(rect().adjusted(0, 1, -m_trailingInset, -1));
    QWidget::resizeEvent(event);
}

// ----------------------------------------------------------- InspectorSlider

InspectorSlider::InspectorSlider(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorSlider"));
    setFixedHeight(ScrubField::kHeight);
    setMinimumWidth(32);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setCursor(Qt::PointingHandCursor);
}

void InspectorSlider::setRange(double minimum, double maximum) {
    m_min = minimum;
    m_max = std::max(maximum, minimum + 1e-9);
    update();
}

void InspectorSlider::setOrigin(double origin) {
    m_origin = origin;
    update();
}

void InspectorSlider::setDetent(double detent) {
    m_detent = detent;
    update();
}

void InspectorSlider::setGainTravel(bool gain) {
    m_gain = gain;
    update();
}

void InspectorSlider::setValue(double value) {
    if (m_dragging) return;
    m_value = std::clamp(value, m_min, m_max);
    update();
}

void InspectorSlider::setMixed(bool mixed) {
    m_mixed = mixed;
    update();
}

bool InspectorSlider::isBipolar() const {
    return m_min < 0.0 && m_max > 0.0 && std::abs(m_origin) < 1e-12;
}

QColor InspectorSlider::fillInk() const {
    if (!isEnabled()) return col(dis);
    return m_dimmed ? col(ink3) : col(track2);
}

void InspectorSlider::setDimmed(bool dimmed) {
    if (m_dimmed == dimmed) return;
    m_dimmed = dimmed;
    update();
}

double InspectorSlider::travelOf(double value) const {
    if (m_gain) return gainToTravel(value);
    return (std::clamp(value, m_min, m_max) - m_min) / (m_max - m_min);
}

double InspectorSlider::fromTravel(double travel) const {
    if (m_gain) return std::clamp(travelToGain(travel), m_min, m_max);
    return m_min + std::clamp(travel, 0.0, 1.0) * (m_max - m_min);
}

QRectF InspectorSlider::groove() const {
    // The kit's 2-point track, between insets of half the knob so the knob
    // is never cut, at the row's middle.
    return QRectF(kKnobRadius, height() / 2.0 - 1.0, std::max(1.0, width() - 2.0 * kKnobRadius), 2.0);
}

QPointF InspectorSlider::knobCenter() const {
    const QRectF track = groove();
    return {track.left() + travelOf(m_value) * track.width(), track.center().y()};
}

double InspectorSlider::valueAt(double x, double scale) const {
    const QRectF track = groove();
    const double detentX = track.left() + travelOf(m_detent) * track.width();
    // Half-open, so of the pointer's whole pixels exactly one lands on it.
    const double catchWidth = kSnap * scale;
    if (x >= detentX - catchWidth && x < detentX + catchWidth) return m_detent;
    return fromTravel((x - track.left()) / track.width());
}

QRect InspectorSlider::detentTick() const {
    if (!isBipolar()) return {};
    const QRectF track = groove();
    const double middle = track.left() + travelOf(0.0) * track.width();
    return {int(std::floor(middle)), int(std::lround(track.center().y())) - 3, 1, 6};
}

void InspectorSlider::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF track = groove();
    painter.setPen(Qt::NoPen);
    painter.setBrush(col(tokens::track));
    painter.drawRoundedRect(track, 1.0, 1.0);
    if (!m_mixed)
        if (const QRectF fill = fillRect(); !fill.isEmpty()) {
            painter.setBrush(fillInk());
            painter.drawRoundedRect(fill, 1.0, 1.0);
        }
    if (const QRect tick = detentTick(); !tick.isEmpty()) {
        painter.setRenderHint(QPainter::Antialiasing, false);
        painter.fillRect(tick, col(track2));
        painter.setRenderHint(QPainter::Antialiasing, true);
    }
    if (m_mixed) return;
    const QPointF centre = knobCenter();
    paintKnobShadow(painter, centre, kKnobRadius);
    painter.setBrush(isEnabled() ? col(tokens::ink) : col(dis));
    painter.drawEllipse(centre, kKnobRadius, kKnobRadius);
}

QRectF InspectorSlider::fillRect() const {
    const QRectF track = groove();
    if (m_mixed || std::abs(m_value - m_origin) < 1e-12) return {};
    const double originX = track.left() + travelOf(m_origin) * track.width();
    const double knob = knobCenter().x();
    return QRectF(QPointF(std::min(originX, knob), track.top()), QPointF(std::max(originX, knob), track.bottom()));
}

void InspectorSlider::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    m_dragging = true;
    m_start = m_value;
    // Nothing moves until the pointer does, then the knob moves by as much,
    // wherever the press began.
    m_pressX = event->position().x();
    m_pressKnob = knobCenter().x();
    m_fine = event->modifiers().testFlag(Qt::AltModifier);
}

double InspectorSlider::dragValue(double x, Qt::KeyboardModifiers modifiers) {
    const bool fine = modifiers.testFlag(Qt::AltModifier);
    if (fine != m_fine) {
        m_fine = fine;
        m_pressX = x;
        m_pressKnob = knobCenter().x();
    }
    const double scale = m_fine ? 1.0 / kFine : 1.0;
    return valueAt(m_pressKnob + (x - m_pressX) * scale, scale);
}

void InspectorSlider::mouseMoveEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    m_value = dragValue(event->position().x(), event->modifiers());
    update();
    emit moved(m_value);
}

void InspectorSlider::mouseReleaseEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    const double x = event->position().x();
    // A click that did not move changes nothing.
    if (std::abs(x - m_pressX) >= 1.0) m_value = dragValue(x, event->modifiers());
    m_dragging = false;
    update();
    if (std::abs(m_value - m_start) > 1e-12) emit released(m_value, m_start);
}

void InspectorSlider::mouseDoubleClickEvent(QMouseEvent *event) {
    // On the knob: back to the default, as a double-click on the label.
    if (event->button() != Qt::LeftButton || !isEnabled() || m_mixed ||
        std::abs(event->position().x() - knobCenter().x()) > kGrab) {
        mousePressEvent(event);
        return;
    }
    m_dragging = false;
    const double start = m_value;
    m_value = std::clamp(m_detent, m_min, m_max);
    update();
    if (std::abs(m_value - start) > 1e-12) emit released(m_value, start);
}

// ----------------------------------------------------------- InspectorSwitch

InspectorSwitch::InspectorSwitch(QWidget *parent) : QAbstractButton(parent) {
    setObjectName(QStringLiteral("inspectorSwitch"));
    setCheckable(true);
    setFixedSize(kSize, kSize);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
}

void InspectorSwitch::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    const QRectF box(0, 0, kSize, kSize);
    paintCheck(painter, box, isChecked(), isEnabled());
    if (hasFocus()) {
        painter.setRenderHint(QPainter::Antialiasing);
        ui::paintFocusRing(painter, box.adjusted(1, 1, -1, -1), radius::small);
    }
}

PowerKey::PowerKey(QWidget *parent) : InspectorSwitch(parent) {
    setObjectName(QStringLiteral("powerKey"));
    setAccessibleName(QStringLiteral("On"));
}

// ------------------------------------------------------------ SegmentedField

SegmentedField::SegmentedField(const QList<Segment> &segments, QWidget *parent) : QWidget(parent), m_segments(segments) {
    setObjectName(QStringLiteral("segmentedField"));
    setFixedHeight(tokens::height::h24);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setCursor(Qt::PointingHandCursor);
}

void SegmentedField::setCurrent(int index) {
    m_current = index >= 0 && index < m_segments.size() ? index : -1;
    update();
}

void SegmentedField::setDefaultIndex(int index) {
    m_default = index;
    update();
}

QRect SegmentedField::segmentRect(int index) const {
    const int count = std::max(1, int(m_segments.size()));
    const double each = (width() - 4.0) / count;
    return QRectF(2.0 + index * each, 2.0, each, height() - 4.0).toRect();
}

QSize SegmentedField::sizeHint() const {
    const QFontMetrics metrics(tokens::font(controlStrong));
    int width = 4;
    for (const Segment &segment : m_segments)
        width += segment.glyph != Glyph::None ? 24 : metrics.horizontalAdvance(segment.label) + 16;
    return {width, tokens::height::h24};
}

QColor SegmentedField::inkAt(int index) const {
    return index == m_current ? col(tokens::ink) : col(ink2);
}

void SegmentedField::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    painter.setBrush(col(raised));
    painter.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    for (int index = 0; index < m_segments.size(); ++index) {
        const QRectF segment = QRectF(segmentRect(index));
        if (index == m_current) {
            painter.setPen(Qt::NoPen);
            painter.setBrush(col(on));
            painter.drawRoundedRect(segment, radius::small, radius::small);
            ui::paintInsetLine(painter, segment, radius::small, col(line2));
            ui::paintTopHighlight(painter, segment, radius::small, col(segmentHighlight));
        }
        const QColor color = inkAt(index);
        const Segment &item = m_segments.at(index);
        if (item.glyph != Glyph::None) {
            painter.setPen(QPen(color, 1.25, Qt::SolidLine, Qt::RoundCap));
            const QPointF c = segment.center();
            const double widths[4] = {10.0, 6.0, 10.0, 6.0};
            for (int line = 0; line < 4; ++line) {
                const double y = c.y() - 4.5 + line * 3.0;
                const double w = widths[line];
                double left = c.x() - 5.0;
                if (item.glyph == Glyph::AlignCenter) left = c.x() - w / 2.0;
                if (item.glyph == Glyph::AlignRight) left = c.x() + 5.0 - w;
                painter.drawLine(QPointF(left, y), QPointF(left + w, y));
            }
        } else {
            painter.setFont(tokens::font(index == m_current ? controlStrong : control));
            painter.setPen(color);
            painter.drawText(segment, Qt::AlignCenter, QFontMetrics(painter.font()).elidedText(item.label, Qt::ElideRight, int(segment.width()) - 4));
        }
    }
    if (hasFocus()) ui::paintFocusRing(painter, QRectF(rect()), radius::control);
}

void SegmentedField::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    for (int index = 0; index < m_segments.size(); ++index)
        if (segmentRect(index).contains(event->position().toPoint())) {
            setCurrent(index);
            emit activated(index);
            return;
        }
}

void SegmentedField::keyPressEvent(QKeyEvent *event) {
    if (event->key() == Qt::Key_Left || event->key() == Qt::Key_Right) {
        const int step = event->key() == Qt::Key_Left ? -1 : 1;
        const int next = std::clamp(m_current < 0 ? 0 : m_current + step, 0, int(m_segments.size()) - 1);
        setCurrent(next);
        emit activated(next);
        return;
    }
    QWidget::keyPressEvent(event);
}

// ---------------------------------------------------------------- GhostPopup

GhostPopup::GhostPopup(QWidget *parent) : QComboBox(parent) {
    setObjectName(QStringLiteral("ghostPopup"));
    setAttribute(Qt::WA_Hover);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    setFixedHeight(tokens::height::h24);
    // Its own look: the app's QComboBox rules do not reach it.
    setStyleSheet(QStringLiteral("QComboBox { background: transparent; border: none; }"));
}

QColor GhostPopup::textColor() const {
    if (!isEnabled()) return col(dis);
    return m_hovered || (m_menu && m_menu->isVisible()) ? col(tokens::ink) : col(ink2);
}

QSize GhostPopup::sizeHint() const {
    // Pad 0 8, the text, gap 4, the 14-point chevron: snapped to whole points.
    return {8 + ui::snapWidth(control, currentIndex() < 0 ? placeholderText() : currentText()) + 4 + 14 + 8, tokens::height::h24};
}

QSize GhostPopup::minimumSizeHint() const {
    return {8 + 14 + 8, tokens::height::h24};
}

void GhostPopup::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF box(rect());
    if (isEnabled() && (m_hovered || (m_menu && m_menu->isVisible()))) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(col(raised));
        painter.drawRoundedRect(box, radius::control, radius::control);
    }
    const QColor color = textColor();
    painter.setFont(tokens::font(control));
    painter.setPen(color);
    const QRectF chevron(box.right() - 8 - 14, (box.height() - 14) / 2.0, 14, 14);
    const QRectF text(box.left() + 8, 0, std::max(0.0, chevron.left() - 4 - box.left() - 8), box.height());
    painter.drawText(text, Qt::AlignRight | Qt::AlignVCenter,
                     painter.fontMetrics().elidedText(currentIndex() < 0 ? placeholderText() : currentText(), Qt::ElideRight, int(text.width())));
    icons::paint(painter, QStringLiteral("g-chev"), chevron, color, 14);
    if (hasFocus()) ui::paintFocusRing(painter, box, radius::control);
}

void GhostPopup::showPopup() {
    if (count() == 0) return;
    delete m_menu;
    auto *menu = new QMenu(this);
    menu->setObjectName(QStringLiteral("ghostPopupMenu"));
    ui::styleMenu(menu);
    for (int index = 0; index < count(); ++index) {
        QAction *action = menu->addAction(itemText(index).isEmpty() ? QStringLiteral("None") : itemText(index));
        action->setCheckable(true);
        action->setChecked(index == currentIndex());
        connect(action, &QAction::triggered, this, [this, index] {
            setCurrentIndex(index);
            emit activated(index);
        });
    }
    connect(menu, &QMenu::aboutToHide, this, qOverload<>(&QWidget::update));
    m_menu = menu;
    ui::popupMenu(menu, this);
    update();
}

void GhostPopup::hidePopup() {
    if (m_menu) m_menu->hide();
    update();
}

void GhostPopup::enterEvent(QEnterEvent *event) {
    m_hovered = true;
    update();
    QComboBox::enterEvent(event);
}

void GhostPopup::leaveEvent(QEvent *event) {
    m_hovered = false;
    update();
    QComboBox::leaveEvent(event);
}

// ------------------------------------------------------------ KeyframeButton

KeyframeButton::KeyframeButton(QWidget *parent) : QAbstractButton(parent) {
    setObjectName(QStringLiteral("keyframeButton"));
    setAttribute(Qt::WA_Hover);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
    setFixedSize(kSize, kSize);
    setAccessibleName(QStringLiteral("Keyframe"));
}

void KeyframeButton::setHas(bool has) {
    if (m_has == has) return;
    m_has = has;
    update();
}

void KeyframeButton::setAnimated(bool animated) {
    if (m_animated == animated) return;
    m_animated = animated;
    update();
}

QString KeyframeButton::iconName() const {
    return m_has ? QStringLiteral("g-kff") : QStringLiteral("g-kf");
}

QColor KeyframeButton::iconColor() const {
    if (!isEnabled()) return col(dis);
    if (m_has || m_hovered || isDown()) return col(tokens::ink);
    return m_animated ? col(ink2) : col(ink3);
}

void KeyframeButton::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    if (isEnabled() && (m_hovered || isDown())) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(col(isDown() ? hover2 : raised));
        painter.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
    }
    icons::paint(painter, iconName(), QRectF(rect()), iconColor(), 16);
    if (hasFocus()) ui::paintFocusRing(painter, QRectF(rect()), radius::small);
}

void KeyframeButton::enterEvent(QEnterEvent *event) {
    m_hovered = true;
    update();
    QAbstractButton::enterEvent(event);
}

void KeyframeButton::leaveEvent(QEvent *event) {
    m_hovered = false;
    update();
    QAbstractButton::leaveEvent(event);
}

// ---------------------------------------------------------------- GlyphButton

void drawInspectorGlyph(QPainter &painter, GlyphButton::Glyph glyph, const QRectF &rect, const QColor &color, bool open) {
    using Glyph = GlyphButton::Glyph;
    const qreal px = std::min<qreal>(16.0, std::min(rect.width(), rect.height()));
    const auto icon = [&](const char *name, qreal size) { icons::paint(painter, QString::fromLatin1(name), rect, color, size); };
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    const QPointF c = rect.center();
    QPen pen(color, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter.setPen(pen);
    painter.setBrush(Qt::NoBrush);
    switch (glyph) {
    case Glyph::Reset:
        icon("g-reset", px);
        break;
    case Glyph::More:
        icon("g-more", px);
        break;
    case Glyph::Plus:
        painter.drawLine(c - QPointF(4.5, 0.0), c + QPointF(4.5, 0.0));
        painter.drawLine(c - QPointF(0.0, 4.5), c + QPointF(0.0, 4.5));
        break;
    case Glyph::Folder: {
        QPainterPath folder;
        folder.moveTo(c + QPointF(-6.0, 4.5));
        folder.lineTo(c + QPointF(-6.0, -4.0));
        folder.lineTo(c + QPointF(-2.0, -4.0));
        folder.lineTo(c + QPointF(-0.5, -2.5));
        folder.lineTo(c + QPointF(6.0, -2.5));
        folder.lineTo(c + QPointF(6.0, 4.5));
        folder.closeSubpath();
        painter.drawPath(folder);
        break;
    }
    case Glyph::Chevron:
        // g-chev points down; folded, it turns −90° (spec 5.1).
        painter.translate(c);
        if (!open) painter.rotate(-90);
        painter.translate(-c);
        icon("g-chev", 14);
        break;
    case Glyph::Menu:
        icon("g-chev", 14);
        break;
    case Glyph::Reverse:
        painter.setBrush(open ? QBrush(color) : QBrush(Qt::NoBrush));
        painter.drawPolygon(QPolygonF{c + QPointF(-4.5, 0.0), c + QPointF(3.5, -5.0), c + QPointF(3.5, 5.0)});
        break;
    case Glyph::Bubble:
        icon("g-comments", px);
        break;
    case Glyph::Diamond:
        icon("g-kf", px);
        break;
    case Glyph::Rectangle:
        icon("g-mrect", px);
        break;
    case Glyph::Ellipse:
        icon("g-mell", px);
        break;
    case Glyph::Pen:
        icon("g-mpen", px);
        break;
    }
    painter.restore();
}

GlyphButton::GlyphButton(Glyph glyph, QWidget *parent) : QAbstractButton(parent), m_glyph(glyph) {
    setObjectName(QStringLiteral("glyphButton"));
    setAttribute(Qt::WA_Hover);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
    setFixedSize(sizeHint());
}

QSize GlyphButton::sizeHint() const {
    switch (m_glyph) {
    case Glyph::Chevron:
    case Glyph::Diamond:
        return {14, 14};
    default:
        return {tokens::height::h24, tokens::height::h24};
    }
}

void GlyphButton::setOpen(bool open) {
    m_open = open;
    update();
}

QColor GlyphButton::discColor() const {
    if (m_glyph == Glyph::Chevron || m_glyph == Glyph::Diamond || !isEnabled()) return {};
    if (isCheckable() && isChecked()) return col(on);
    if (isDown()) return col(hover2);
    if (underMouse()) return col(raised);
    return {};
}

QColor GlyphButton::inkColor() const {
    if (!isEnabled()) return col(dis);
    if (m_glyph == Glyph::Chevron || m_glyph == Glyph::Diamond) return col(ink3);
    if (underMouse() || isDown() || (isCheckable() && isChecked())) return col(tokens::ink);
    return col(ink2);
}

void GlyphButton::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    if (const QColor disc = discColor(); disc.isValid()) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(disc);
        painter.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
        if (isCheckable() && isChecked()) ui::paintInsetLine(painter, QRectF(rect()), radius::control, col(line2));
    }
    drawInspectorGlyph(painter, m_glyph, QRectF(rect()), inkColor(), isCheckable() ? isChecked() : m_open);
    if (hasFocus()) ui::paintFocusRing(painter, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------- RowLabel

RowLabel::RowLabel(const QString &text, QWidget *parent) : QLabel(text, parent), m_full(text) {
    setObjectName(QStringLiteral("inspectorLabel"));
    setFont(tokens::font(control));
    setIndent(0);
    setMinimumWidth(0);
    setStrong(false);
    setToolTip(text);
}

void RowLabel::setStrong(bool strong) {
    setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(strong ? tokens::ink : ink2), inspectorFontCss(control)));
}

void RowLabel::resizeEvent(QResizeEvent *event) {
    // One line, ended with an ellipsis where its column cuts it.
    const QString shown = fontMetrics().elidedText(m_full, Qt::ElideRight, std::max(0, width()));
    if (shown != text()) setText(shown);
    QLabel::resizeEvent(event);
}

void RowLabel::mousePressEvent(QMouseEvent *event) {
    if (m_field && event->button() == Qt::LeftButton && !m_field->isReadOnly() && m_field->isEnabled()) {
        m_pressed = true;
        m_field->scrubPress(event->globalPosition().toPoint());
        return;
    }
    QLabel::mousePressEvent(event);
}

void RowLabel::mouseMoveEvent(QMouseEvent *event) {
    if (m_pressed && m_field) {
        m_field->scrubMove(event->globalPosition().toPoint(), event->modifiers());
        return;
    }
    QLabel::mouseMoveEvent(event);
}

void RowLabel::mouseReleaseEvent(QMouseEvent *event) {
    if (m_pressed && m_field) {
        m_pressed = false;
        m_field->scrubRelease();
        return;
    }
    QLabel::mouseReleaseEvent(event);
}

void RowLabel::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() == Qt::LeftButton) {
        emit resetRequested();
        return;
    }
    QLabel::mouseDoubleClickEvent(event);
}

// --------------------------------------------------------------- InspectorRow

InspectorRow::InspectorRow(const QString &label, QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorRow"));
    setFixedHeight(kHeight);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(kPadLeft, 0, kPadRight, 0);
    layout->setSpacing(kGap);
    m_label = new RowLabel(label);
    m_label->setFixedWidth(kLabelWidth);
    layout->addWidget(m_label);
    m_controlSlot = new QWidget;
    m_controlSlot->setObjectName(QStringLiteral("rowControls"));
    m_controls = new QHBoxLayout(m_controlSlot);
    m_controls->setContentsMargins(0, 0, 0, 0);
    m_controls->setSpacing(12);
    layout->addWidget(m_controlSlot, 1);
    m_valueSlot = new QWidget;
    m_valueSlot->setObjectName(QStringLiteral("rowValue"));
    m_valueSlot->setFixedWidth(kValueField);
    m_values = new QHBoxLayout(m_valueSlot);
    m_values->setContentsMargins(0, 0, 0, 0);
    m_values->setSpacing(0);
    m_valueSlot->hide();
    layout->addWidget(m_valueSlot);
    m_keyframe = new KeyframeButton;
    // The keyframe column keeps its place on every row, button or not.
    QSizePolicy keep = m_keyframe->sizePolicy();
    keep.setRetainSizeWhenHidden(true);
    m_keyframe->setSizePolicy(keep);
    m_keyframe->hide();
    layout->addWidget(m_keyframe);
    connect(m_label, &RowLabel::resetRequested, this, &InspectorRow::reset);
}

void InspectorRow::addValue(QWidget *value) {
    m_values->addWidget(value, 1);
    m_valueSlot->show();
}

void InspectorRow::setWide(bool wide) {
    if (m_wide == wide) return;
    m_wide = wide;
    auto *row = static_cast<QHBoxLayout *>(layout());
    if (wide) {
        // The label takes what its controls leave; they keep their size.
        m_label->setMinimumWidth(kLabelWidth);
        m_label->setMaximumWidth(QWIDGETSIZE_MAX);
        m_label->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        m_controlSlot->setSizePolicy(QSizePolicy::Maximum, QSizePolicy::Preferred);
        row->setStretchFactor(m_label, 1);
        row->setStretchFactor(m_controlSlot, 0);
    } else {
        m_label->setFixedWidth(kLabelWidth);
        m_label->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
        m_controlSlot->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Preferred);
        row->setStretchFactor(m_label, 0);
        row->setStretchFactor(m_controlSlot, 1);
    }
}

void InspectorRow::setChanged(bool changed) {
    m_changed = changed;
}

void InspectorRow::setAnimated(bool animated, const QString &tip) {
    m_animated = animated;
    if (!tip.isEmpty()) m_keyframe->setToolTip(tip);
}

void InspectorRow::setResetHandler(std::function<void()> reset) {
    m_resetHandler = std::move(reset);
}

void InspectorRow::reset() {
    if (m_resetHandler) m_resetHandler();
}

// ----------------------------------------------------------- InspectorSection

InspectorSection::InspectorSection(const QString &id, const QString &title, QWidget *parent)
    : QWidget(parent), m_id(id), m_title(title) {
    setObjectName(QStringLiteral("inspectorSection"));
    setProperty("sectionId", id);
    auto *layout = new QVBoxLayout(this);
    // The line at its foot is drawn inside it (inset 0 −1px 0).
    layout->setContentsMargins(0, 0, 0, 1);
    layout->setSpacing(0);
    m_header = new QWidget;
    m_header->setObjectName(QStringLiteral("sectionHeader"));
    m_header->setAttribute(Qt::WA_Hover);
    m_header->setFixedHeight(kHeaderHeight);
    m_header->setCursor(Qt::PointingHandCursor);
    // Tab reaches the header, and Space or Return folds or opens it.
    m_header->setFocusPolicy(Qt::TabFocus);
    m_header->installEventFilter(this);
    auto *row = new QHBoxLayout(m_header);
    row->setContentsMargins(12, 0, 8, 0);
    row->setSpacing(8);
    m_enable = new InspectorSwitch;
    m_enable->setObjectName(QStringLiteral("sectionEnabled"));
    m_enable->hide();
    connect(m_enable, &QAbstractButton::clicked, this, &InspectorSection::enableToggled);
    row->addWidget(m_enable);
    m_titleLabel = new QLabel(title);
    m_titleLabel->setObjectName(QStringLiteral("sectionTitle"));
    m_titleLabel->setFont(tokens::font(controlStrong));
    m_titleLabel->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(tokens::ink), inspectorFontCss(controlStrong)));
    m_titleLabel->setAccessibleName(title);
    row->addWidget(m_titleLabel);
    m_countLabel = new QLabel;
    m_countLabel->setObjectName(QStringLiteral("sectionCount"));
    m_countLabel->setFont(tokens::font(control));
    m_countLabel->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; %2 }").arg(css(ink3), inspectorFontCss(control)));
    m_countLabel->hide();
    row->addWidget(m_countLabel);
    row->addStretch(1);
    m_trailing = new QHBoxLayout;
    m_trailing->setSpacing(0);
    row->addLayout(m_trailing);
    m_reset = new GlyphButton(GlyphButton::Glyph::Reset);
    m_reset->setObjectName(QStringLiteral("sectionReset"));
    ui::setTip(m_reset, QStringLiteral("Reset %1").arg(title.toLower()));
    QSizePolicy keep = m_reset->sizePolicy();
    keep.setRetainSizeWhenHidden(true);
    m_reset->setSizePolicy(keep);
    m_reset->hide();
    connect(m_reset, &QAbstractButton::clicked, this, [this] {
        if (m_resetHandler) m_resetHandler();
    });
    row->addWidget(m_reset);
    m_chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    m_chevron->setObjectName(QStringLiteral("sectionChevron"));
    m_chevron->setFocusPolicy(Qt::NoFocus);
    m_chevron->setAttribute(Qt::WA_TransparentForMouseEvents);
    row->addWidget(m_chevron);
    layout->addWidget(m_header);
    m_bodyWidget = new QWidget;
    m_bodyWidget->setObjectName(QStringLiteral("sectionBody"));
    m_body = new QVBoxLayout(m_bodyWidget);
    m_body->setContentsMargins(0, 0, 0, kFoot);
    m_body->setSpacing(0);
    m_bodyWidget->hide();
    layout->addWidget(m_bodyWidget);
    updateHeader();
}

void InspectorSection::setEnable(Enable enable, const QString &tip) {
    m_enableState = enable;
    m_enable->setVisible(enable != Enable::Hidden);
    {
        const QSignalBlocker blocker(m_enable);
        m_enable->setChecked(enable == Enable::On || enable == Enable::Always);
    }
    m_enable->setEnabled(enable == Enable::On || enable == Enable::Off);
    ui::setTip(m_enable, tip.isEmpty() ? (enable == Enable::Always ? QStringLiteral("%1 is always applied").arg(m_title) : m_title) : tip);
}

void InspectorSection::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(QRect(0, height() - 1, width(), 1), col(line));
    if (m_header->hasFocus()) {
        painter.setRenderHint(QPainter::Antialiasing);
        ui::paintFocusRing(painter, QRectF(m_header->geometry()).adjusted(3, 3, -3, -3), radius::control);
    }
}

QSize InspectorSection::headerSizeHint() const {
    return m_header->sizeHint();
}

void InspectorSection::setOpen(bool open) {
    const bool was = m_open;
    m_open = open;
    m_bodyWidget->setVisible(open);
    updateHeader();
    if (was != open) emit foldChanged(open);
}

void InspectorSection::setCount(int count) {
    m_countLabel->setText(QString::number(count));
    m_countLabel->setVisible(count > 0);
}

void InspectorSection::setSummary(const Summary &summary) {
    m_summary = summary;
    updateHeader();
}

void InspectorSection::setSummary(const QString &summary) {
    setSummary(Summary{summary.isEmpty() ? QStringList{} : QStringList{summary}, {}, {}, summary});
}

QString InspectorSection::fitSummary(const Summary &summary, const QFontMetrics &metrics, int room) {
    const auto fits = [&](const QString &text) { return metrics.horizontalAdvance(text) <= room; };
    const qsizetype count = summary.items.size();
    for (qsizetype shown = count; shown >= 1; --shown) {
        QString text = summary.items.mid(0, shown).join(QStringLiteral(", "));
        if (shown < count) text += QStringLiteral(" +%1").arg(count - shown);
        text += summary.suffix;
        if (fits(text)) return text;
    }
    if (!summary.compact.isEmpty() && fits(summary.compact)) return summary.compact;
    return {};
}

void InspectorSection::addTrailing(QWidget *widget) {
    m_trailing->addWidget(widget);
}

void InspectorSection::setChanged(bool changed) {
    m_changed = changed;
    updateHeader();
}

void InspectorSection::setResetHandler(std::function<void()> reset) {
    m_resetHandler = std::move(reset);
}

void InspectorSection::updateHeader() {
    m_chevron->setOpen(m_open);
    m_bodyWidget->layout()->setContentsMargins(0, 0, 0, kFoot);
    // What changed, whole, in the header's tooltip; the reset shows while
    // the pointer is over the header, active once anything moved.
    m_header->setToolTip(m_summary.full);
    m_reset->setVisible(m_hovered && m_resetHandler != nullptr);
    m_reset->setEnabled(m_changed);
    m_header->setAccessibleName(QStringLiteral("%1, %2").arg(m_title, m_open ? QStringLiteral("expanded") : QStringLiteral("collapsed")));
}

bool InspectorSection::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_header) {
        switch (event->type()) {
        case QEvent::Enter:
        case QEvent::HoverEnter:
            m_hovered = true;
            updateHeader();
            break;
        case QEvent::Leave:
        case QEvent::HoverLeave:
            m_hovered = false;
            updateHeader();
            break;
        case QEvent::FocusIn:
        case QEvent::FocusOut:
            update();
            break;
        case QEvent::ShortcutOverride: {
            // Space is play everywhere else; on a focused header it folds.
            const int key = static_cast<QKeyEvent *>(event)->key();
            if (key == Qt::Key_Space || key == Qt::Key_Return || key == Qt::Key_Enter) event->accept();
            break;
        }
        case QEvent::KeyPress: {
            auto *key = static_cast<QKeyEvent *>(event);
            if (key->key() != Qt::Key_Space && key->key() != Qt::Key_Return && key->key() != Qt::Key_Enter) break;
            const Qt::KeyboardModifiers modifiers = key->modifiers() & ~Qt::KeypadModifier;
            setOpen(!m_open);
            emit toggled(m_open, modifiers);
            return true;
        }
        case QEvent::MouseButtonRelease: {
            auto *mouse = static_cast<QMouseEvent *>(event);
            if (mouse->button() != Qt::LeftButton) break;
            const Qt::KeyboardModifiers modifiers = mouse->modifiers();
            setOpen(!m_open);
            emit toggled(m_open, modifiers);
            return true;
        }
        default:
            break;
        }
    }
    return QWidget::eventFilter(watched, event);
}

// ---------------------------------------------------------------- ColorWheel

ColorWheel::ColorWheel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("colorWheel"));
    setFixedSize(kSize, kSize);
    setCursor(Qt::PointingHandCursor);
}

void ColorWheel::setPuck(const QPointF &unit) {
    if (m_dragging) return;
    QPointF puck = unit;
    const double length = std::hypot(puck.x(), puck.y());
    if (length > 1.0) puck /= length;
    m_puck = puck;
    update();
}

QPointF ColorWheel::puckCenter() const {
    return QPointF(kSize / 2.0, kSize / 2.0) + m_puck * kTravel;
}

void ColorWheel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF disc(0, 0, kSize, kSize);
    // The ring: an inset 2-point line in `track`.
    painter.setPen(QPen(col(tokens::track), kRing));
    painter.setBrush(Qt::NoBrush);
    painter.drawEllipse(disc.adjusted(kRing / 2.0, kRing / 2.0, -kRing / 2.0, -kRing / 2.0));
    // The crosshair, 1 point and 48 long, at 35 (the mockup's left/top 35).
    painter.setRenderHint(QPainter::Antialiasing, false);
    const int from = (kSize - kCross) / 2;
    painter.fillRect(QRect(kSize / 2 - 1, from, 1, kCross), col(tokens::track));
    painter.fillRect(QRect(from, kSize / 2 - 1, kCross, 1), col(tokens::track));
    painter.setRenderHint(QPainter::Antialiasing, true);
    const QPointF centre = puckCenter();
    paintKnobShadow(painter, centre, kPuck / 2.0);
    painter.setBrush(isEnabled() ? col(tokens::ink) : col(dis));
    painter.drawEllipse(centre, kPuck / 2.0, kPuck / 2.0);
}

void ColorWheel::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    m_pressed = true;
    m_dragging = false;
    m_pressAt = event->position();
    m_start = m_puck;
}

void ColorWheel::mouseMoveEvent(QMouseEvent *event) {
    if (!m_pressed) return;
    const QPointF delta = event->position() - m_pressAt;
    if (!m_dragging && std::hypot(delta.x(), delta.y()) < kClickSlop) return;
    m_dragging = true;
    // ⌥: a tenth as far, for fine trims.
    const double rate = event->modifiers().testFlag(Qt::AltModifier) ? 0.1 : 1.0;
    QPointF puck = m_start + delta * rate / kTravel;
    const double length = std::hypot(puck.x(), puck.y());
    if (length > 1.0) puck /= length;
    m_puck = puck;
    update();
    emit puckMoved(m_puck);
}

void ColorWheel::mouseReleaseEvent(QMouseEvent *) {
    if (!m_pressed) return;
    m_pressed = false;
    const bool dragged = m_dragging;
    m_dragging = false;
    if (dragged && m_puck != m_start) emit puckReleased(m_puck, m_start);
}

void ColorWheel::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    m_pressed = false;
    m_dragging = false;
    const QPointF start = m_puck;
    m_puck = QPointF();
    update();
    if (start != m_puck) emit puckReleased(m_puck, start);
}

// ----------------------------------------------------------------- KindSquare

KindSquare::KindSquare(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("kindSquare"));
    setFixedSize(8, 8);
}

void KindSquare::setColor(const QColor &color) {
    m_color = color;
    update();
}

void KindSquare::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    painter.setBrush(m_color);
    painter.drawRoundedRect(QRectF(rect()), 2.0, 2.0);
}

// ------------------------------------------------------------------ NameLabel

NameLabel::NameLabel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorName"));
    setFont(tokens::font(tokens::title));
    setStyleSheet(QStringLiteral("QWidget#inspectorName { %1 }").arg(inspectorFontCss(tokens::title)));
    setFixedHeight(20);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFocusPolicy(Qt::TabFocus);
}

QSize NameLabel::sizeHint() const {
    return {120, 20};
}

void NameLabel::setName(const QString &name) {
    m_name = name;
    setToolTip(m_editable ? QStringLiteral("%1 — double-click to rename").arg(name) : name);
    update();
}

void NameLabel::setEditable(bool editable) {
    m_editable = editable;
    setCursor(editable ? Qt::IBeamCursor : Qt::ArrowCursor);
    setName(m_name);
}

void NameLabel::paintEvent(QPaintEvent *) {
    if (m_editor && !m_editor->isHidden()) return;
    QPainter painter(this);
    painter.setFont(font());
    painter.setPen(col(tokens::ink));
    // Shortened in the middle, so a long name keeps both its start and the
    // end that tells takes apart.
    painter.drawText(rect(), Qt::AlignLeft | Qt::AlignVCenter, fontMetrics().elidedText(m_name, Qt::ElideMiddle, width()));
    if (hasFocus()) {
        painter.setPen(QPen(col(accent), 1.0));
        painter.drawLine(QPointF(0, height() - 0.5), QPointF(std::min<double>(width(), fontMetrics().horizontalAdvance(m_name)), height() - 0.5));
    }
}

void NameLabel::beginEditing() {
    if (!m_editable) return;
    if (!m_editor) {
        m_editor = new QLineEdit(this);
        m_editor->setObjectName(QStringLiteral("inspectorNameEditor"));
        m_editor->setFont(font());
        m_editor->setStyleSheet(editorSheet());
        m_editor->installEventFilter(this);
    }
    m_editor->setGeometry(rect().adjusted(-4, -2, 0, 2));
    m_editor->setText(m_name);
    m_editor->show();
    m_editor->setFocus(Qt::OtherFocusReason);
    m_editor->selectAll();
    update();
}

void NameLabel::endEditing(bool commit) {
    if (!m_editor || m_editor->isHidden()) return;
    const QString text = m_editor->text().trimmed();
    m_editor->hide();
    update();
    if (commit && text != m_name) emit renamed(text);
}

void NameLabel::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() == Qt::LeftButton) beginEditing();
}

void NameLabel::keyPressEvent(QKeyEvent *event) {
    if (event->key() == Qt::Key_Return || event->key() == Qt::Key_Enter) {
        beginEditing();
        return;
    }
    QWidget::keyPressEvent(event);
}

void NameLabel::resizeEvent(QResizeEvent *event) {
    if (m_editor) m_editor->setGeometry(rect().adjusted(-4, -2, 0, 2));
    QWidget::resizeEvent(event);
}

bool NameLabel::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_editor) {
        if (event->type() == QEvent::KeyPress) {
            const int key = static_cast<QKeyEvent *>(event)->key();
            if (key == Qt::Key_Escape) {
                endEditing(false);
                return true;
            }
            if (key == Qt::Key_Return || key == Qt::Key_Enter) {
                endEditing(true);
                return true;
            }
        } else if (event->type() == QEvent::FocusOut) {
            endEditing(true);
        }
    }
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
