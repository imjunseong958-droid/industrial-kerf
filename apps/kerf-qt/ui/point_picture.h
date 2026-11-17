#pragma once

// A picture to point on: the first frame of a clip, as its sequence draws it,
// and the points a person presses on it. A shot's graph shows it where a clip
// is read from footage (docs/shots.md, From footage): one point for a clip to
// follow, or points on a thing to cut out, with points off it.

#include "test_friends.h"

#include <QImage>
#include <QJsonArray>
#include <QList>
#include <QPointF>
#include <QRectF>
#include <QWidget>

namespace kerf {

class PointPicture final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // With `many`, points gather until they are taken: a press is on the
    // thing, and one with Alt held, or the other button, is off it. Without,
    // a press is the one point.
    explicit PointPicture(bool many, QWidget *parent = nullptr);
    void setPicture(const QImage &picture);
    void setPoints(const QList<QPointF> &on, const QList<QPointF> &off);
    // The points, each across the picture and down it from 0 to 1.
    QList<QPointF> on() const { return m_on; }
    QList<QPointF> off() const { return m_off; }
    static QJsonArray said(const QList<QPointF> &points);
    // Where the picture is drawn, inside the widget.
    QRectF frame() const;
    QSize sizeHint() const override;
    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override;

signals:
    // A point was pressed: on() and off() have it.
    void pointed();

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;

private:
    bool m_many;
    QImage m_picture;
    QList<QPointF> m_on;
    QList<QPointF> m_off;
};

} // namespace kerf
