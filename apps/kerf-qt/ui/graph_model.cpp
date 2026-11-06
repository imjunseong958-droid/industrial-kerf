#include "graph_model.h"

#include <QJsonArray>
#include <QStringList>

#include <algorithm>
#include <cmath>
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

} // namespace kerf::graph
