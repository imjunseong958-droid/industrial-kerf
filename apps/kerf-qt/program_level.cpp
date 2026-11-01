#include "program_level.h"
#include "editor_common.h"
#include "media_artwork.h"

#include <QJsonArray>
#include <QPainter>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// The sound a reading stands for: what played over the last 50 ms, about
// the time between two of the playback clock's ticks, so a short transient
// between ticks still moves the needle.
constexpr double kWindowSeconds = 0.05;
// Closer than this to its target the needle has arrived; a step's tail
// would otherwise keep the timer running for a change nobody could see.
constexpr double kArrived = 0.001;

// The source seconds `offset` seconds into `clip`, as the timeline reads
// them (TimelineWidget::clipSourceTime): from its in point towards its out,
// backwards from its out when it plays in reverse.
double sourceTime(const QJsonObject &clip, double offset) {
    const double duration = clip.value(QStringLiteral("duration")).toDouble();
    const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
    const double in = clip.value(QStringLiteral("in")).toDouble();
    const double out = clip.value(QStringLiteral("out")).toDouble(in + duration * std::abs(speed));
    const bool reverse = clip.value(QStringLiteral("reverse")).toBool() || speed < 0.0;
    const double rate = duration > 0.0 ? (out - in) / duration : std::abs(speed);
    return reverse ? out - offset * rate : in + offset * rate;
}

} // namespace

double programPeak(const QJsonObject &timeline, const MediaArtwork *artwork, double seconds) {
    if (!artwork) return 0.0;
    double loudest = 0.0;
    for (const QJsonValue &trackValue : timeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        if (track.value(QStringLiteral("type")).toString() != QStringLiteral("audio")) continue;
        if (track.value(QStringLiteral("muted")).toBool() || !track.value(QStringLiteral("enabled")).toBool(true)) continue;
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            // A clip switched off sounds nothing, as a track switched off does.
            if (!clip.value(QStringLiteral("enabled")).toBool(true)) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            const double duration = clip.value(QStringLiteral("duration")).toDouble();
            if (seconds < start || seconds >= start + duration) continue;
            const auto found = artwork->peaks.constFind(clip.value(QStringLiteral("sourceId")).toString());
            if (found == artwork->peaks.cend() || found->isEmpty()) continue;
            // The clip's own seconds, then its source's: its in point, speed
            // and direction decide which of its sound is playing.
            const double to = seconds - start;
            const double from = std::max(0.0, to - kWindowSeconds);
            const AudioPeaks::Bucket bucket = found->range(sourceTime(clip, from), sourceTime(clip, to));
            // Clip gain is in decibels, as the waveform draws it.
            const double gain = std::pow(10.0, clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("gain")).toDouble(0.0) / 20.0);
            loudest = std::max(loudest, std::max(std::abs(double(bucket.min)), std::abs(double(bucket.max))) * gain);
        }
    }
    return loudest;
}

double vuNeedle(double peak) {
    if (!(peak > 0.0)) return 0.0;
    // 0 VU is a sine 18 dB under digital full scale.
    const double vu = std::clamp(20.0 * std::log10(peak) + 18.0, -20.0, 3.0);
    // The printed scale is linear in voltage: -20 VU is a tenth of 0 VU's,
    // so it crowds the quiet end, as a real meter's does.
    const double volts = std::pow(10.0, vu / 20.0);
    if (volts <= 1.0) return 0.75 * (volts - 0.1) / 0.9;
    return 0.75 + 0.25 * (volts - 1.0) / (std::pow(10.0, 3.0 / 20.0) - 1.0);
}

VuMeter::VuMeter(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("programLevel"));
    setAccessibleName(QStringLiteral("Program level"));
    setToolTip(QStringLiteral("Program level"));
    // About a frame of a 60 Hz screen: the needle moves smoothly, and only
    // while there is something to show.
    m_tick.setInterval(16);
    connect(&m_tick, &QTimer::timeout, this, &VuMeter::step);
}

QSize VuMeter::sizeHint() const { return {48, 40}; }

void VuMeter::setTarget(double needle) {
    m_target = std::clamp(needle, 0.0, 1.0);
    updateTick();
}

void VuMeter::setPlaying(bool playing) {
    m_playing = playing;
    updateTick();
}

void VuMeter::advance(double seconds) {
    // A first-order lag that covers 99% of a step in kSettleSeconds, rising
    // and falling alike.
    const double tau = kSettleSeconds / std::log(100.0);
    const double before = m_needle;
    m_needle = m_target + (m_needle - m_target) * std::exp(-std::max(0.0, seconds) / tau);
    if (std::abs(m_needle - m_target) < kArrived) m_needle = m_target;
    if (m_needle != before) update();
}

void VuMeter::step() {
    // A late tick moves the needle as far as the time that passed, but a
    // stall (a modal dialog, a sleep) does not fling it.
    advance(std::min(0.1, m_clock.restart() / 1000.0));
    updateTick();
}

void VuMeter::updateTick() {
    const bool needed = m_playing || std::abs(m_needle - m_target) >= kArrived;
    if (needed && !m_tick.isActive()) {
        m_clock.start();
        m_tick.start();
    } else if (!needed && m_tick.isActive()) {
        m_tick.stop();
    }
}

void VuMeter::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    paintVuMeter(painter, rect(), m_needle);
}

} // namespace kerf
