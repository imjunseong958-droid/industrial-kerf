#include "motion_path.h"

#include <QStringList>

#include <algorithm>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf::motionpath {

namespace {

// The two control points of a curve across a span: x and y from 0 to 1 at its
// ends, as the document writes a Bezier interp.
struct Handles {
    double x1 = 0.0;
    double y1 = 0.0;
    double x2 = 1.0;
    double y2 = 1.0;
};

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

} // namespace

double curve(const QString &interp, double t) {
    if (interp == QLatin1String("linear")) return t;
    if (interp == QLatin1String("hold")) return 0.0;
    if (interp == QLatin1String("ease-in")) return t * t;
    if (interp == QLatin1String("ease-out")) return 1.0 - (1.0 - t) * (1.0 - t);
    if (interp == QLatin1String("ease")) return t * t * (3.0 - 2.0 * t);
    if (const auto handles = parsed(interp)) return bezierAt(*handles, t);
    return t;
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

double MotionPath::alongOf(int index) const {
    const double total = m_distances.last();
    if (total <= 0.0) return 0.0;
    return m_distances.at(std::clamp<qsizetype>(index, 0, m_points.size() - 1) * kPathSamples) / total;
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

} // namespace kerf::motionpath
