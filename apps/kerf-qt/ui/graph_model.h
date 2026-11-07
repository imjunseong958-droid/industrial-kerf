#pragma once

// What the graph editor reads and writes: a clip's keyframed numbers, the
// curves between their keys, the edit that keeps a change, and the curve of a
// clip's motion path. All of it is a pure function of its arguments.
//
// curve() is kerf_model::Interp::curve in C++, and MotionPath is
// kerf_model::MotionPath: what the graph and the picture draw is what the
// compositor draws. A test on each side holds them to one table of values.

#include <QJsonArray>
#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QPointF>
#include <QString>

#include <limits>
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
    // What the document allows it, when the catalog says.
    double low = -std::numeric_limits<double>::infinity();
    double high = std::numeric_limits<double>::infinity();
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
// `channel` held to what the entry named for it among `defs` allows, those being
// a catalog's parameter definitions.
void holdToCatalog(Channel &channel, const QJsonArray &defs);
// `keys` as the bridge takes them.
QJsonValue keysJson(const QList<Key> &keys);
// The op that gives `channel` of `clipId` the value `value`.
QJsonObject editOf(const QString &clipId, const Channel &channel, const QJsonValue &value);
// `keys` with key `index` moved to `frame` and `value`, kept between the keys
// either side of it and on the clip.
QList<Key> moved(QList<Key> keys, int index, qint64 frame, double value, qint64 duration);
// How many frames back and forward `indices` of `keys` (in order) can all go
// by one amount, each staying between the keys not among them and on the clip,
// so that they keep their spacing. Never past 0: keys already out of place can
// stay where they are.
std::pair<qint64, qint64> roomToMove(const QList<Key> &keys, const QList<int> &indices, qint64 duration);
// `keys` with those at `indices` moved `frames` along and `value` up, each value
// held from `low` to `high`.
QList<Key> shifted(QList<Key> keys, const QList<int> &indices, qint64 frames, double value,
                   double low = -std::numeric_limits<double>::infinity(), double high = std::numeric_limits<double>::infinity());

// The points of a clip's `path=`, `x,y x,y`; none when it does not read as the
// document requires.
std::optional<QList<QPointF>> parsePath(const QString &text);
// `points` as `path=` writes them, each held to what the document allows.
QString pathText(const QList<QPointF> &points);
// Whether `path=` can say `point` as it is: pathText holds each axis apart, which
// sends anything further out somewhere else.
bool pathHolds(const QPointF &point);
// `points` with one more, on their curve `u` points along, which is as close to
// the curve they drew as a curve through points can stay: `path=` has no handles.
QList<QPointF> withPointOn(QList<QPointF> points, double aspect, double u);

// The curve through a clip's path points, on a canvas `aspect` times as wide as
// it is high.
class MotionPath {
public:
    struct Point {
        double x = 0.0;
        double y = 0.0;
        // The way it is heading, in degrees clockwise from the right.
        double heading = 0.0;
    };

    // How many samples() each span between two points has.
    static constexpr int kSamplesPerSpan = 32; // kerf_model's path::SAMPLES

    // From two points up.
    MotionPath(const QList<QPointF> &points, double aspect);
    // How far along it is at its point `index`, as `at` takes it.
    double alongOf(int index) const;
    // On the curve, `u` points along.
    QPointF curve(double u) const;
    // Where a clip `along` of the way along is, from 0 to 1 of the distance.
    Point at(double along) const;
    // The curve as a line, a few samples to each span between two points.
    QList<QPointF> samples() const;

private:
    double atDistance(double along) const;

    QList<QPointF> m_points;
    double m_aspect = 1.0;
    QList<double> m_distances;
};

} // namespace kerf::graph
