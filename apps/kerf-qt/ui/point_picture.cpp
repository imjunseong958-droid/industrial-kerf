// A picture to point on (point_picture.h).

#include "point_picture.h"
#include "components.h"
#include "design_tokens.h"

#include <QMouseEvent>
#include <QPainter>

#include <algorithm>

namespace kerf {

using namespace tokens;

PointPicture::PointPicture(bool many, QWidget *parent) : QWidget(parent), m_many(many) {
    setCursor(Qt::CrossCursor);
    QSizePolicy policy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    policy.setHeightForWidth(true);
    setSizePolicy(policy);
}

void PointPicture::setPicture(const QImage &picture) {
    m_picture = picture;
    updateGeometry();
    update();
}

void PointPicture::setPoints(const QList<QPointF> &on, const QList<QPointF> &off) {
    m_on = on;
    m_off = off;
    update();
}

QJsonArray PointPicture::said(const QList<QPointF> &points) {
    QJsonArray said;
    for (const QPointF &point : points) said.append(QJsonArray{point.x(), point.y()});
    return said;
}

int PointPicture::heightForWidth(int width) const {
    if (m_picture.isNull() || m_picture.width() <= 0) return width * 9 / 16;
    return std::max(1, width * m_picture.height() / m_picture.width());
}

QSize PointPicture::sizeHint() const { return {240, heightForWidth(240)}; }

QRectF PointPicture::frame() const {
    const QRectF all(rect());
    if (m_picture.isNull()) return all;
    const QSizeF fitted = QSizeF(m_picture.size()).scaled(all.size(), Qt::KeepAspectRatio);
    return QRectF(all.center() - QPointF(fitted.width() / 2, fitted.height() / 2), fitted);
}

void PointPicture::mousePressEvent(QMouseEvent *event) {
    const QRectF inside = frame();
    if (!isEnabled() || m_picture.isNull() || !inside.contains(event->position())) return;
    const bool right = event->button() == Qt::RightButton;
    if (event->button() != Qt::LeftButton && !right) return;
    const QPointF point(std::clamp((event->position().x() - inside.left()) / inside.width(), 0.0, 1.0),
                        std::clamp((event->position().y() - inside.top()) / inside.height(), 0.0, 1.0));
    if (!m_many) {
        if (right) return;
        m_on = {point};
    } else if (right || event->modifiers().testFlag(Qt::AltModifier)) {
        m_off << point;
    } else {
        m_on << point;
    }
    update();
    emit pointed();
}

void PointPicture::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    const QRectF inside = frame();
    if (m_picture.isNull()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(well));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
        return;
    }
    p.drawImage(inside, m_picture);
    if (!isEnabled()) p.fillRect(inside, QColor(0, 0, 0, 96));
    // A point on the thing as a ring of the path's colour, one off it as a cross: told by
    // shape as well as by hue.
    const auto place = [&](const QPointF &point) { return QPointF(inside.left() + point.x() * inside.width(), inside.top() + point.y() * inside.height()); };
    for (const QPointF &point : m_on) {
        const QPointF at = place(point);
        p.setBrush(Qt::NoBrush);
        p.setPen(QPen(QColor(0, 0, 0, 160), 4));
        p.drawEllipse(at, 5, 5);
        p.setPen(QPen(color(nodes::path), 2));
        p.drawEllipse(at, 5, 5);
    }
    for (const QPointF &point : m_off) {
        const QPointF at = place(point);
        for (const auto &[ink, width] : {std::pair{QColor(0, 0, 0, 160), 4.0}, std::pair{color(status::failed), 2.0}}) {
            p.setPen(QPen(ink, width, Qt::SolidLine, Qt::RoundCap));
            p.drawLine(at + QPointF(-4, -4), at + QPointF(4, 4));
            p.drawLine(at + QPointF(-4, 4), at + QPointF(4, -4));
        }
    }
}

} // namespace kerf
