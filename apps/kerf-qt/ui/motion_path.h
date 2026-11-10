#pragma once

// The curve between two keys and the curve of a clip's motion path, each a
// pure function of its arguments.
//
// curve() is kerf_model::Interp::curve in C++, and MotionPath is
// kerf_model::MotionPath: what the picture draws is what the compositor
// draws. A test on each side holds them to one table of values.

#include <QList>
#include <QPointF>
#include <QString>

#include <optional>

namespace kerf::motionpath {

// How far along a span `interp` is, `t` of the way across.
double curve(const QString &interp, double t);
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

} // namespace kerf::motionpath
