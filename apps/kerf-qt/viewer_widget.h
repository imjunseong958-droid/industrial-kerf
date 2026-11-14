#pragma once

#include "design_tokens.h"
#include "editor_common.h"
#include "test_friends.h"

#include <QColor>
#include <QElapsedTimer>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPointer>
#include <QTransform>
#include <QVector>
#include <QWidget>

#include <functional>
#include <optional>

#include <optional>

class QGraphicsOpacityEffect;
class QVariantAnimation;

namespace kerf {

// The picture region (mockup, minimal pass): the frame, flush, on black bars
// where the region is not of its shape. Over it the caption, the QC overlap
// box, safe zones (off by default) and the comments' drawings; it captures a
// drawing on the frame, and draws the selected picture clip's masks and edits
// them: a mask tool draws a new one, a handle drag moves a corner or point, a
// drag inside a mask moves it, and Delete removes the selected one. Every
// change leaves as a typed operation. The agent HUD sits in title safe, and a
// proposal's preview chip 12 inside the picture's top left, as children that
// show and hide themselves.
class ViewerWidget final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit ViewerWidget(QWidget *parent = nullptr);
    void setFrame(const QImage &frame, double time);
    QImage frame() const { return m_frame; }
    void clearFrame();
    // The size of the frame showing, empty with none.
    QSize frameSize() const { return m_frame.size(); }
    // The next frame wipes in over this one, for a change of what is showing.
    void wipeToNextFrame();
    // A still of what the picture was: it wipes to and fro over what plays, with
    // each side named. Null takes it away.
    void setCompareFrame(const QImage &before);
    bool isComparing() const { return !m_compareFrame.isNull(); }
    void setLiveScrub(bool live);
    // While the window's regions move, the picture it has is drawn to size, smooth
    // enough to move, rather than made again in each frame.
    void setStageMoving(bool moving);
    // The engine's caption at the playhead (a `caption {text}` event).
    void setCaption(const QString &caption);
    QString caption() const { return m_caption; }
    void setProjectInfo(double width, double height);

    static constexpr int kHudWidth = 288;
    static constexpr int kHudHeight = 72;
    // The preview chip's place, inside the picture's top left.
    static constexpr int kChipInset = 12;

    // The agent HUD and the preview chip, as children; each shows and hides itself.
    void setOverlays(QWidget *hud, QWidget *chip);
    // A chip over the picture saying Kerf is at work, in the preview chip's place.
    void setWorkChip(QWidget *chip);
    // Over the window's room rather than on black: only the picture is painted.
    void setFloating(bool floating);
    QWidget *hud() const { return m_hud; }
    QWidget *previewChip() const { return m_chip; }

    // Action and title safe over the picture, every format; off by default.
    void setSafeZones(bool shown);
    bool safeZonesShown() const { return m_safeZones; }
    // The picture's width over its height.
    double pictureAspect() const { return aspect(); }
    // A vertical sequence (under 1.2): a caption wraps and balances.
    bool verticalSequence() const;
    // The frame at Fit: the largest of its shape in the region, centred.
    QRectF videoRect() const;
    // The picture as drawn: the frame at Fit, else at the zoom, centred on it
    // (clipped to the frame at Fit).
    QRectF pictureRect() const;
    // Where the caption sits: centred, its foot on title safe's (mockup-spec 3.3).
    QRectF captionRect() const;
    // The agent HUD's place: a card in title safe, at the picture's top right.
    QRect hudRect() const;
    // A click on the picture plays or pauses while this is on (Play's state).
    void setPlayEnabled(bool enabled) { m_playEnabled = enabled; }
    bool playEnabled() const { return m_playEnabled; }
    // Where the agent HUD's pop shadow reaches (empty while it is hidden).
    QRectF hudShadowRect() const;

    // The comments' drawings over the picture (view.annotations).
    void setAnnotationsShown(bool shown);
    bool annotationsShown() const { return m_annotationsShown; }
    // One comment's drawing to show now: strokes normalised to the frame
    // ({tool: pen|ellipse|rect|arrow, points: [[x, y], ...], color}) and
    // the initials on the 16 pt avatar at the first stroke's start.
    struct Annotation {
        QString comment;
        QJsonArray drawing;
        QString initials;
    };
    void setAnnotations(const QList<Annotation> &annotations);
    QList<Annotation> annotations() const { return m_annotations; }
    // The QC overlap box: `rect` normalised to the frame, `tag` its text
    // ("8 px"); an empty rect hides it (fading over 160 ms).
    void setOverlap(const QRectF &rect, const QString &tag);
    QRectF overlap() const { return m_overlapRect; }
    QString overlapTag() const { return m_overlapTag; }
    bool overlapShown() const { return m_overlapShown; }
    // The overlap box and its tag in widget points, as drawn.
    QRectF overlapBox() const;
    QRectF overlapTagRect() const;

    // Viewer zoom: 0 is Fit, else the share of the frame's own pixels (0.5,
    // 1, 2), one frame pixel a device pixel.
    void setZoom(double zoom);
    double zoom() const { return m_zoom; }

    // A drawing on the frame (the composer's Draw): the pointer draws pen
    // strokes over the picture; Return finishes (drawingFinished with the
    // strokes, normalised to the frame), Esc cancels.
    void beginDrawing();
    void finishDrawing();
    void cancelDrawingCapture();
    bool capturingDrawing() const { return m_capturing; }
    QJsonArray capturedStrokes() const;

    // The masks of picture clip `clip` as its snapshot carries them, and
    // where its picture lands on the canvas (picturePlacement). An empty
    // clip shows none and stops a mask tool; a placement that is not invertible
    // shows none and takes no clicks, and the masks are there again when it is.
    void setMasks(const QString &clip, const QJsonArray &masks, const QTransform &placement);
    void setMaskTool(MaskTool tool);
    MaskTool maskTool() const { return m_maskTool; }
    void selectMask(const QString &name);
    QString selectedMask() const { return m_selectedMask; }

    // The motion path of picture clip `clip`, as its `path=` reads: `placement`
    // takes its points to shares of the canvas (pathPlacement), and `along` is how
    // far along the clip is now. An empty clip, a path that does not read, a
    // placement that flattens it, or one that puts all of it behind the camera
    // shows none.
    //
    // Dragging a point moves it, a double-click on the curve adds one there, and
    // Delete removes the one clicked, down to the two a path needs; each writes
    // `path=` as one edit.
    void setMotionPath(const QString &clip, const QString &text, const QTransform &placement, double along);
    // A curve to show where no placement can: its places in shares of the canvas, none where it cannot
    // be seen, and where the clip is now. Empty shows none.
    void setSampledPath(const QList<std::optional<QPointF>> &places, const std::optional<QPointF> &now);
    // What it takes to drag the points of such a curve: no one map takes a drag back to a point of
    // `path=`, so each step asks `solve` what point lands there, and `reshape` for the curve that comes
    // of it. A point is dragged, and written as one edit; adding and removing them stay with a path
    // one map can show.
    struct SampledPathEdit {
        QString clip;
        QList<QPointF> points;
        std::function<std::optional<QPointF>(const QList<QPointF> &, int, const QPointF &)> solve;
        std::function<QList<std::optional<QPointF>>(const QList<QPointF> &)> curve;
        std::function<QList<std::optional<QPointF>>(const QList<QPointF> &)> handles;
    };
    void setSampledPathEdit(const SampledPathEdit &edit);
    bool showsSampledPath() const { return !m_sampledPath.isEmpty(); }
    bool showsMotionPath() const { return !m_pathClip.isEmpty(); }
    // Its points, the curve through them, and where the clip is on it, all in
    // widget points as drawn. In depth, what is not in sight (inSight) is not drawn
    // or hit, though these still say where it would map to.
    QList<QPointF> motionPathPoints() const;
    QList<QPointF> motionPathCurve() const;
    QPointF motionPathMarker() const;
    // The point Delete would remove, or -1.
    int selectedPathPoint() const { return m_pathSelected; }

    // Tests: -1 fades where something shows them (not on the offscreen
    // platform, not under reduced motion), 0 never, 1 always.
    static void setMotionOverride(int state);

signals:
    void scrubbed(double fraction);
    void scrubEnded();
    void editRequested(const QJsonArray &ops);
    // A click on the picture (see setPlayEnabled).
    void playRequested();
    // A mask was drawn or the tool was put down.
    void maskToolChanged(MaskTool tool);
    // A click chose a mask, or none.
    void maskSelected(const QString &name);
    void drawingFinished(const QJsonArray &strokes);
    void drawingCancelled();

protected:
    bool event(QEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void leaveEvent(QEvent *) override;
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void showEvent(QShowEvent *) override;

private:
    // The HUD and the chip where the picture is now.
    void placeOverlays();
    bool motionAllowed() const;
    double aspect() const;
    // Across the picture, as the player's bar is.
    double fractionAt(double x) const;
    // While a tool is out or the clip has masks, the pointer edits masks
    // instead of scrubbing or seeking.
    bool editsMasks() const;
    // Fractions of the clip's picture to widget pixels.
    QTransform pictureToWidget() const;
    std::optional<QPointF> picturePoint(const QPointF &widget) const;
    // A point of the motion path, in shares of the canvas from its origin, to
    // widget points and back.
    QPointF pathToWidget(const QPointF &point) const;
    std::optional<QPointF> pathPoint(const QPointF &widget) const;
    // The path point under the pointer, or -1.
    int pathPointAt(const QPointF &widget) const;
    // How far along the path's curve (as MotionPath::curve takes it) the pointer
    // is, within reach of it, or -1.
    double pathCurveAt(const QPointF &widget) const;
    void selectPathPoint(int point);
    void writePath();
    void removeSelectedPathPoint();
    void paintMotionPath(QPainter &p);
    // Widget points to fractions of the frame, and back.
    QPointF frameFraction(const QPointF &widget) const;
    QPointF fromFrame(const QPointF &fraction) const;
    int maskIndex(const QString &name) const;
    // The selected mask's handle under the pointer, or -1.
    int handleAt(const QPointF &widget) const;
    // The topmost mask under the pointer, the selected one first.
    QString maskAt(const QPointF &widget) const;
    void updateCursor(const QPointF &widget);
    void paintMasks(QPainter &p);
    void paintCaption(QPainter &p, bool flagged);
    void paintOverlap(QPainter &p);
    void paintAnnotations(QPainter &p);
    void paintStrokes(QPainter &p, const QJsonArray &drawing, const QString &initials);
    // The caption's text box: its lines wrapped (and balanced) on a vertical
    // picture, one line on a wide one, and the box with its padding.
    QRectF captionBox(QString *text) const;
    // Add `mask` (an outline) as the next free `k1`, `k2`, …, select it, and
    // put the tool down.
    void addMask(QJsonObject mask);
    void finishPen(bool closed);
    void cancelDrawing();
    void removeSelectedMask();

    QImage m_frame;
    // Marks what a line across the picture crossed in going `from` `to`, as shares
    // of the picture, and `reach` points either side.
    void updateLine(qreal from, qreal to, int reach);
    QImage m_scaled;
    bool m_stageMoving = false;
    bool m_floating = false;
    QImage m_compareFrame;
    qreal m_compareAt = tokens::motion::compareFrom;
    QVariantAnimation *m_compareSweep = nullptr;
    QPointer<QWidget> m_workChip;
    QImage m_wipeFrom;
    bool m_wipeArmed = false;
    qreal m_wipe = -1.0;
    QVariantAnimation *m_wipeRun = nullptr;
    double m_time = 0.0;
    double m_width = 0.0;
    double m_height = 0.0;
    QPointer<QWidget> m_hud;
    QPointer<QWidget> m_chip;
    bool m_safeZones = false;
    bool m_playEnabled = true;
    double m_skimFraction = -1.0;
    bool m_skimming = false;
    bool m_liveScrub = false;
    bool m_scaledFast = false;
    // Frames coming one after another, as they do while playing: each is drawn as it
    // comes, sampled straight onto the screen, and the last at its best once they stop.
    bool m_streaming = false;
    QImage m_straight;
    // The before still at the picture's size, made once for as long as it is watched.
    QImage m_compareScaled;
    QElapsedTimer m_frameClock;
    class QTimer *m_settle = nullptr;
    qreal m_scaledDpr = 1.0;
    QString m_caption;

    bool m_annotationsShown = true;
    QList<Annotation> m_annotations;
    QRectF m_overlapRect;
    QString m_overlapTag;
    bool m_overlapShown = false;
    double m_overlapLevel = 0.0;
    QVariantAnimation *m_overlapFade = nullptr;
    double m_zoom = 0.0;
    // The drawing being captured: finished strokes and the one in hand, in
    // fractions of the frame.
    bool m_capturing = false;
    QList<QVector<QPointF>> m_strokes;
    QVector<QPointF> m_stroke;

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

    QString m_pathClip;
    QList<QPointF> m_pathPoints;
    QTransform m_pathPlacement;
    QList<std::optional<QPointF>> m_sampledPath;
    std::optional<QPointF> m_sampledNow;
    // The points of it that a drag can take, where they are, and the one in hand.
    SampledPathEdit m_sampledEdit;
    QList<std::optional<QPointF>> m_sampledHandles;
    int m_sampledDrag = -1;
    QPointF m_sampledHeld;
    QPointF m_sampledPressed;
    bool m_sampledMoved = false;
    QList<QPointF> m_sampledBefore;
    int sampledPointAt(const QPointF &widget) const;
    QPointF sampledToWidget(const QPointF &share) const;
    double m_pathAlong = 0.0;
    int m_pathSelected = -1;
    // The point in hand, what the path was before it, and from the pointer to
    // the point, which keeps it from jumping there.
    int m_pathDrag = -1;
    QList<QPointF> m_pathBefore;
    QPointF m_pathHeld;
    QPointF m_pathPressed;
    bool m_pathMoved = false;
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
