#include "inspector_widgets.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QApplication>
#include <QCursor>
#include <QEnterEvent>
#include <QFocusEvent>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLineEdit>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QScreen>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

constexpr double kFieldRadius = 5.0;
// Pixels of drag per step.
constexpr int kScrubPixels = 2;
// A press that travels less than this is a click.
constexpr int kClickSlop = 3;

QFont sized(const QFont &base, int pixels, QFont::Weight weight = QFont::Normal, bool tabular = false) {
    QFont font = base;
    font.setPixelSize(pixels);
    font.setWeight(weight);
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
    if (tabular) font.setFeature(QFont::Tag("tnum"), 1);
#else
    Q_UNUSED(tabular)
#endif
    return font;
}

void fillField(QPainter &painter, const QRectF &rect, bool hovered, bool pressed) {
    const EditorTheme &theme = editorTheme();
    QPainterPath path;
    path.addRoundedRect(rect, kFieldRadius, kFieldRadius);
    painter.fillPath(path, theme.inspectorField);
    if (pressed) painter.fillPath(path, theme.pressed);
    else if (hovered) painter.fillPath(path, theme.hover);
}

void ringFocus(QPainter &painter, const QRectF &rect) {
    const EditorTheme &theme = editorTheme();
    painter.setPen(QPen(theme.focusHalo, 3.0));
    painter.setBrush(Qt::NoBrush);
    painter.drawRoundedRect(rect.adjusted(-1.0, -1.0, 1.0, 1.0), kFieldRadius + 1.0, kFieldRadius + 1.0);
    painter.setPen(QPen(theme.focusRing, 1.0));
    painter.drawRoundedRect(rect.adjusted(0.5, 0.5, -0.5, -0.5), kFieldRadius, kFieldRadius);
}

} // namespace

// ---------------------------------------------------------------- ScrubField

ScrubField::ScrubField(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("scrubField"));
    // Tab reaches it; a click or a drag does not take the keys from the
    // timeline (typing gives them to its editor).
    setFocusPolicy(Qt::TabFocus);
    setAttribute(Qt::WA_Hover);
    setCursor(Qt::SizeHorCursor);
    setFixedHeight(kHeight);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
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

QColor ScrubField::valueInk() const {
    // A default, mixed or flashing value is quieter; so is one whose effect
    // is switched off, in the secondary ink, which still reads at 4.5:1 on
    // the field (it still edits; the switch and the effect's name say it is
    // off).
    const EditorTheme &theme = editorTheme();
    const bool quiet = m_mixed || isDefault() || !isEnabled() || m_flash->isActive();
    return m_dimmed || quiet ? theme.textSecondary : theme.text;
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

QSize ScrubField::sizeHint() const {
    return {78, kHeight};
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
    // A timing row reads from the left, under its labels, timecode and
    // "Mixed" alike; every other value is right-aligned, its unit after it.
    return m_format.kind == NumberFormat::Kind::Timecode ? Qt::AlignLeft : Qt::AlignRight;
}

bool ScrubField::showsWhole() const {
    // As paintEvent lays it out: the value, its unit and its prefix inside
    // the padding.
    const double pad = m_readOnly ? 0.0 : m_format.kind == NumberFormat::Kind::Timecode ? 4.0 : 6.0;
    const QFontMetricsF value(sized(font(), 12, QFont::Normal, true));
    const QFontMetricsF small(sized(font(), 11));
    const QString unit = m_mixed ? QString() :
        (m_format.kind == NumberFormat::Kind::Pan || m_format.kind == NumberFormat::Kind::Timecode) ? QString()
        : (m_format.kind == NumberFormat::Kind::Gain && m_value <= m_format.min + 1e-6) ? QString()
        : m_format.kind == NumberFormat::Kind::Speed ? QStringLiteral("%") : m_format.unit;
    double needed = value.horizontalAdvance(text());
    if (!unit.isEmpty()) needed += small.horizontalAdvance(unit) + ((m_format.tightUnit || m_format.kind == NumberFormat::Kind::Speed) ? 1.0 : 3.0);
    if (!m_format.prefix.isEmpty()) needed += small.horizontalAdvance(m_format.prefix) + 4.0;
    return needed <= width() - 2.0 * pad - m_trailingInset;
}

void ScrubField::paintEvent(QPaintEvent *) {
    if (isTyping()) return;
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF box = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
    if (!m_readOnly) fillField(painter, box, m_hovered && isEnabled(), m_dragging);
    // A timecode keeps 4 points of padding (its eleven figures fill a
    // third of the column); everything else 6.
    const double pad = m_readOnly ? 0.0 : m_format.kind == NumberFormat::Kind::Timecode ? 4.0 : 6.0;
    const QRectF inner = QRectF(rect()).adjusted(pad, 0, -pad - m_trailingInset, 0);
    const QFont valueFont = sized(font(), 12, QFont::Normal, true);
    const QFont unitFont = sized(font(), 11);
    double right = inner.right();
    // The unit, after the value (% and ° sit against it).
    const QString unit = m_mixed ? QString() :
        (m_format.kind == NumberFormat::Kind::Pan || m_format.kind == NumberFormat::Kind::Timecode) ? QString()
        : (m_format.kind == NumberFormat::Kind::Gain && m_value <= m_format.min + 1e-6) ? QString()
        : m_format.kind == NumberFormat::Kind::Speed ? QStringLiteral("%") : m_format.unit;
    if (!unit.isEmpty()) {
        const QFontMetricsF metrics(unitFont);
        const double width = metrics.horizontalAdvance(unit);
        painter.setFont(unitFont);
        painter.setPen(theme.textSecondary);
        painter.drawText(QRectF(right - width, inner.top(), width, inner.height()), Qt::AlignRight | Qt::AlignVCenter, unit);
        right -= width + ((m_format.tightUnit || m_format.kind == NumberFormat::Kind::Speed) ? 1.0 : 3.0);
    }
    painter.setFont(valueFont);
    painter.setPen(valueInk());
    double left = inner.left();
    if (!m_format.prefix.isEmpty()) {
        painter.save();
        painter.setFont(unitFont);
        painter.setPen(theme.textSecondary);
        painter.drawText(QRectF(left, inner.top(), inner.width(), inner.height()), Qt::AlignLeft | Qt::AlignVCenter, m_format.prefix);
        left += QFontMetricsF(unitFont).horizontalAdvance(m_format.prefix) + 4.0;
        painter.restore();
    }
    const QRectF valueRect(left, inner.top(), std::max(0.0, right - left), inner.height());
    const QString shown = QFontMetricsF(valueFont).elidedText(text(), Qt::ElideLeft, valueRect.width());
    painter.drawText(valueRect, valueAlignment() | Qt::AlignVCenter, shown);
    if (hasFocus() && !m_readOnly) ringFocus(painter, box);
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
        m_editor->setFont(sized(font(), 12, QFont::Normal, true));
        m_editor->installEventFilter(this);
        connect(m_editor, &QLineEdit::editingFinished, this, [this] {
            if (isTyping()) endTyping(true);
        });
    }
    m_editor->setGeometry(rect().adjusted(0, 0, -m_trailingInset, 0));
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
    // From what the field says (a typed value counts), one step, clamped;
    // the text follows, selected, so the next key replaces or steps it.
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
    if (m_editor) m_editor->setGeometry(rect().adjusted(0, 0, -m_trailingInset, 0));
    QWidget::resizeEvent(event);
}

// ----------------------------------------------------------- InspectorSlider

InspectorSlider::InspectorSlider(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorSlider"));
    setFixedHeight(ScrubField::kHeight);
    setMinimumWidth(48);
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

QColor InspectorSlider::fillInk() const {
    const EditorTheme &theme = editorTheme();
    return !isEnabled() ? theme.textDisabled : m_dimmed ? theme.textSecondary : theme.switchOn;
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
    return QRectF(7.0, height() / 2.0 - 2.0, std::max(1.0, width() - 14.0), 4.0);
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
    const QRectF track = groove();
    const double detentX = track.left() + travelOf(m_detent) * track.width();
    const int x = int(std::floor(detentX));
    const int middle = int(std::lround(track.center().y()));
    // Under the knob, it runs past the knob's top and bottom so its ends
    // show; elsewhere a 1x10 mark.
    const bool under = !m_mixed && std::abs(detentX - knobCenter().x()) < kKnobRadius + 1.0;
    const int half = under ? int(std::ceil(kKnobRadius)) + kTickBeyondKnob : 5;
    return {x, middle - half, 1, 2 * half};
}

QColor InspectorSlider::knobEdge() const {
    const EditorTheme &theme = editorTheme();
    return std::abs(m_value - m_detent) > 1e-12 ? theme.sliderHandleChanged : theme.sliderHandleAtDefault;
}

void InspectorSlider::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF track = groove();
    painter.setPen(Qt::NoPen);
    painter.setBrush(theme.sliderGroove);
    painter.drawRoundedRect(track, 2.0, 2.0);
    if (!m_mixed)
        if (const QRectF fill = fillRect(); !fill.isEmpty()) {
            painter.setBrush(fillInk());
            painter.drawRoundedRect(fill, 2.0, 2.0);
        }
    // The default: a crisp one-pixel mark, drawn before the knob so
    // nothing ever lies on the knob.
    painter.setRenderHint(QPainter::Antialiasing, false);
    painter.fillRect(detentTick(), theme.inspectorDetent);
    painter.setRenderHint(QPainter::Antialiasing, true);
    if (m_mixed) return;
    // Changed: the knob's edge in full ink; at its default, grey.
    painter.setPen(QPen(knobEdge(), 1.0));
    painter.setBrush(theme.sliderHandle);
    painter.drawEllipse(knobCenter(), kKnobRadius, kKnobRadius);
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
    // On the knob: it follows the pointer. Elsewhere: nothing moves until
    // the pointer does, then the knob moves by as much.
    const double x = event->position().x();
    m_relative = std::abs(x - knobCenter().x()) > kGrab;
    m_pressX = x;
    m_pressKnob = knobCenter().x();
    // With ⌥ held from the start, fine travel from where the knob is.
    m_fine = event->modifiers().testFlag(Qt::AltModifier);
    if (m_fine) m_relative = true;
}

double InspectorSlider::dragValue(double x, Qt::KeyboardModifiers modifiers) {
    const bool fine = modifiers.testFlag(Qt::AltModifier);
    if (fine != m_fine) {
        // ⌥ pressed or let go mid-drag: on from where the knob is, at the
        // new rate.
        m_fine = fine;
        m_relative = true;
        m_pressX = x;
        m_pressKnob = knobCenter().x();
    }
    const double scale = m_fine ? 1.0 / kFine : 1.0;
    return valueAt(m_relative ? m_pressKnob + (x - m_pressX) * scale : x, scale);
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
    setFixedSize(26, 16);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
}

void InspectorSwitch::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF plate = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
    const double radius = plate.height() / 2.0;
    if (isChecked()) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(isEnabled() ? theme.switchOn : theme.textDisabled);
    } else {
        painter.setPen(QPen(theme.switchOffStroke, 1.0));
        painter.setBrush(theme.switchOff);
    }
    painter.drawRoundedRect(plate, radius, radius);
    const double knobRadius = radius - 2.0;
    const QPointF centre(isChecked() ? plate.right() - radius : plate.left() + radius, plate.center().y());
    painter.setPen(Qt::NoPen);
    if (isChecked()) {
        // A 1px shadow under the knob, so on reads as raised, not greyed.
        QColor shadow(0, 0, 0, theme.dark ? 110 : 60);
        painter.setBrush(shadow);
        painter.drawEllipse(centre + QPointF(0, 1.0), knobRadius, knobRadius);
    }
    painter.setBrush(isChecked() ? theme.switchOnKnob : theme.knob);
    painter.drawEllipse(centre, knobRadius, knobRadius);
    if (!isChecked()) {
        painter.setPen(QPen(theme.switchOffStroke, 0.75));
        painter.setBrush(Qt::NoBrush);
        painter.drawEllipse(centre, knobRadius, knobRadius);
    }
    if (hasFocus()) {
        painter.setPen(QPen(theme.focusRing, 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawRoundedRect(plate, radius, radius);
    }
}

// ------------------------------------------------------------ SegmentedField

SegmentedField::SegmentedField(const QList<Segment> &segments, QWidget *parent) : QWidget(parent), m_segments(segments) {
    setObjectName(QStringLiteral("segmentedField"));
    setFixedHeight(ScrubField::kHeight);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setCursor(Qt::PointingHandCursor);
}

void SegmentedField::setCurrent(int index) {
    m_current = index >= 0 && index < m_segments.size() ? index : -1;
    update();
}

QRect SegmentedField::segmentRect(int index) const {
    const int count = std::max(1, int(m_segments.size()));
    const double each = (width() - 4.0) / count;
    return QRectF(2.0 + index * each, 2.0, each, height() - 4.0).toRect();
}

QSize SegmentedField::sizeHint() const {
    return {78, ScrubField::kHeight};
}

void SegmentedField::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    QPainterPath track;
    track.addRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), 6.0, 6.0);
    painter.fillPath(track, theme.inspectorField);
    painter.setFont(sized(font(), 12));
    // With none chosen, a hairline at half height parts the choices, so they
    // never read as one field holding two words.
    if (m_current < 0)
        for (int index = 1; index < m_segments.size(); ++index) {
            const QRect segment = segmentRect(index);
            painter.fillRect(QRectF(segment.left() - 0.5, height() * 0.25, 1.0, height() * 0.5), theme.divider);
        }
    for (int index = 0; index < m_segments.size(); ++index) {
        const QRectF segment = QRectF(segmentRect(index));
        if (index == m_current) {
            painter.setPen(QPen(theme.buttonEdge, 1.0));
            painter.setBrush(theme.raised);
            painter.drawRoundedRect(segment.adjusted(0.5, 0.5, -0.5, -0.5), 5.0, 5.0);
        }
        const QColor ink = index == m_current ? theme.text : theme.textSecondary;
        const Segment &item = m_segments.at(index);
        if (item.glyph != Glyph::None) {
            painter.setPen(QPen(ink, 1.25, Qt::SolidLine, Qt::RoundCap));
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
            painter.setPen(ink);
            painter.drawText(segment, Qt::AlignCenter, QFontMetrics(painter.font()).elidedText(item.label, Qt::ElideRight, int(segment.width()) - 4));
        }
    }
    if (hasFocus()) ringFocus(painter, QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5));
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

// --------------------------------------------------------------- RotationDial

RotationDial::RotationDial(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("rotationDial"));
    setFixedSize(16, 16);
    setCursor(Qt::PointingHandCursor);
    setToolTip(QStringLiteral("Drag around the dial to turn; ⇧ snaps to 15°"));
}

void RotationDial::setAngle(double degrees) {
    if (m_dragging) return;
    m_angle = degrees;
    update();
}

void RotationDial::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    const QPointF c = QRectF(rect()).center();
    painter.setPen(QPen(theme.textSecondary, 1.0));
    painter.setBrush(Qt::NoBrush);
    painter.drawEllipse(c, 7.0, 7.0);
    // Clockwise from 12 o'clock, as the compositor turns.
    const double radians = qDegreesToRadians(m_angle);
    painter.setPen(QPen(theme.text, 1.5, Qt::SolidLine, Qt::RoundCap));
    painter.drawLine(c, c + QPointF(std::sin(radians) * 5.0, -std::cos(radians) * 5.0));
}

double RotationDial::angleAt(const QPointF &point, Qt::KeyboardModifiers modifiers) const {
    const QPointF d = point - QRectF(rect()).center();
    double degrees = qRadiansToDegrees(std::atan2(d.x(), -d.y()));
    // Keep the turns the clip already has.
    const double turns = std::round((m_start - degrees) / 360.0);
    degrees += turns * 360.0;
    if (modifiers & Qt::ShiftModifier) degrees = std::round(degrees / 15.0) * 15.0;
    return degrees;
}

void RotationDial::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    m_dragging = true;
    m_start = m_angle;
}

void RotationDial::mouseMoveEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    m_angle = angleAt(event->position(), event->modifiers());
    update();
    emit turned(m_angle);
}

void RotationDial::mouseReleaseEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    m_dragging = false;
    m_angle = angleAt(event->position(), event->modifiers());
    update();
    if (std::abs(m_angle - m_start) > 1e-6) emit released(m_angle, m_start);
}

// ---------------------------------------------------------------- GlyphButton

void drawInspectorGlyph(QPainter &painter, GlyphButton::Glyph glyph, const QRectF &rect, const QColor &color, bool open) {
    using Glyph = GlyphButton::Glyph;
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    const QPointF c = rect.center();
    QPen pen(color, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    painter.setPen(pen);
    painter.setBrush(Qt::NoBrush);
    switch (glyph) {
    case Glyph::Reset: {
        // ↺: an open circle with an arrowhead at its start.
        const QRectF ring(c.x() - 4.5, c.y() - 4.5, 9.0, 9.0);
        painter.drawArc(ring, 100 * 16, 290 * 16);
        const QPointF tip(c.x() - 1.0, c.y() - 4.5);
        painter.drawLine(tip, tip + QPointF(2.5, -2.0));
        painter.drawLine(tip, tip + QPointF(2.5, 2.0));
        break;
    }
    case Glyph::More:
        painter.setPen(Qt::NoPen);
        painter.setBrush(color);
        for (int dot = -1; dot <= 1; ++dot) painter.drawEllipse(c + QPointF(dot * 5.0, 0.0), 1.4, 1.4);
        break;
    case Glyph::Plus:
        painter.drawLine(c - QPointF(5.0, 0.0), c + QPointF(5.0, 0.0));
        painter.drawLine(c - QPointF(0.0, 5.0), c + QPointF(0.0, 5.0));
        break;
    case Glyph::Folder: {
        // A folder: its tab at the top left, its body under it.
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
        if (open) {
            painter.drawLine(c + QPointF(-3.5, -1.5), c + QPointF(0.0, 2.0));
            painter.drawLine(c + QPointF(0.0, 2.0), c + QPointF(3.5, -1.5));
        } else {
            painter.drawLine(c + QPointF(-1.5, -3.5), c + QPointF(2.0, 0.0));
            painter.drawLine(c + QPointF(2.0, 0.0), c + QPointF(-1.5, 3.5));
        }
        break;
    case Glyph::Menu:
        painter.drawLine(c + QPointF(-3.0, -1.0), c + QPointF(0.0, 2.0));
        painter.drawLine(c + QPointF(0.0, 2.0), c + QPointF(3.0, -1.0));
        break;
    case Glyph::Reverse: {
        // Play backwards: the play triangle turned to point left, as a
        // transport's reverse-play button draws it. Not a curved arrow,
        // which reads as undo; not two arrows, which read as swap. Off it
        // is only an outline (`open` false); on, solid.
        painter.setPen(QPen(color, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.setBrush(open ? QBrush(color) : QBrush(Qt::NoBrush));
        painter.drawPolygon(QPolygonF{c + QPointF(-4.5, 0.0), c + QPointF(3.5, -5.0), c + QPointF(3.5, 5.0)});
        break;
    }
    case Glyph::Bubble: {
        // A speech bubble: a rounded body with a tail at its lower left.
        const double w = rect.width(), h = rect.height();
        const QRectF body(rect.left() + 0.12 * w, rect.top() + 0.14 * h, 0.76 * w, 0.56 * h);
        QPainterPath bubble;
        bubble.addRoundedRect(body, 0.2 * h, 0.2 * h);
        QPainterPath tip;
        tip.moveTo(body.left() + 0.18 * w, body.bottom() - 0.5);
        tip.lineTo(body.left() + 0.14 * w, rect.top() + 0.9 * h);
        tip.lineTo(body.left() + 0.42 * w, body.bottom() - 0.5);
        painter.drawPath(bubble.united(tip));
        break;
    }
    case Glyph::Diamond: {
        painter.setPen(Qt::NoPen);
        painter.setBrush(color);
        QPolygonF diamond{c + QPointF(0, -4), c + QPointF(4, 0), c + QPointF(0, 4), c + QPointF(-4, 0)};
        painter.drawPolygon(diamond);
        break;
    }
    case Glyph::Rectangle:
        painter.drawRect(QRectF(c.x() - 5.0, c.y() - 3.5, 10.0, 7.0));
        break;
    case Glyph::Ellipse:
        painter.drawEllipse(c, 5.0, 4.0);
        break;
    case Glyph::Pen:
        drawPenNib(painter, c, color);
        break;
    }
    painter.restore();
}

GlyphButton::GlyphButton(Glyph glyph, QWidget *parent) : QAbstractButton(parent), m_glyph(glyph) {
    setObjectName(QStringLiteral("glyphButton"));
    setAttribute(Qt::WA_Hover);
    setFocusPolicy(Qt::TabFocus);
    setCursor(Qt::PointingHandCursor);
    const QSize size = sizeHint();
    setFixedSize(size);
}

QSize GlyphButton::sizeHint() const {
    switch (m_glyph) {
    case Glyph::Reset:
    case Glyph::Diamond:
        return {16, 16};
    case Glyph::Chevron:
        return {16, 24};
    default:
        return {24, 24};
    }
}

void GlyphButton::setOpen(bool open) {
    m_open = open;
    update();
}

QColor GlyphButton::discColor() const {
    // A toggle on sits on the neutral pressed disc (no accent wash: purple
    // is the focus ring's alone here).
    const EditorTheme &theme = editorTheme();
    const bool hovered = underMouse() && isEnabled() && m_glyph != Glyph::Chevron && m_glyph != Glyph::Diamond;
    const bool checked = isCheckable() && isChecked();
    if (isDown() || checked) return theme.pressed;
    if (hovered) return theme.hover;
    return {};
}

QColor GlyphButton::inkColor() const {
    const EditorTheme &theme = editorTheme();
    if (!isEnabled()) return theme.textDisabled;
    const bool hovered = underMouse() && m_glyph != Glyph::Chevron && m_glyph != Glyph::Diamond;
    // Off, a toggle is a quiet secondary glyph (full ink under the pointer).
    const bool quiet = m_glyph == Glyph::Reset || m_glyph == Glyph::Chevron || m_glyph == Glyph::Diamond ||
                       (isCheckable() && !isChecked() && !hovered);
    return quiet ? theme.textSecondary : theme.text;
}

void GlyphButton::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    if (const QColor disc = discColor(); disc.isValid()) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(disc);
        painter.drawEllipse(QRectF(rect()).adjusted(1, 1, -1, -1));
    }
    drawInspectorGlyph(painter, m_glyph, QRectF(rect()), inkColor(), isCheckable() ? isChecked() : m_open);
    if (hasFocus()) {
        painter.setPen(QPen(theme.focusRing, 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawEllipse(QRectF(rect()).adjusted(1.5, 1.5, -1.5, -1.5));
    }
}

// ------------------------------------------------------------------- RowLabel

RowLabel::RowLabel(const QString &text, QWidget *parent) : QLabel(text, parent), m_full(text) {
    setObjectName(QStringLiteral("inspectorLabel"));
    setFont(sized(font(), 12));
    setIndent(0);
    setToolTip(text);
}

void RowLabel::resizeEvent(QResizeEvent *event) {
    // One line at 12 points, ended with an ellipsis where the label column
    // cuts it; the tooltip has it whole.
    if (font().pixelSize() != 12) setFont(sized(font(), 12));
    const QString shown = fontMetrics().elidedText(m_full, Qt::ElideRight, std::max(0, width()));
    if (shown != text()) setText(shown);
    QLabel::resizeEvent(event);
}

void RowLabel::mousePressEvent(QMouseEvent *event) {
    if (m_field && event->button() == Qt::LeftButton && !m_field->isReadOnly()) {
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
    setAttribute(Qt::WA_Hover);
    setFixedHeight(kHeight);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    m_label = new RowLabel(label);
    m_label->setFixedWidth(kLabelWidth);
    layout->addWidget(m_label);
    layout->addSpacing(8);
    m_controls = new QHBoxLayout;
    m_controls->setContentsMargins(0, 0, 0, 0);
    m_controls->setSpacing(8);
    layout->addLayout(m_controls, 1);
    layout->addSpacing(4);
    auto *lane = new QWidget;
    lane->setFixedWidth(kResetLane);
    // The ↺ at the lane's right end, level with the section headers' ends.
    auto *laneLayout = new QHBoxLayout(lane);
    laneLayout->setContentsMargins(0, 0, 0, 0);
    m_reset = new GlyphButton(GlyphButton::Glyph::Reset);
    m_reset->setObjectName(QStringLiteral("rowReset"));
    m_reset->setToolTip(QStringLiteral("Reset to the default"));
    m_reset->hide();
    connect(m_reset, &QAbstractButton::clicked, this, &InspectorRow::reset);
    laneLayout->addWidget(m_reset);
    m_diamond = new QLabel;
    m_diamond->setObjectName(QStringLiteral("animatedMark"));
    m_diamond->setFixedSize(16, 16);
    m_diamond->setPixmap(glyphPixmap(QSize(16, 16), [](QPainter &p, const QColor &color) {
        drawInspectorGlyph(p, GlyphButton::Glyph::Diamond, QRectF(0, 0, 16, 16), color);
    }, editorTheme().textSecondary));
    m_diamond->hide();
    laneLayout->addWidget(m_diamond);
    layout->addWidget(lane);
    connect(m_label, &RowLabel::resetRequested, this, &InspectorRow::reset);
}

void InspectorRow::setChanged(bool changed) {
    m_changed = changed;
    updateLane();
}

void InspectorRow::setAnimated(bool animated, const QString &tip) {
    m_animated = animated;
    m_diamond->setToolTip(tip);
    updateLane();
}

void InspectorRow::setResetHandler(std::function<void()> reset) {
    m_resetHandler = std::move(reset);
}

void InspectorRow::reset() {
    if (m_resetHandler && !m_animated) m_resetHandler();
}

void InspectorRow::updateLane() {
    m_diamond->setVisible(m_animated);
    m_reset->setVisible(!m_animated && m_changed && m_hovered && m_resetHandler);
}

void InspectorRow::enterEvent(QEnterEvent *event) {
    m_hovered = true;
    updateLane();
    QWidget::enterEvent(event);
}

void InspectorRow::leaveEvent(QEvent *event) {
    m_hovered = false;
    updateLane();
    QWidget::leaveEvent(event);
}

// ----------------------------------------------------------- InspectorSection

InspectorSection::InspectorSection(const QString &id, const QString &title, QWidget *parent)
    : QWidget(parent), m_id(id), m_title(title) {
    setObjectName(QStringLiteral("inspectorSection"));
    setProperty("sectionId", id);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    m_header = new QWidget;
    m_header->setObjectName(QStringLiteral("sectionHeader"));
    m_header->setAttribute(Qt::WA_Hover);
    m_header->setAttribute(Qt::WA_StyledBackground);
    m_header->setFixedHeight(InspectorRow::kHeight);
    m_header->setCursor(Qt::PointingHandCursor);
    m_header->installEventFilter(this);
    auto *row = new QHBoxLayout(m_header);
    row->setContentsMargins(0, 1, 0, 0);
    row->setSpacing(4);
    m_chevron = new GlyphButton(GlyphButton::Glyph::Chevron);
    m_chevron->setFocusPolicy(Qt::NoFocus);
    m_chevron->setAttribute(Qt::WA_TransparentForMouseEvents);
    row->addWidget(m_chevron);
    m_titleLabel = new QLabel(title);
    m_titleLabel->setObjectName(QStringLiteral("sectionTitle"));
    m_titleLabel->setFont(sized(font(), 12, QFont::DemiBold));
    row->addWidget(m_titleLabel);
    m_countLabel = new QLabel;
    m_countLabel->setObjectName(QStringLiteral("sectionCount"));
    m_countLabel->setFont(sized(font(), 12));
    m_countLabel->hide();
    row->addWidget(m_countLabel);
    m_summaryLabel = new QLabel;
    m_summaryLabel->setObjectName(QStringLiteral("sectionSummary"));
    m_summaryLabel->setFont(sized(font(), 12));
    m_summaryLabel->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    m_summaryLabel->setMinimumWidth(0);
    m_summaryLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    row->addWidget(m_summaryLabel, 1);
    m_trailing = new QHBoxLayout;
    m_trailing->setSpacing(0);
    row->addLayout(m_trailing);
    m_reset = new GlyphButton(GlyphButton::Glyph::Reset);
    m_reset->setObjectName(QStringLiteral("sectionReset"));
    m_reset->setToolTip(QStringLiteral("Reset %1").arg(title));
    m_reset->hide();
    connect(m_reset, &QAbstractButton::clicked, this, [this] {
        if (m_resetHandler) m_resetHandler();
    });
    row->addWidget(m_reset);
    layout->addWidget(m_header);
    m_bodyWidget = new QWidget;
    m_bodyWidget->setObjectName(QStringLiteral("sectionBody"));
    m_body = new QVBoxLayout(m_bodyWidget);
    m_body->setContentsMargins(0, 0, 0, 8);
    m_body->setSpacing(0);
    m_bodyWidget->hide();
    layout->addWidget(m_bodyWidget);
    updateHeader();
}

QSize InspectorSection::headerSizeHint() const {
    return m_header->sizeHint();
}

void InspectorSection::setOpen(bool open) {
    m_open = open;
    m_bodyWidget->setVisible(open);
    updateHeader();
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

QString InspectorSection::shownSummary() const {
    return m_summaryLabel->text();
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
    // A folded section says what changed in it, in whole words; an open
    // one shows it. The whole in its tooltip.
    const int room = m_summaryLabel->contentsRect().width();
    const QString shown = room > 0 ? fitSummary(m_summary, m_summaryLabel->fontMetrics(), room) : m_summary.full;
    m_summaryLabel->setText(m_open ? QString() : shown);
    m_summaryLabel->setToolTip(m_summary.full);
    m_reset->setVisible(m_changed && m_hovered && m_resetHandler);
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
        case QEvent::Resize:
            QTimer::singleShot(0, this, &InspectorSection::updateHeader);
            break;
        case QEvent::MouseButtonRelease: {
            auto *mouse = static_cast<QMouseEvent *>(event);
            if (mouse->button() != Qt::LeftButton) break;
            const Qt::KeyboardModifiers modifiers = mouse->modifiers();
            setOpen(!m_open);
            emit toggled(m_open, modifiers);
            return true;
        }
        case QEvent::Paint: {
            // The hairline above the header.
            QPainter painter(m_header);
            painter.fillRect(QRect(0, 0, m_header->width(), 1), editorTheme().divider);
            break;
        }
        default:
            break;
        }
    }
    return QWidget::eventFilter(watched, event);
}

// -------------------------------------------------------- InspectorThumbnail

InspectorThumbnail::InspectorThumbnail(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorThumbnail"));
    setFixedSize(sizeHint());
}

QSize InspectorThumbnail::sizeHint() const {
    // Several clips stack, 4 points up and to the right each.
    const int extra = 4 * std::max(0, int(m_images.size()) - 1);
    return {64 + extra, 36 + extra};
}

void InspectorThumbnail::setImages(const QList<QImage> &images) {
    m_images = images.mid(0, 3);
    setFixedSize(sizeHint());
    update();
}

void InspectorThumbnail::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setRenderHint(QPainter::SmoothPixmapTransform);
    const int count = int(m_images.size());
    // The first clip in front, the others stacked up and to the right.
    for (int index = count - 1; index >= 0; --index) {
        const QRectF tile(index * 4.0 + 0.5, 4.0 * (count - 1 - index) + 0.5, 63.0, 35.0);
        QPainterPath path;
        path.addRoundedRect(tile, 4.0, 4.0);
        painter.save();
        painter.setClipPath(path);
        painter.fillRect(tile, theme.thumbnail);
        const QImage &image = m_images.at(index);
        if (!image.isNull()) {
            // Cover the tile, centred.
            const QSizeF scaled = QSizeF(image.size()).scaled(tile.size(), Qt::KeepAspectRatioByExpanding);
            const QRectF target(tile.center() - QPointF(scaled.width(), scaled.height()) / 2.0, scaled);
            painter.drawImage(target, image);
        }
        painter.restore();
        painter.setPen(QPen(theme.borderSubtle, 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(path);
    }
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
    // A 1px inner edge of the secondary ink at 40%, so a dark clip colour
    // still reads against the dark panel (3:1 for the shape).
    QColor edge = editorTheme().textSecondary;
    edge.setAlphaF(0.4);
    painter.setPen(QPen(edge, 1.0));
    painter.setBrush(Qt::NoBrush);
    painter.drawRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), 1.5, 1.5);
}

// ------------------------------------------------------------------ NameLabel

NameLabel::NameLabel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("inspectorName"));
    setFont(sized(font(), 14, QFont::DemiBold));
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
    painter.setPen(editorTheme().textStrong);
    painter.drawText(rect(), Qt::AlignLeft | Qt::AlignVCenter, fontMetrics().elidedText(m_name, Qt::ElideRight, width()));
    if (hasFocus()) {
        painter.setPen(QPen(editorTheme().focusRing, 1.0));
        painter.drawLine(QPointF(0, height() - 0.5), QPointF(std::min<double>(width(), fontMetrics().horizontalAdvance(m_name)), height() - 0.5));
    }
}

void NameLabel::beginEditing() {
    if (!m_editable) return;
    if (!m_editor) {
        m_editor = new QLineEdit(this);
        m_editor->setObjectName(QStringLiteral("inspectorNameEditor"));
        m_editor->setFont(font());
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
