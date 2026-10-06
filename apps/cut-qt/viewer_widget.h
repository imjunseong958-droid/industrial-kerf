#pragma once

#include <QImage>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QWidget>

namespace cut {

class ViewerWidget final : public QWidget {
    Q_OBJECT
public:
    explicit ViewerWidget(QWidget *parent = nullptr);
    void setFrame(const QImage &frame, double time);
    QImage frame() const { return m_frame; }
    void clearFrame();
    void setLiveScrub(bool live);
    void setCaption(const QString &caption);
    void setProjectInfo(double width, double height);
    QRectF videoRect() const;
    QRectF captionRect() const;

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void seekRequested(double fraction);

protected:
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void leaveEvent(QEvent *) override;
    void paintEvent(QPaintEvent *) override;

private:
    double fractionAt(int x) const;

    QImage m_frame;
    QImage m_scaled;
    double m_time = 0.0;
    double m_width = 0.0;
    double m_height = 0.0;
    double m_skimFraction = -1.0;
    bool m_skimming = false;
    bool m_liveScrub = false;
    bool m_scaledFast = false;
    QString m_caption;
};

class RegionPicker final : public QWidget {
public:
    explicit RegionPicker(const QImage &frame, const QRectF &region, QWidget *parent = nullptr);
    QRectF region() const { return m_region; }

protected:
    void paintEvent(QPaintEvent *) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    QRectF imageRect() const;
    QPointF normalizedPoint(const QPointF &point) const;

    QImage m_frame;
    QRectF m_region;
    QPointF m_start;
    bool m_dragging = false;
};

} // namespace cut
