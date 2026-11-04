#include "program_level.h"
#include "design_tokens.h"
#include "editor_common.h"

#include <QJsonArray>
#include <QPainter>

#include <algorithm>
#include <cmath>

namespace kerf {

// ================================================================ ProgramMeter

namespace {

// How often the holds age while one is up: about 30 frames a second, enough
// for a line falling 20 dB a second to move smoothly.
constexpr int kTickMs = 33;

// A reading from the event: a number, or nothing for null or a missing key.
bool number(const QJsonValue &value, double *out) {
    if (!value.isDouble()) return false;
    const double v = value.toDouble();
    if (!std::isfinite(v)) return false;
    *out = v;
    return true;
}

} // namespace

ProgramMeter::ProgramMeter(QObject *parent) : QObject(parent) {
    m_tick.setInterval(kTickMs);
    connect(&m_tick, &QTimer::timeout, this, &ProgramMeter::step);
}

bool ProgramMeter::apply(const QJsonObject &event) {
    if (event.contains(QStringLiteral("type")) && event.value(QStringLiteral("type")).toString() != QStringLiteral("meter"))
        return false;
    const QJsonArray peaks = event.value(QStringLiteral("peak")).toArray();
    const QJsonArray rms = event.value(QStringLiteral("rms")).toArray();
    for (int c = 0; c < kChannels; ++c) {
        Channel &channel = m_channels[size_t(c)];
        double value = 0.0;
        channel.hasLevel = number(rms.at(c), &value);
        channel.level = channel.hasLevel ? std::min(0.0, value) : kFloorDb;
        channel.hasPeak = number(peaks.at(c), &value);
        channel.peak = channel.hasPeak ? std::min(0.0, value) : kFloorDb;
        // A louder peak raises the hold and starts its 1.5 s again.
        if (channel.hasPeak && channel.peak > kFloorDb && (!channel.hasHold || channel.peak >= channel.hold)) {
            channel.hasHold = true;
            channel.hold = channel.peak;
            channel.holdAge = 0.0;
        }
    }
    double value = 0.0;
    m_hasMomentary = number(event.value(QStringLiteral("momentary")), &value);
    m_momentary = m_hasMomentary ? value : 0.0;
    m_hasShortTerm = number(event.value(QStringLiteral("shortTerm")), &value);
    m_shortTerm = m_hasShortTerm ? value : 0.0;
    m_at = number(event.value(QStringLiteral("at")), &value) ? value : -1.0;
    updateTick();
    emit changed();
    return true;
}

void ProgramMeter::clear() {
    m_channels = {};
    m_hasMomentary = m_hasShortTerm = false;
    m_momentary = m_shortTerm = 0.0;
    m_at = -1.0;
    updateTick();
    emit changed();
}

void ProgramMeter::advance(double seconds) {
    if (!(seconds > 0.0)) return;
    bool moved = false;
    for (Channel &channel : m_channels) {
        if (!channel.hasHold) continue;
        const double before = channel.holdAge;
        channel.holdAge += seconds;
        // Only the time past the 1.5 s hold makes it fall.
        const double falling = channel.holdAge - std::max(kHoldSeconds, before);
        if (falling <= 0.0) continue;
        channel.hold -= falling * kFallDbPerSecond;
        moved = true;
        if (channel.hasPeak && channel.peak > kFloorDb && channel.hold <= channel.peak) {
            // It has come down to what plays now, and holds that.
            channel.hold = channel.peak;
            channel.holdAge = 0.0;
        } else if (channel.hold <= kFloorDb) {
            channel.hasHold = false;
            channel.hold = kFloorDb;
            channel.holdAge = 0.0;
        }
    }
    updateTick();
    if (moved) emit changed();
}

void ProgramMeter::setAutoAdvance(bool on) {
    m_auto = on;
    updateTick();
}

void ProgramMeter::step() {
    // A late tick ages the holds by the time that passed, but a stall (a
    // modal dialog, a sleep) does not throw them off the scale at once.
    advance(std::min(0.25, m_clock.restart() / 1000.0));
}

void ProgramMeter::updateTick() {
    bool held = false;
    for (const Channel &channel : m_channels) held = held || channel.hasHold;
    const bool needed = m_auto && held;
    if (needed && !m_tick.isActive()) {
        m_clock.start();
        m_tick.start();
    } else if (!needed && m_tick.isActive()) {
        m_tick.stop();
    }
}

bool ProgramMeter::hasLevel(int channel) const {
    return channel >= 0 && channel < kChannels && m_channels[size_t(channel)].hasLevel;
}
double ProgramMeter::level(int channel) const {
    return hasLevel(channel) ? m_channels[size_t(channel)].level : kFloorDb;
}
bool ProgramMeter::hasPeak(int channel) const {
    return channel >= 0 && channel < kChannels && m_channels[size_t(channel)].hasPeak;
}
double ProgramMeter::peak(int channel) const {
    return hasPeak(channel) ? m_channels[size_t(channel)].peak : kFloorDb;
}
bool ProgramMeter::hasHold(int channel) const {
    return channel >= 0 && channel < kChannels && m_channels[size_t(channel)].hasHold;
}
double ProgramMeter::hold(int channel) const {
    return hasHold(channel) ? m_channels[size_t(channel)].hold : kFloorDb;
}

bool ProgramMeter::hasData() const {
    for (const Channel &channel : m_channels)
        if (channel.hasLevel || channel.hasPeak) return true;
    return false;
}

QString ProgramMeter::momentaryText() const {
    if (!m_hasMomentary) return QStringLiteral("—");
    const double rounded = std::round(m_momentary * 10.0) / 10.0;
    const QString digits = QString::number(std::abs(rounded), 'f', 1);
    return rounded < 0.0 ? QStringLiteral("−") + digits : digits;
}

double ProgramMeter::y(double db, double height) {
    const double clamped = std::clamp(db, kFloorDb, 0.0);
    return std::round(-clamped / -kFloorDb * height);
}

QList<ProgramMeter::Segment> ProgramMeter::segments(double db, double height) {
    // As the mockup's bar(): each run from the level (capped at the run's
    // top) down to the run's foot, drawn only where the level reaches it.
    const double level = std::min(db, 0.0);
    const struct {
        double top, bottom;
        QRgb color;
    } runs[] = {{std::min(level, kWarnDb), kFloorDb, tokens::track2},
                {std::min(level, kHotDb), kWarnDb, tokens::warn},
                {level, kHotDb, tokens::neg}};
    QList<Segment> out;
    for (const auto &run : runs) {
        if (!(run.top > run.bottom)) continue;
        Segment segment;
        segment.top = y(run.top, height);
        segment.bottom = y(run.bottom, height);
        segment.color = run.color;
        if (segment.bottom > segment.top) out << segment;
    }
    return out;
}

QList<int> ProgramMeter::scaleMarks() { return {0, 6, 12, 18, 24, 40, 60}; }

} // namespace kerf
