#include "graph_model.h"

#include <QJsonArray>
#include <QStringList>

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf::graph {

namespace {

constexpr double kReach = 5.0; // kerf_model's BEZIER_REACH

double cubic(double a, double b, double s) {
    const double r = 1.0 - s;
    return 3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s;
}

std::optional<Handles> parsed(const QString &interp) {
    const QStringList parts = interp.split(QLatin1Char(','));
    if (parts.size() != 4) return std::nullopt;
    Handles handles;
    double *numbers[4] = {&handles.x1, &handles.y1, &handles.x2, &handles.y2};
    for (int index = 0; index < 4; ++index) {
        bool ok = false;
        *numbers[index] = parts.at(index).toDouble(&ok);
        if (!ok) return std::nullopt;
    }
    return handles;
}

double bezierAt(const Handles &handles, double t) {
    double low = 0.0;
    double high = 1.0;
    for (int step = 0; step < 40; ++step) {
        const double middle = (low + high) / 2.0;
        if (cubic(handles.x1, handles.x2, middle) < t) low = middle;
        else high = middle;
    }
    return cubic(handles.y1, handles.y2, (low + high) / 2.0);
}

// A number or a run of keys: the keys, when it is keyed by numbers.
std::optional<QList<Key>> keysOf(const QJsonValue &param) {
    if (!param.isArray()) return std::nullopt;
    QList<Key> keys;
    for (const QJsonValue &value : param.toArray()) {
        const QJsonObject key = value.toObject();
        if (!key.value(QStringLiteral("value")).isDouble()) return std::nullopt;
        keys.append(Key{qint64(key.value(QStringLiteral("frame")).toDouble()), key.value(QStringLiteral("value")).toDouble(),
                        key.value(QStringLiteral("interp")).toString(QStringLiteral("linear"))});
    }
    if (keys.isEmpty()) return std::nullopt;
    return keys;
}

} // namespace

QString Channel::id() const {
    switch (kind) {
    case Kind::Prop: return QStringLiteral("prop/%1").arg(name);
    case Kind::Effect: return QStringLiteral("fx/%1/%2").arg(index).arg(name);
    case Kind::Mask: return QStringLiteral("mask/%1/%2").arg(mask, name);
    }
    return name;
}

double curve(const QString &interp, double t) {
    if (interp == QLatin1String("linear")) return t;
    if (interp == QLatin1String("hold")) return 0.0;
    if (interp == QLatin1String("ease-in")) return t * t;
    if (interp == QLatin1String("ease-out")) return 1.0 - (1.0 - t) * (1.0 - t);
    if (interp == QLatin1String("ease")) return t * t * (3.0 - 2.0 * t);
    if (const auto handles = parsed(interp)) return bezierAt(*handles, t);
    return t;
}

std::optional<Handles> handlesOf(const QString &interp) {
    constexpr double third = 1.0 / 3.0;
    if (interp == QLatin1String("hold")) return std::nullopt;
    if (interp == QLatin1String("ease-in")) return Handles{third, 0.0, 2.0 * third, third};
    if (interp == QLatin1String("ease-out")) return Handles{third, 2.0 * third, 2.0 * third, 1.0};
    if (interp == QLatin1String("ease")) return Handles{third, 0.0, 2.0 * third, 1.0};
    if (const auto handles = parsed(interp)) return handles;
    return Handles{third, third, 2.0 * third, 2.0 * third};
}

QString interpOf(const Handles &handles) {
    const auto text = [](double value) {
        // The document holds a curve to four places.
        const double rounded = std::round(value * 10000.0) / 10000.0;
        return QString::number(rounded == 0.0 ? 0.0 : rounded, 'g', 6);
    };
    return QStringLiteral("%1,%2,%3,%4")
        .arg(text(std::clamp(handles.x1, 0.0, 1.0)), text(std::clamp(handles.y1, -kReach, kReach)),
             text(std::clamp(handles.x2, 0.0, 1.0)), text(std::clamp(handles.y2, -kReach, kReach)));
}

double valueAt(const QList<Key> &keys, double frame) {
    if (keys.isEmpty()) return 0.0;
    if (frame <= double(keys.first().frame)) return keys.first().value;
    if (frame >= double(keys.last().frame)) return keys.last().value;
    for (qsizetype index = 0; index + 1 < keys.size(); ++index) {
        const Key &left = keys.at(index);
        const Key &right = keys.at(index + 1);
        if (frame >= double(right.frame)) continue;
        const double t = (frame - double(left.frame)) / double(right.frame - left.frame);
        return left.value + (right.value - left.value) * curve(left.interp, t);
    }
    return keys.last().value;
}

std::pair<double, double> reach(const QList<Key> &keys) {
    double low = std::numeric_limits<double>::infinity();
    double high = -low;
    for (qsizetype index = 0; index < keys.size(); ++index) {
        low = std::min(low, keys.at(index).value);
        high = std::max(high, keys.at(index).value);
        if (index + 1 == keys.size()) break;
        // A curve can go past its ends.
        constexpr int kSamples = 32;
        const Key &next = keys.at(index + 1);
        for (int step = 1; step < kSamples; ++step) {
            const double at = keys.at(index).value + (next.value - keys.at(index).value) * curve(keys.at(index).interp, double(step) / kSamples);
            low = std::min(low, at);
            high = std::max(high, at);
        }
    }
    return {low, high};
}

QList<Channel> channelsOf(const QJsonObject &clip) {
    QList<Channel> channels;
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    for (auto it = props.constBegin(); it != props.constEnd(); ++it)
        if (const auto keys = keysOf(it.value())) channels.append(Channel{Channel::Kind::Prop, it.key(), -1, {}, it.key(), *keys});
    const QJsonArray effects = clip.value(QStringLiteral("effects")).toArray();
    for (qsizetype index = 0; index < effects.size(); ++index) {
        const QJsonObject effect = effects.at(index).toObject();
        const QJsonObject params = effect.value(QStringLiteral("params")).toObject();
        const QString fx = effect.value(QStringLiteral("fx")).toString();
        for (auto it = params.constBegin(); it != params.constEnd(); ++it)
            if (const auto keys = keysOf(it.value()))
                channels.append(Channel{Channel::Kind::Effect, it.key(), int(index), {}, QStringLiteral("%1 · %2").arg(fx, it.key()), *keys});
    }
    for (const QJsonValue &value : clip.value(QStringLiteral("masks")).toArray()) {
        const QJsonObject mask = value.toObject();
        const QString name = mask.value(QStringLiteral("name")).toString();
        const QJsonObject params = mask.value(QStringLiteral("params")).toObject();
        for (auto it = params.constBegin(); it != params.constEnd(); ++it)
            if (const auto keys = keysOf(it.value()))
                channels.append(Channel{Channel::Kind::Mask, it.key(), -1, name, QStringLiteral("%1 · %2").arg(name, it.key()), *keys});
    }
    return channels;
}

void holdToCatalog(Channel &channel, const QJsonArray &defs) {
    for (const QJsonValue &value : defs) {
        const QJsonObject def = value.toObject();
        if (def.value(QStringLiteral("name")).toString() != channel.name) continue;
        channel.low = def.value(QStringLiteral("min")).toDouble(channel.low);
        channel.high = def.value(QStringLiteral("max")).toDouble(channel.high);
        return;
    }
}

QJsonValue keysJson(const QList<Key> &keys) {
    QJsonArray list;
    for (const Key &key : keys)
        list.append(QJsonObject{{QStringLiteral("at"), double(key.frame)}, {QStringLiteral("value"), key.value}, {QStringLiteral("interp"), key.interp}});
    return QJsonObject{{QStringLiteral("keys"), list}};
}

QJsonObject editOf(const QString &clipId, const Channel &channel, const QJsonValue &value) {
    const QJsonObject given{{channel.name, value}};
    switch (channel.kind) {
    case Channel::Kind::Prop:
        return {{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), clipId}, {QStringLiteral("props"), given}};
    case Channel::Kind::Effect:
        return {{QStringLiteral("op"), QStringLiteral("fx-set")}, {QStringLiteral("clip"), clipId}, {QStringLiteral("index"), channel.index},
                {QStringLiteral("params"), given}};
    case Channel::Kind::Mask:
        return {{QStringLiteral("op"), QStringLiteral("mask-set")}, {QStringLiteral("clip"), clipId}, {QStringLiteral("name"), channel.mask},
                {QStringLiteral("params"), given}};
    }
    return {};
}

QList<Key> moved(QList<Key> keys, int index, qint64 frame, double value, qint64 duration) {
    if (index < 0 || index >= keys.size()) return keys;
    const qint64 low = index > 0 ? keys.at(index - 1).frame + 1 : 0;
    const qint64 high = index + 1 < keys.size() ? keys.at(index + 1).frame - 1 : std::max<qint64>(duration - 1, low);
    keys[index].frame = std::clamp(frame, low, std::max(low, high));
    keys[index].value = value;
    return keys;
}

std::pair<qint64, qint64> roomToMove(const QList<Key> &keys, const QList<int> &indices, qint64 duration) {
    qint64 back = std::numeric_limits<qint64>::min();
    qint64 forward = std::numeric_limits<qint64>::max();
    const auto taken = [&indices](int index) { return indices.contains(index); };
    for (const int index : indices) {
        if (index < 0 || index >= keys.size()) continue;
        // The nearest key not moving with it, either way, sets its limit.
        if (index == 0) back = std::max(back, -keys.at(index).frame);
        else if (!taken(index - 1)) back = std::max(back, keys.at(index - 1).frame + 1 - keys.at(index).frame);
        if (index + 1 == keys.size()) forward = std::min(forward, duration - 1 - keys.at(index).frame);
        else if (!taken(index + 1)) forward = std::min(forward, keys.at(index + 1).frame - 1 - keys.at(index).frame);
    }
    return {std::min<qint64>(back, 0), std::max<qint64>(forward, 0)};
}

QList<Key> shifted(QList<Key> keys, const QList<int> &indices, qint64 frames, double value, double low, double high) {
    for (const int index : indices) {
        if (index < 0 || index >= keys.size()) continue;
        keys[index].frame += frames;
        // A drag means a pixel's worth: the document need not hold more than four places of it.
        const double moved = value == 0.0 ? keys.at(index).value : std::round((keys.at(index).value + value) * 10000.0) / 10000.0;
        keys[index].value = std::clamp(moved, low, high);
    }
    return keys;
}

// ------------------------------------------------------------- motion paths

namespace {

constexpr int kPathSamples = MotionPath::kSamplesPerSpan;
constexpr double kPathLimit = 10.0;

QPointF lerped(QPointF a, QPointF b, double at, double low, double high) {
    const double k = (at - low) / (high - low);
    return a + (b - a) * k;
}

} // namespace

std::optional<QList<QPointF>> parsePath(const QString &text) {
    QList<QPointF> points;
    for (const QString &point : text.split(QLatin1Char(' '), Qt::SkipEmptyParts)) {
        const QStringList parts = point.split(QLatin1Char(','));
        if (parts.size() != 2) return std::nullopt;
        bool xOk = false;
        bool yOk = false;
        const double x = parts.at(0).toDouble(&xOk);
        const double y = parts.at(1).toDouble(&yOk);
        if (!xOk || !yOk || !std::isfinite(x) || !std::isfinite(y) || std::abs(x) > kPathLimit || std::abs(y) > kPathLimit) return std::nullopt;
        points.append({x, y});
    }
    if (points.size() < 2) return std::nullopt;
    return points;
}

QString pathText(const QList<QPointF> &points) {
    const auto text = [](double value) {
        const double rounded = std::round(std::clamp(value, -kPathLimit, kPathLimit) * 10000.0) / 10000.0;
        return QString::number(rounded == 0.0 ? 0.0 : rounded, 'g', 7);
    };
    QStringList parts;
    for (const QPointF &point : points) parts << QStringLiteral("%1,%2").arg(text(point.x()), text(point.y()));
    return parts.join(QLatin1Char(' '));
}

bool pathHolds(const QPointF &point) { return std::abs(point.x()) <= kPathLimit && std::abs(point.y()) <= kPathLimit; }

QList<QPointF> withPointOn(QList<QPointF> points, double aspect, double u) {
    const QPointF on = MotionPath(points, aspect).curve(u);
    const qsizetype span = std::clamp<qsizetype>(qsizetype(std::floor(u)), 0, points.size() - 2);
    points.insert(span + 1, on);
    return points;
}

MotionPath::MotionPath(const QList<QPointF> &points, double aspect) : m_points(points), m_aspect(aspect) {
    const qsizetype total = (m_points.size() - 1) * kPathSamples;
    m_distances.reserve(total + 1);
    m_distances.append(0.0);
    QPointF last = curve(0.0);
    double walked = 0.0;
    for (qsizetype sample = 1; sample <= total; ++sample) {
        const QPointF at = curve(double(sample) / kPathSamples);
        walked += std::hypot((at.x() - last.x()) * m_aspect, at.y() - last.y());
        m_distances.append(walked);
        last = at;
    }
}

QPointF MotionPath::curve(double u) const {
    const qsizetype last = m_points.size() - 1;
    const qsizetype span = std::min<qsizetype>(qsizetype(std::floor(u)), last - 1);
    const double t = std::clamp(u - double(span), 0.0, 1.0);
    const QPointF p1 = m_points.at(span);
    const QPointF p2 = m_points.at(span + 1);
    // Past the ends, a point continuing straight on.
    const QPointF p0 = span == 0 ? 2.0 * p1 - p2 : m_points.at(span - 1);
    const QPointF p3 = span + 2 > last ? 2.0 * p2 - p1 : m_points.at(span + 2);
    // Knots a square root of the distance apart, on the canvas.
    const auto gap = [this](QPointF a, QPointF b) {
        return std::max(std::sqrt(std::hypot((a.x() - b.x()) * m_aspect, a.y() - b.y())), 1e-9);
    };
    const double t0 = 0.0;
    const double t1 = t0 + gap(p0, p1);
    const double t2 = t1 + gap(p1, p2);
    const double t3 = t2 + gap(p2, p3);
    const double at = t1 + (t2 - t1) * t;
    // Barry and Goldman's pyramid.
    const QPointF a1 = lerped(p0, p1, at, t0, t1);
    const QPointF a2 = lerped(p1, p2, at, t1, t2);
    const QPointF a3 = lerped(p2, p3, at, t2, t3);
    const QPointF b1 = lerped(a1, a2, at, t0, t2);
    const QPointF b2 = lerped(a2, a3, at, t1, t3);
    return lerped(b1, b2, at, t1, t2);
}

double MotionPath::atDistance(double along) const {
    const double total = m_distances.last();
    if (total <= 0.0) return 0.0;
    const double wanted = std::clamp(along, 0.0, 1.0) * total;
    const auto found = std::lower_bound(m_distances.cbegin(), m_distances.cend(), wanted);
    const qsizetype next = std::clamp<qsizetype>(found - m_distances.cbegin(), 1, m_distances.size() - 1);
    const double low = m_distances.at(next - 1);
    const double high = m_distances.at(next);
    const double within = high > low ? (wanted - low) / (high - low) : 0.0;
    return (double(next - 1) + within) / kPathSamples;
}

MotionPath::Point MotionPath::at(double along) const {
    const double u = atDistance(along);
    const QPointF here = curve(u);
    const double last = double(m_points.size() - 1);
    constexpr double kStep = 1e-3;
    const QPointF before = curve(std::max(u - kStep, 0.0));
    const QPointF after = curve(std::min(u + kStep, last));
    const double heading = std::atan2(after.y() - before.y(), (after.x() - before.x()) * m_aspect) * 180.0 / M_PI;
    return {here.x(), here.y(), heading};
}

QList<QPointF> MotionPath::samples() const {
    QList<QPointF> line;
    const qsizetype total = (m_points.size() - 1) * kPathSamples;
    line.reserve(total + 1);
    for (qsizetype sample = 0; sample <= total; ++sample) line.append(curve(double(sample) / kPathSamples));
    return line;
}

} // namespace kerf::graph
