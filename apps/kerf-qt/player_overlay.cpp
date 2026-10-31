#include "player_overlay.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QFontMetricsF>
#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QVariantAnimation>
#include <QWidget>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

constexpr std::array<PlayerOverlay::Control, 3> kControls{PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack,
                                                          PlayerOverlay::Control::StepForward};

QColor faded(QColor color, qreal amount) {
    color.setAlphaF(color.alphaF() * amount);
    return color;
}

} // namespace

PlayerOverlay::PlayerOverlay(QWidget *host) : QObject(host), m_host(host) {
    m_idle.setSingleShot(true);
    m_idle.setInterval(kIdleTimeout);
    connect(&m_idle, &QTimer::timeout, this, [this] {
        m_idleExpired = true;
        updateShown();
    });
    m_fade = new QVariantAnimation(this);
    connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_opacity = value.toReal();
        if (m_host) m_host->update();
    });
}

void PlayerOverlay::setRect(const QRectF &picture, bool compact) {
    if (picture == m_rect && compact == m_compact) return;
    m_rect = picture;
    m_compact = compact;
}

void PlayerOverlay::setTime(double time, double duration, double fps) {
    if (time == m_time && duration == m_duration && fps == m_fps) return;
    m_time = time;
    m_duration = duration;
    m_fps = fps > 0 ? fps : 24.0;
    if (m_host) m_host->update();
}

void PlayerOverlay::setPlaying(bool playing) {
    if (playing == m_playing) return;
    m_playing = playing;
    // The pointer's moment starts again when playback does.
    m_idleExpired = false;
    if (playing) m_idle.start();
    else m_idle.stop();
    updateShown();
    if (m_host) m_host->update();
}

void PlayerOverlay::setControlEnabled(Control control, bool enabled) {
    bool &slot = m_enabled[std::size_t(control)];
    if (slot == enabled) return;
    slot = enabled;
    if (m_host) m_host->update();
}

void PlayerOverlay::setControlTip(Control control, const QString &tip) { m_tips[std::size_t(control)] = tip; }

void PlayerOverlay::updateShown() {
    fadeTo(!m_playing || m_scrubbing || (m_inside && !m_idleExpired));
}

void PlayerOverlay::fadeTo(bool shown) {
    if (shown == m_target) return;
    m_target = shown;
    m_fade->stop();
    const qreal end = shown ? 1.0 : 0.0;
    if (m_fadeDuration <= 0) {
        m_opacity = end;
        if (m_host) m_host->update();
        return;
    }
    m_fade->setStartValue(m_opacity);
    m_fade->setEndValue(end);
    m_fade->setDuration(int(m_fadeDuration * std::abs(end - m_opacity)) + 1);
    m_fade->start();
}

QRectF PlayerOverlay::controlRect(Control control) const {
    const int index = int(control);
    if (m_compact) {
        // A cluster at the top left, as a short-video player puts its play button.
        return QRectF(m_rect.left() + 4 + index * kCompactHitSize, m_rect.top() + 4, kCompactHitSize, kCompactHitSize);
    }
    const double top = m_rect.bottom() - kRowHeight + (kRowHeight - kHitSize) / 2.0;
    return QRectF(m_rect.left() + 8 + index * kHitSize, top, kHitSize, kHitSize);
}

QRectF PlayerOverlay::barRect() const {
    const double height = m_overBar || m_scrubbing ? kBarHoverHeight : kBarHeight;
    if (m_compact) return QRectF(m_rect.left(), m_rect.bottom() - height, m_rect.width(), height);
    const double centre = m_rect.bottom() - kRowHeight - 2.0;
    return QRectF(m_rect.left() + 12, centre - height / 2.0, std::max(1.0, m_rect.width() - 24), height);
}

QRectF PlayerOverlay::barReach() const {
    const QRectF bar = barRect();
    const QRectF reach(bar.left(), bar.center().y() - kBarReach, bar.width(), 2.0 * kBarReach);
    return reach.intersected(m_rect);
}

QFont PlayerOverlay::timeFont(int pixelSize) const {
    QFont font = m_host ? m_host->font() : QFont();
    font.setPixelSize(pixelSize);
    font.setWeight(QFont::Medium);
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
    // Tabular figures, so the time holds still as it counts.
    font.setFeature(QFont::Tag("tnum"), 1);
#endif
    return font;
}

QRectF PlayerOverlay::timecodeRect() const {
    const QFontMetricsF metrics(timeFont(timeSize()));
    const double height = std::ceil(metrics.height());
    if (m_compact) {
        // Small, over the bar at the picture's foot.
        return QRectF(m_rect.left() + 10, m_rect.bottom() - 10 - height, std::max(1.0, m_rect.width() - 20), height);
    }
    const double left = controlRect(Control::StepForward).right() + 8;
    const double centre = m_rect.bottom() - kRowHeight / 2.0;
    return QRectF(left, centre - height / 2.0, std::max(1.0, m_rect.right() - 12 - left), height);
}

QString PlayerOverlay::timecodeText() const {
    const QString time = timelineTimecode(m_time, m_fps);
    const QString full = QStringLiteral("%1 / %2").arg(time, timelineTimecode(m_duration, m_fps));
    // Where both do not fit, the playhead's time alone.
    const QFontMetricsF metrics(timeFont(timeSize()));
    return metrics.horizontalAdvance(full) <= timecodeRect().width() ? full : time;
}

QRectF PlayerOverlay::controlsBounds() const {
    QRectF bounds = barRect();
    for (Control control : kControls) bounds |= controlRect(control);
    const QFontMetricsF metrics(timeFont(timeSize()));
    const QRectF time = timecodeRect();
    bounds |= QRectF(time.topLeft(), QSizeF(metrics.horizontalAdvance(timecodeText()), time.height()));
    return bounds;
}

double PlayerOverlay::fractionAt(double x) const {
    const QRectF bar = barRect();
    return std::clamp((x - bar.left()) / std::max(1.0, bar.width()), 0.0, 1.0);
}

void PlayerOverlay::paintGlyph(QPainter &p, Control control, const QRectF &area) const {
    const double size = m_compact ? 20.0 : kGlyphSize;
    const double unit = size / 24.0;
    const QPointF o = area.center() - QPointF(size / 2.0, size / 2.0);
    const auto at = [&](double x, double y) { return o + QPointF(x * unit, y * unit); };
    p.setPen(Qt::NoPen);
    p.setBrush(controlEnabled(control) ? editorTheme().playerInk : editorTheme().playerInkDisabled);
    switch (control) {
    case Control::Play:
        if (m_playing) {
            p.drawRoundedRect(QRectF(at(6.5, 5), at(10, 19)), unit, unit);
            p.drawRoundedRect(QRectF(at(14, 5), at(17.5, 19)), unit, unit);
        } else {
            p.drawPolygon(QPolygonF{at(7.5, 4.5), at(19.5, 12), at(7.5, 19.5)});
        }
        break;
    case Control::StepBack:
        p.drawRoundedRect(QRectF(at(5.5, 6), at(8, 18)), 0.6 * unit, 0.6 * unit);
        p.drawPolygon(QPolygonF{at(18.5, 6), at(9.5, 12), at(18.5, 18)});
        break;
    case Control::StepForward:
        p.drawRoundedRect(QRectF(at(16, 6), at(18.5, 18)), 0.6 * unit, 0.6 * unit);
        p.drawPolygon(QPolygonF{at(5.5, 6), at(14.5, 12), at(5.5, 18)});
        break;
    }
}

void PlayerOverlay::paint(QPainter &p) const {
    if (m_opacity <= 0.0 || m_rect.isEmpty()) return;
    const EditorTheme &t = editorTheme();
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setOpacity(p.opacity() * m_opacity);
    // The gradient at the foot: clear, then the shade, held under the
    // controls and the time, so they read on a white picture too.
    const double footHeight = std::max(m_rect.height() * 0.25, double(m_compact ? 48 : kRowHeight + 24));
    const double hold = m_compact ? 34.0 : kRowHeight + 6.0;
    QLinearGradient foot(0, m_rect.bottom() - footHeight, 0, m_rect.bottom());
    foot.setColorAt(0, faded(t.playerShade, 0));
    foot.setColorAt(std::clamp(1.0 - hold / footHeight, 0.0, 1.0), t.playerShade);
    foot.setColorAt(1, t.playerShade);
    p.fillRect(QRectF(m_rect.left(), m_rect.bottom() - footHeight, m_rect.width(), footHeight), foot);
    if (m_compact) {
        // A shade at the top for the cluster.
        const double topHeight = kCompactHitSize + 32.0;
        QLinearGradient top(0, m_rect.top(), 0, m_rect.top() + topHeight);
        top.setColorAt(0, t.playerTopShade);
        top.setColorAt((kCompactHitSize + 8.0) / topHeight, t.playerTopShade);
        top.setColorAt(1, faded(t.playerTopShade, 0));
        p.fillRect(QRectF(m_rect.left(), m_rect.top(), m_rect.width(), topHeight), top);
    }

    // The bar: unplayed, played to the playhead, the scrubber under the pointer.
    const QRectF bar = barRect();
    const double played = m_duration > 0 ? std::clamp(m_time / m_duration, 0.0, 1.0) : 0.0;
    p.setPen(Qt::NoPen);
    p.setBrush(t.playerTrack);
    p.drawRect(bar);
    p.setBrush(t.playerPlayed);
    p.drawRect(QRectF(bar.left(), bar.top(), bar.width() * played, bar.height()));
    const bool hovering = m_overBar || m_scrubbing;
    if (hovering) {
        const QPointF knob(bar.left() + bar.width() * played, bar.center().y());
        p.drawEllipse(knob, kScrubber / 2.0, kScrubber / 2.0);
        // The time under the pointer, over the bar.
        const QString tip = timelineTimecode(fractionAt(m_pointer.x()) * m_duration, m_fps);
        const QFont font = timeFont(11);
        const QFontMetricsF metrics(font);
        const QSizeF size(metrics.horizontalAdvance(tip) + 12, metrics.height() + 6);
        double x = std::clamp(m_pointer.x() - size.width() / 2.0, m_rect.left() + 2, m_rect.right() - 2 - size.width());
        const QRectF box(x, bar.top() - 8 - size.height(), size.width(), size.height());
        p.setBrush(t.playerTipFill);
        p.drawRoundedRect(box, 4, 4);
        p.setFont(font);
        p.setPen(t.playerInk);
        p.drawText(box, Qt::AlignCenter, tip);
    }

    // The controls, a round wash under the pointer.
    for (Control control : kControls) {
        const QRectF area = controlRect(control);
        if (controlEnabled(control) && area.contains(m_pointer) && m_inside && !hovering) {
            p.setPen(Qt::NoPen);
            p.setBrush(t.playerHover);
            p.drawEllipse(area.center(), area.width() / 2.0, area.height() / 2.0);
        }
        paintGlyph(p, control, area);
    }

    // The time, with a soft shadow under it.
    const QRectF time = timecodeRect();
    const QString text = timecodeText();
    p.setFont(timeFont(timeSize()));
    p.setPen(QColor(0, 0, 0, 128));
    p.drawText(time.translated(0, 1), Qt::AlignLeft | Qt::AlignVCenter, text);
    p.setPen(t.playerInk);
    p.drawText(time, Qt::AlignLeft | Qt::AlignVCenter, text);
    p.restore();
}

bool PlayerOverlay::mousePress(const QPointF &position) {
    if (!m_target || !m_rect.contains(position)) return false;
    m_pointer = position;
    if (barReach().contains(position)) {
        m_scrubbing = true;
        updateShown();
        emit seekRequested(fractionAt(position.x()));
        if (m_host) m_host->update();
        return true;
    }
    for (Control control : kControls) {
        if (!controlRect(control).contains(position)) continue;
        if (controlEnabled(control)) emit controlTriggered(control);
        return true;
    }
    return false;
}

bool PlayerOverlay::mouseMove(const QPointF &position, bool leftDown) {
    m_pointer = position;
    if (m_scrubbing) {
        if (leftDown) emit seekRequested(fractionAt(position.x()));
        if (m_host) m_host->update();
        return true;
    }
    const bool inside = m_rect.contains(position);
    const bool overBar = inside && barReach().contains(position);
    bool overControl = false;
    for (Control control : kControls) overControl = overControl || (inside && controlRect(control).contains(position));
    m_inside = inside;
    m_overBar = overBar;
    if (inside) {
        // Each move over the picture shows the controls for a moment more.
        m_idleExpired = false;
        if (m_playing) m_idle.start();
    }
    updateShown();
    if (m_host) m_host->update();
    return overBar || overControl;
}

bool PlayerOverlay::mouseRelease(const QPointF &position) {
    if (!m_scrubbing) return false;
    m_scrubbing = false;
    m_pointer = position;
    m_overBar = barReach().contains(position);
    updateShown();
    if (m_host) m_host->update();
    return true;
}

void PlayerOverlay::leave() {
    if (m_scrubbing) return;
    m_inside = false;
    m_overBar = false;
    updateShown();
    if (m_host) m_host->update();
}

QString PlayerOverlay::toolTipAt(const QPointF &position) const {
    if (!m_target || !m_rect.contains(position)) return {};
    for (Control control : kControls)
        if (controlRect(control).contains(position)) return controlTip(control);
    return {};
}

} // namespace kerf
