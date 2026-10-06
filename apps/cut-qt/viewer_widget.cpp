#include "viewer_widget.h"
#include "editor_common.h"

#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>

#include <algorithm>
#include <cmath>

namespace cut {

ViewerWidget::ViewerWidget(QWidget *parent) : QWidget(parent) {
    setMinimumSize(320, 180);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMouseTracking(true);
}

void ViewerWidget::setFrame(const QImage &frame, double time) {
    m_frame = frame;
    m_time = time;
    m_scaled = QImage();
    update();
}

void ViewerWidget::clearFrame() {
    m_frame = QImage();
    m_scaled = QImage();
    update();
}

void ViewerWidget::setLiveScrub(bool live) {
    if (m_liveScrub == live) return;
    m_liveScrub = live;
    m_scaled = QImage();
    update();
}

void ViewerWidget::setCaption(const QString &caption) {
    m_caption = caption;
    update();
}

void ViewerWidget::setProjectInfo(double width, double height) {
    m_width = width;
    m_height = height;
    update();
}

QRectF ViewerWidget::videoRect() const {
    const QRectF target = QRectF(rect()).adjusted(8, 8, -8, -8);
    double aspect = 16.0 / 9.0;
    if (m_width > 0 && m_height > 0) aspect = m_width / m_height;
    else if (!m_frame.isNull()) aspect = static_cast<double>(m_frame.width()) / m_frame.height();
    QSizeF canvasSize(target.width(), target.width() / aspect);
    if (canvasSize.height() > target.height())
        canvasSize = QSizeF(target.height() * aspect, target.height());
    return QRectF(target.center() - QPointF(canvasSize.width() / 2, canvasSize.height() / 2), canvasSize);
}

QRectF ViewerWidget::captionRect() const {
    const QRectF canvas = videoRect();
    const double horizontalMargin = std::max(12.0, canvas.width() * 0.08);
    const double bottomMargin = std::max(12.0, canvas.height() * 0.08);
    const double captionHeight = std::clamp(canvas.height() * 0.12, 34.0, 64.0);
    return QRectF(canvas.left() + horizontalMargin,
                  canvas.bottom() - bottomMargin - captionHeight,
                  std::max(1.0, canvas.width() - horizontalMargin * 2.0), captionHeight);
}

void ViewerWidget::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return;
    emit seekRequested(fractionAt(event->position().x()));
}

void ViewerWidget::mouseMoveEvent(QMouseEvent *event) {
    const double fraction = fractionAt(event->position().x());
    if (m_skimming && std::abs(fraction - m_skimFraction) < 0.0005) return;
    m_skimming = true;
    m_skimFraction = fraction;
    emit scrubbed(fraction);
}

void ViewerWidget::leaveEvent(QEvent *) {
    if (!m_skimming) return;
    m_skimming = false;
    emit scrubEnded();
}

void ViewerWidget::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.fillRect(rect(), QColor("#141414"));
    const QRectF canvas = videoRect();
    QPainterPath canvasPath;
    canvasPath.addRoundedRect(canvas.adjusted(0.5, 0.5, -0.5, -0.5), 2, 2);
    p.setPen(QPen(QColor("#050505"), 1));
    p.setBrush(QColor("#050505"));
    p.drawPath(canvasPath);
    if (!m_frame.isNull()) {
        p.save();
        p.setClipPath(canvasPath);
        const qreal dpr = m_liveScrub ? 1.0 : std::max(1.0, devicePixelRatioF());
        const QSize target(std::max(1, static_cast<int>(std::round(canvas.width() * dpr))),
                           std::max(1, static_cast<int>(std::round(canvas.height() * dpr))));
        if (m_scaled.isNull() || m_scaled.size() != target || m_scaledFast != m_liveScrub) {
            m_scaled = m_frame.scaled(target, Qt::KeepAspectRatio,
                                      m_liveScrub ? Qt::FastTransformation : Qt::SmoothTransformation);
            m_scaled.setDevicePixelRatio(dpr);
            m_scaledFast = m_liveScrub;
        }
        const QPointF topLeft(canvas.center().x() - m_scaled.width() / dpr / 2.0,
                              canvas.center().y() - m_scaled.height() / dpr / 2.0);
        p.drawImage(topLeft, m_scaled);
        p.restore();
    } else {
        const QPointF center = canvas.center() - QPointF(0, 22);
        p.setPen(QColor("#363636"));
        p.setBrush(QColor("#181818"));
        p.drawEllipse(center, 20, 20);
        p.setPen(QColor("#626262"));
        p.setBrush(Qt::NoBrush);
        p.drawEllipse(center, 20, 20);
        p.setPen(Qt::NoPen);
        p.setBrush(QColor("#b1b1b1"));
        p.drawPolygon(QPolygonF{center + QPointF(-5, -8), center + QPointF(9, 0), center + QPointF(-5, 8)});
        p.setPen(QColor("#a0a0a0"));
        p.drawText(QRectF(canvas.left(), center.y() + 32, canvas.width(), 20), Qt::AlignCenter,
                   QStringLiteral("Your viewer"));
        p.setPen(QColor("#747474"));
        p.drawText(QRectF(canvas.left(), center.y() + 52, canvas.width(), 20), Qt::AlignCenter,
                   QStringLiteral("The playhead frame shows here."));
    }
    if (!m_caption.isEmpty()) {
        const QRectF caption = captionRect();
        p.setPen(Qt::NoPen);
        p.setBrush(QColor(15, 15, 15, 225));
        p.drawRoundedRect(caption, 4, 4);
        p.setPen(QColor("#676767"));
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(caption, 4, 4);
        p.setPen(QColor("#eeeeee"));
        p.drawText(caption.adjusted(10, 4, -10, -4), Qt::AlignCenter | Qt::TextWordWrap, m_caption);
    }
}

double ViewerWidget::fractionAt(int x) const {
    return std::clamp(static_cast<double>(x) / std::max(1, width()), 0.0, 1.0);
}

RegionPicker::RegionPicker(const QImage &frame, const QRectF &region, QWidget *parent)
    : QWidget(parent), m_frame(frame), m_region(region) {
    setMinimumSize(480, 270);
    setMouseTracking(true);
}

void RegionPicker::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), QColor("#141414"));
    const QRectF image = imageRect();
    painter.drawImage(image, m_frame);
    if (m_region.isEmpty()) return;
    const QRectF selected(image.x() + m_region.x() * image.width(),
                          image.y() + m_region.y() * image.height(),
                          m_region.width() * image.width(), m_region.height() * image.height());
    painter.setPen(QPen(QColor("#f2bd69"), 2));
    painter.setBrush(QColor(242, 189, 105, 35));
    painter.drawRect(selected);
}

void RegionPicker::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !imageRect().contains(event->position())) return;
    m_start = normalizedPoint(event->position());
    m_dragging = true;
    m_region = QRectF();
    update();
}

void RegionPicker::mouseMoveEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    const QPointF end = normalizedPoint(event->position());
    m_region = QRectF(m_start, end).normalized();
    update();
}

void RegionPicker::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !m_dragging) return;
    m_dragging = false;
    const QPointF end = normalizedPoint(event->position());
    m_region = QRectF(m_start, end).normalized();
    if (m_region.width() < 0.01 || m_region.height() < 0.01) m_region = QRectF();
    update();
}

QRectF RegionPicker::imageRect() const {
    const QSizeF fitted = m_frame.size().scaled(size(), Qt::KeepAspectRatio);
    return QRectF((width() - fitted.width()) / 2.0, (height() - fitted.height()) / 2.0,
                  fitted.width(), fitted.height());
}

QPointF RegionPicker::normalizedPoint(const QPointF &point) const {
    const QRectF image = imageRect();
    return QPointF(std::clamp((point.x() - image.x()) / image.width(), 0.0, 1.0),
                   std::clamp((point.y() - image.y()) / image.height(), 0.0, 1.0));
}

} // namespace cut
