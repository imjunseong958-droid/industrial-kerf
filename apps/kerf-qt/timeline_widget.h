#pragma once

#include "editor_common.h"

#include <QHash>
#include <QImage>
#include <QJsonObject>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPixmap>
#include <QResizeEvent>
#include <QVector>
#include <QWidget>

namespace kerf {

class MediaArtwork;

// Editing tools, as in other edit suites: what a click or drag on a clip does.
enum class TimelineTool { Selection, Razor, Ripple, Roll, Slip, Slide };

class TimelineWidget final : public QWidget {
    Q_OBJECT
public:
    explicit TimelineWidget(QWidget *parent = nullptr);
    void setProject(const QJsonObject &project, double duration, double fps);
    void setSelectedClip(const QString &id);
    void setPlayhead(double time);
    void showSkimmer(double time);
    double skimmerTime() const { return m_skimmerTime; }
    void setZoom(double zoom);
    void fitToWidth(int width);
    bool isFitToWidth() const { return m_fitWidth > 0; }
    void setArtwork(MediaArtwork *artwork);
    void clearSkimmer();
    void setTool(TimelineTool tool);
    TimelineTool tool() const { return m_tool; }
    // Dragged edges snap to clip edges, markers, the playhead, and the start.
    void setSnapping(bool snapping) { m_snapping = snapping; }
    bool snapping() const { return m_snapping; }
    // The track header toggles: picture output or sound mute, and lock.
    QRect headerToggleRect(int row, int index) const;
    // A clip's rectangle in widget coordinates; empty when it is not shown.
    QRect clipBounds(const QString &id) const;
    // A sequence marker on the ruler, with the span of its duration, in
    // widget coordinates; empty when there is no such marker.
    QRect markerBounds(const QString &id) const;
    // The timeline a pending proposal would make, drawn over this one:
    // new and changed clips as outlines where they would land, removed
    // clips struck through. An empty object clears it.
    void setProposal(const QJsonObject &timeline);
    bool hasProposal() const { return !m_proposal.isEmpty(); }

    // Every track shares the visible panel height, the way Pro Tools fits
    // tracks to the window. The empty add-track row stays reachable below.
    void fitTracksToView();
    void resetTrackHeights();
    QSize minimumSizeHint() const override { return minimumSize(); }
    bool addLaneVisible() const { return m_addLaneVisible; }
    QPoint addLaneButtonCenter() const { return addLaneButtonRect().center(); }
    QString skimLabel() const { return m_hasSkimmer ? prettyTime(m_skimmerTime) : QString(); }
    int trackRowCount() const { return static_cast<int>(m_rows.size()); }
    QString trackRowLabel(int row) const;
    QRect trackHeaderRect(int row) const;

signals:
    void seekRequested(double time);
    void hoverSeekRequested(double time);
    void hoverSeekEnded();
    void clipSelected(const QString &clipId);
    // A body drag, with the Selection, Slip, or Slide tool.
    void moveRequested(const QString &clipId, double timelineStart);
    // An edge drag, with the Selection, Ripple, or Roll tool.
    void trimRequested(const QString &clipId, bool leading, double timelineFrame);
    void razorRequested(const QString &clipId, double timelineFrame);
    void trackToggleRequested(const QString &trackId, const QString &field, bool value);
    // Double-click: open the clip's media in the source monitor.
    void clipActivated(const QString &clipId);
    // Option-double-click: ask Kerf about the clip.
    void askRequested(const QString &clipId);
    void clipMenuRequested(const QString &clipId, const QPoint &globalPosition);
    void addLaneRequested();

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void moveEvent(QMoveEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void contextMenuEvent(QContextMenuEvent *event) override;
    void leaveEvent(QEvent *) override;

private:
    struct ClipHit {
        QString id;
        int row = -1;
        double start = 0.0;
        double duration = 0.0;
        QRectF rect;
    };

    // One display row per track. The project keeps tracks in layer order;
    // rows place them the way an edit suite shows them.
    struct TrackRow {
        int track = -1;
        bool audio = false;
        int number = 0;
        int top = kRulerHeight;
        int height = kTrackHeight;
        int bottom() const { return top + height; }
    };

    static QString trackKey(const QJsonObject &track, int index);

    // Classic order: playback draws a later picture track over earlier ones,
    // so picture tracks stack upward from V1, which sits on the sound bank.
    // Sound tracks run downward from A1.
    void layoutRows();
    void relayout();
    void setTrackHeight(int row, int height);
    int tracksBottom() const { return m_rows.isEmpty() ? kRulerHeight : m_rows.constLast().bottom(); }
    int rowAt(int y) const;

    // The part of the content on screen. Inside a scroll area the widget is
    // moved by the scroll offset; the ruler and headers are pinned to it.
    QRect viewportRect() const;
    QPoint stickyOrigin() const { return viewportRect().topLeft(); }
    bool inHeader(const QPoint &point) const { return point.x() < stickyOrigin().x() + kTimelineLabelWidth; }
    bool inRuler(const QPoint &point) const { return point.y() < stickyOrigin().y() + kRulerHeight; }
    QRect headerColumnRect() const;
    int resizeRowAt(const QPoint &point) const;
    void clearHoverClip();
    void paintLanes(QPainter &p, const QRect &visible);
    void paintLane(QPainter &p, const QRect &visible, int row, const QJsonObject &track);
    void paintHeaders(QPainter &p, const QRect &dirty);
    void paintTrackHeader(QPainter &p, int row, const QJsonObject &track, int left);
    void paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QColor &accent, bool audio);
    void paintProposal(QPainter &p, int row, const QJsonObject &track);
    void paintRuler(QPainter &p, const QRect &dirty, int top);
    // Sequence markers sit on the ruler: a tab on their frame and a band
    // over their duration.
    void paintMarkers(QPainter &p, const QRect &dirty, int top);
    QRectF markerRect(const QJsonObject &marker, int top) const;

    // Painted every frame on top of the lane cache, in stacking order: lane
    // lines, pinned ruler with its playhead tab, pinned headers, corner, and
    // transient feedback.
    void paintOverlay(QPainter &p, const QRect &dirty);

    // While a clip moves or trims, show where the edge lands and how far it went.
    void paintDragReadout(QPainter &p);
    void paintCornerTime(QPainter &p, const QPoint &origin, double time, const QColor &color) const;
    void repaintAt(qreal x);
    void ensureLaneCache();
    bool lanesAreCacheable() const;
    void invalidateLanes() { m_laneCache = QPixmap(); }
    qreal timeToX(double time) const;

    // Clips sit on whole pixels at their exact edit points, so adjacent clips
    // butt together the way the cut plays.
    QRectF clipRect(int row, double start, double duration) const;
    double dragOrigin() const;
    QRect headerPreviewRect(int row, int left) const;
    QImage tintedWave(const QString &sourceId, const QColor &color);
    QRect pointerBand(qreal x) const;
    void updateContentSize();
    double pixelsPerSecond() const;
    double timeAt(qreal x) const;

    // Lanes hidden under the pinned headers are not addressable.
    double scrubTime(qreal x) const;
    void beginScrub(const QPointF &point);
    void updateSkimmer(const QPointF &point);
    QRect addLaneButtonRect() const;
    QRect addLaneHitRect() const;
    void updateAddLane(const QPoint &point);
    ClipHit clipAt(const QPoint &point) const;
    // The header toggle under the pointer: its row and the track field it sets.
    bool toggleAt(const QPoint &point, int *row, QString *field) const;
    bool trimsEdges() const { return m_tool == TimelineTool::Selection || m_tool == TimelineTool::Ripple || m_tool == TimelineTool::Roll; }
    bool dragsBodies() const { return m_tool == TimelineTool::Selection || m_tool == TimelineTool::Slip || m_tool == TimelineTool::Slide; }
    void setToolCursor(const ClipHit &hit, int edge);
    // The dragged edge or start, snapped to the nearest edit point in reach.
    double snapped(double time, double length) const;

    MediaArtwork *m_artwork = nullptr;
    QJsonObject m_project;
    QJsonObject m_proposal;
    QVector<TrackRow> m_rows;
    // Track heights are a view preference keyed by track id, so they survive
    // snapshots and reordering without touching the project.
    QHash<QString, int> m_trackHeights;
    QString m_selectedClip;
    QString m_hoverClipId;
    QString m_dragClipId;
    int m_dragRow = 0;
    int m_hoverEdge = 0;
    int m_hoverGrip = -1;
    int m_resizeRow = -1;
    int m_resizeAnchor = 0;
    int m_resizeStartHeight = 0;
    double m_duration = 1.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_skimmerTime = 0.0;
    double m_hoverFrame = -1.0;
    double m_zoom = 1.0;
    int m_fitWidth = 0;
    double m_dragStart = 0.0;
    double m_dragDuration = 0.0;
    double m_pressTime = 0.0;
    double m_previewTime = 0.0;
    bool m_scrubbing = false;
    bool m_dragging = false;
    bool m_hasSkimmer = false;
    bool m_dragLeading = false;
    bool m_dragTrailing = false;
    bool m_dragMoved = false;
    bool m_addLaneVisible = false;
    bool m_snapping = true;
    TimelineTool m_tool = TimelineTool::Selection;
    QPointF m_feedbackPoint;
    QPixmap m_laneCache;
    QHash<QString, QImage> m_waveTints;
};

} // namespace kerf
