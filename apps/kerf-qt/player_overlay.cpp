#include "player_overlay.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QFontMetricsF>
#include <QVariantAnimation>
#include <QWidget>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

constexpr std::array<PlayerOverlay::Control, 3> kControls{PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack,
                                                          PlayerOverlay::Control::StepForward};

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

double PlayerOverlay::footTop() const {
    // The bar at its hovered height, so the top holds still under the pointer.
    double top = m_compact ? m_rect.bottom() - kBarHoverHeight : m_rect.bottom() - kRowHeight - 2.0 - kBarHoverHeight / 2.0;
    if (controlsShown()) {
        top = std::min(top, timecodeRect().top());
        if (!m_compact)
            for (Control control : kControls) top = std::min(top, controlRect(control).top());
    }
    return top;
}

void PlayerOverlay::setControlsSuppressed(bool suppressed) {
    if (suppressed == m_suppressed) return;
    m_suppressed = suppressed;
    if (m_host) m_host->update();
}

double PlayerOverlay::fractionAt(double x) const {
    const QRectF bar = barRect();
    return std::clamp((x - bar.left()) / std::max(1.0, bar.width()), 0.0, 1.0);
}

void PlayerOverlay::paint(QPainter &) const {
    // Nothing on the picture: the transport is the scrub strip's.
}

bool PlayerOverlay::mousePress(const QPointF &position) {
    m_pointer = position;
    return false;
}

bool PlayerOverlay::mouseMove(const QPointF &position, bool) {
    m_pointer = position;
    m_inside = m_rect.contains(position);
    return false;
}

bool PlayerOverlay::mouseRelease(const QPointF &position) {
    m_pointer = position;
    return false;
}

void PlayerOverlay::leave() { m_inside = false; }

QString PlayerOverlay::toolTipAt(const QPointF &) const { return {}; }

} // namespace kerf
