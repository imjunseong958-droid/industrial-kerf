#pragma once

// What the graph editor reads and writes: a clip's keyframed numbers, the
// curves between their keys, and the edit that keeps a change. All of it is a
// pure function of its arguments.
//
// curve() is kerf_model::Interp::curve in C++: what the graph draws is what
// the compositor draws. A test on each side holds them to one table of values.

#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QString>

#include <optional>

namespace kerf::graph {

struct Key {
    qint64 frame = 0;
    double value = 0.0;
    QString interp;
};

// One number of a clip that has keyframes.
struct Channel {
    enum class Kind { Prop, Effect, Mask };
    Kind kind = Kind::Prop;
    QString name;
    int index = -1;  // an effect's place in the clip's stack
    QString mask;    // a mask's name
    QString label;
    QList<Key> keys;
    // The same number of the same clip, however its keys have changed.
    QString id() const;
};

// The two control points of a curve across a span: x and y from 0 to 1 at its
// ends, as the document writes a Bezier interp.
struct Handles {
    double x1 = 0.0;
    double y1 = 0.0;
    double x2 = 1.0;
    double y2 = 1.0;
};

// How far along a span `interp` is, `t` of the way across.
double curve(const QString &interp, double t);
// The curve `interp` draws, as handles; none for a hold, which has no curve.
std::optional<Handles> handlesOf(const QString &interp);
// The interp that draws `handles`, each held to what the document allows.
QString interpOf(const Handles &handles);
// The value of `keys` at `frame`; outside them, the nearest key's.
double valueAt(const QList<Key> &keys, double frame);
// What `keys` reach, curves and all, low then high.
std::pair<double, double> reach(const QList<Key> &keys);

// The numbers with keyframes in the timeline's JSON for a clip.
QList<Channel> channelsOf(const QJsonObject &clip);
// `keys` as the bridge takes them.
QJsonValue keysJson(const QList<Key> &keys);
// The op that gives `channel` of `clipId` the value `value`.
QJsonObject editOf(const QString &clipId, const Channel &channel, const QJsonValue &value);
// `keys` with key `index` moved to `frame` and `value`, kept between the keys
// either side of it and on the clip.
QList<Key> moved(QList<Key> keys, int index, qint64 frame, double value, qint64 duration);

} // namespace kerf::graph
