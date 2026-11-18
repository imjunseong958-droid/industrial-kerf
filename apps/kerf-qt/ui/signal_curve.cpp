// A signal's number over its clip, drawn and drawn by hand (signal_curve.h).

#include "signal_curve.h"
#include "components.h"
#include "design_tokens.h"

#include <QJsonObject>
#include <QMouseEvent>
#include <QPainter>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

namespace signalcurve {

QPolygonF curve(const QJsonArray &values, const QRectF &rect) {
    QPolygonF curve;
    if (values.isEmpty() || rect.width() <= 0.0 || rect.height() <= 0.0) return curve;
    const int last = int(values.size()) - 1;
    for (int index = 0; index <= last; ++index) {
        const double along = last == 0 ? 0.0 : double(index) / double(last);
        const double value = std::clamp(values.at(index).toDouble(), 0.0, 1.0);
        curve << QPointF(rect.left() + along * rect.width(), rect.bottom() - value * rect.height());
    }
    // One value is a level across the whole clip.
    if (last == 0) curve << QPointF(rect.right(), curve.first().y());
    return curve;
}

QJsonArray keys(const QMap<int, double> &drawn, double within) {
    QJsonArray keys;
    if (drawn.isEmpty()) return keys;
    const QList<int> frames = drawn.keys();
    const auto key = [&](int frame) {
        return QJsonObject{{QStringLiteral("at"), frame},
                           {QStringLiteral("value"), std::round(drawn.value(frame) * 1000.0) / 1000.0},
                           {QStringLiteral("interp"), QStringLiteral("linear")}};
    };
    // Kept from the first frame on: a frame is a key when the line from the last key to
    // the next frame would pass some frame between by more than `within`.
    int kept = 0;
    keys.append(key(frames.at(0)));
    for (int next = 2; next < frames.size(); ++next) {
        const double a = drawn.value(frames.at(kept));
        const double b = drawn.value(frames.at(next));
        const double span = frames.at(next) - frames.at(kept);
        bool misses = false;
        for (int between = kept + 1; between < next && !misses; ++between) {
            const double on = a + (b - a) * (frames.at(between) - frames.at(kept)) / span;
            misses = std::abs(on - drawn.value(frames.at(between))) > within;
        }
        if (misses) {
            kept = next - 1;
            keys.append(key(frames.at(kept)));
        }
    }
    if (frames.size() > 1) keys.append(key(frames.last()));
    return keys;
}

} // namespace signalcurve

SignalCurve::SignalCurve(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("signalCurve"));
    setAccessibleName(QStringLiteral("Signal"));
    setCursor(Qt::CrossCursor);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    ui::setTip(this, QStringLiteral("Drag across to draw the signal: 0 at the foot, 1 at the top"));
}

void SignalCurve::setSignal(const QJsonArray &values, int frames) {
    m_values = values;
    m_frames = std::max(1, frames);
    // What was being drawn was of the signal as it was.
    if (!m_drawing) m_drawn.clear();
    update();
}

QSize SignalCurve::sizeHint() const { return {240, 112}; }

QRectF SignalCurve::plot() const { return QRectF(rect()).adjusted(8, 8, -8, -8); }

void SignalCurve::drawTo(const QPointF &position) {
    const QRectF inside = plot();
    if (inside.width() <= 0.0 || inside.height() <= 0.0) return;
    const int last = m_frames - 1;
    const int frame = std::clamp(int(std::lround((position.x() - inside.left()) / inside.width() * last)), 0, last);
    const double value = std::clamp(1.0 - (position.y() - inside.top()) / inside.height(), 0.0, 1.0);
    // Every frame the pointer passed since the last one is on the line between the two.
    if (m_last >= 0 && m_last != frame) {
        const double start = m_drawn.value(m_last);
        const int step = frame > m_last ? 1 : -1;
        for (int at = m_last + step; at != frame; at += step)
            m_drawn.insert(at, start + (value - start) * double(at - m_last) / double(frame - m_last));
    }
    m_drawn.insert(frame, value);
    m_last = frame;
    update();
}

void SignalCurve::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !isEnabled()) return;
    m_drawing = true;
    m_drawn.clear();
    m_last = -1;
    drawTo(event->position());
}

void SignalCurve::mouseMoveEvent(QMouseEvent *event) {
    if (m_drawing) drawTo(event->position());
}

void SignalCurve::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !m_drawing) return;
    m_drawing = false;
    // One point is a click, not a drawing; what was drawn stays shown until
    // the project has it and says its curve again.
    if (m_drawn.size() >= 2) emit drawn(signalcurve::keys(m_drawn));
    else m_drawn.clear();
    update();
}

void SignalCurve::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF card = QRectF(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(well));
    p.drawRoundedRect(card, radius::control, radius::control);
    ui::paintInsetLine(p, card, radius::control, color(line2));
    const QRectF inside = plot();
    // Its floor, its middle and its top, as hairlines.
    p.setPen(QPen(color(line), 1));
    for (const qreal share : {0.0, 0.5, 1.0}) {
        const qreal y = std::round(inside.bottom() - inside.height() * share) + 0.5;
        p.drawLine(QPointF(inside.left(), y), QPointF(inside.right(), y));
    }
    const QColor ink = color(isEnabled() ? tokens::nodes::signal : dis);
    QPolygonF line = signalcurve::curve(m_values, inside);
    if (!line.isEmpty()) {
        QPolygonF under = line;
        under << QPointF(inside.right(), inside.bottom()) << QPointF(inside.left(), inside.bottom());
        QColor wash = ink;
        wash.setAlphaF(0.16);
        p.setPen(Qt::NoPen);
        p.setBrush(wash);
        p.drawPolygon(under);
        p.setBrush(Qt::NoBrush);
        QColor was = ink;
        // While another is drawn over it, what it was falls back.
        if (!m_drawn.isEmpty()) was.setAlphaF(0.45);
        p.setPen(QPen(was, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p.drawPolyline(line);
    }
    if (m_drawn.isEmpty()) return;
    QPolygonF drawn;
    const double last = std::max(1, m_frames - 1);
    for (auto at = m_drawn.constBegin(); at != m_drawn.constEnd(); ++at)
        drawn << QPointF(inside.left() + inside.width() * at.key() / last, inside.bottom() - inside.height() * at.value());
    p.setPen(QPen(color(tokens::ink), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    if (drawn.size() == 1) p.drawPoint(drawn.first());
    else p.drawPolyline(drawn);
}

} // namespace kerf
