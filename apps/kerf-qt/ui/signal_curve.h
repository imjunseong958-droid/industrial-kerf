#pragma once

// A signal's number over its clip: how it is drawn, and how it is drawn by
// hand. A signal is a `gen=signal@1` clip whose `value` goes from 0 to 1 over
// its time (docs/shots.md, Signals); the timeline shows it along its clip, and
// a shot's graph on its node, and either lets it be drawn.
//
// curve() and keys() are the two pure parts the timeline and the graph share;
// SignalCurve is the graph's editor: the curve as it is, and a drag across it
// that draws it anew.

#include "test_friends.h"

#include <QJsonArray>
#include <QMap>
#include <QPolygonF>
#include <QRectF>
#include <QWidget>

namespace kerf {

namespace signalcurve {

// A signal's number along its clip, 0 at the bottom of `rect` and 1 at its
// top: the points of its curve, from the values the project gives for it.
QPolygonF curve(const QJsonArray &values, const QRectF &rect);
// The keys that say a number drawn by hand, `drawn` by frame of its clip: its
// ends and each frame the line between the keys kept so far would miss by more
// than `within`. Each is {at, value, interp: linear}, as an edit's `set` takes them.
QJsonArray keys(const QMap<int, double> &drawn, double within = 0.02);

} // namespace signalcurve

// A signal's curve, to look at and to draw: a drag across it sets its number
// on each frame the pointer passes, and letting go says the keys that hold it.
class SignalCurve final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit SignalCurve(QWidget *parent = nullptr);
    // What the signal comes to along its clip, and how many frames the clip has.
    void setSignal(const QJsonArray &values, int frames);
    QJsonArray values() const { return m_values; }
    int frames() const { return m_frames; }
    // Where the curve is drawn, inside the widget.
    QRectF plot() const;
    QSize sizeHint() const override;

signals:
    // A drag ended: the keys of what was drawn, by frame of the clip.
    void drawn(const QJsonArray &keys);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    void drawTo(const QPointF &position);
    QJsonArray m_values;
    int m_frames = 1;
    bool m_drawing = false;
    int m_last = -1;
    QMap<int, double> m_drawn;
};

} // namespace kerf
