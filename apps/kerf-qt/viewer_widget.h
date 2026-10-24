#pragma once

#include "editor_common.h"

#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QTransform>
#include <QVector>
#include <QWidget>

namespace kerf {

// The Program monitor. It also draws the selected picture clip's masks over
// the picture and edits them: a mask tool draws a new one, a handle drag
// moves a corner or point, a drag inside a mask moves it, and Delete
// removes the selected one. Every change leaves as a typed operation.
class ViewerWidget final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
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

    // The masks of picture clip `clip` as its snapshot carries them, and
    // where its picture lands on the canvas (picturePlacement). An empty
    // clip shows none and stops a mask tool.
    void setMasks(const QString &clip, const QJsonArray &masks, const QTransform &placement);
    void setMaskTool(MaskTool tool);
    MaskTool maskTool() const { return m_maskTool; }
    void selectMask(const QString &name);
    QString selectedMask() const { return m_selectedMask; }

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void seekRequested(double fraction);
    void editRequested(const QJsonArray &ops);
    // A mask was drawn or the tool was put down.
    void maskToolChanged(MaskTool tool);
    // A click chose a mask, or none.
    void maskSelected(const QString &name);

protected:
    bool event(QEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void leaveEvent(QEvent *) override;
    void paintEvent(QPaintEvent *) override;

private:
    double fractionAt(int x) const;
    // While a tool is out or the clip has masks, the pointer edits masks
    // instead of scrubbing or seeking.
    bool editsMasks() const;
    // Fractions of the clip's picture to widget pixels.
    QTransform pictureToWidget() const;
    QPointF picturePoint(const QPointF &widget) const;
    int maskIndex(const QString &name) const;
    // The selected mask's handle under the pointer, or -1.
    int handleAt(const QPointF &widget) const;
    // The topmost mask under the pointer, the selected one first.
    QString maskAt(const QPointF &widget) const;
    void updateCursor(const QPointF &widget);
    void paintMasks(QPainter &p);
    // Add `mask` (an outline) as the next free `k1`, `k2`, …, select it, and
    // put the tool down.
    void addMask(QJsonObject mask);
    void finishPen(bool closed);
    void cancelDrawing();
    void removeSelectedMask();

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

    QString m_maskClip;
    QJsonArray m_masks;
    QTransform m_placement;
    MaskTool m_maskTool = MaskTool::None;
    QString m_selectedMask;
    // A rectangle or ellipse being dragged out, in picture fractions.
    bool m_drawing = false;
    QPointF m_drawStart;
    QPointF m_drawEnd;
    // The pen's points so far, in picture fractions, and the pointer.
    QVector<QPointF> m_penPoints;
    QPointF m_pointer;
    // A handle (its index), the whole mask (kMoveMask), or nothing being
    // dragged, from where and from which outline.
    static constexpr int kMoveMask = -2;
    int m_dragHandle = -1;
    QPointF m_dragFrom;
    QJsonObject m_dragOriginal;
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

} // namespace kerf
