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
#include <QScrollArea>
#include <QStaticText>
#include <QVector>
#include <QWidget>

#include <utility>

class QAction;
class QSlider;
class QToolButton;

namespace kerf {

class MediaArtwork;

// The timeline's scroll area. A scroll step moves what is already on
// screen and repaints only what scrolls into view, with the ruler and the
// track headers that stay pinned, so it costs a strip, not the panel.
class TimelineScrollArea final : public QScrollArea {
public:
    explicit TimelineScrollArea(QWidget *parent = nullptr);

protected:
    void scrollContentsBy(int dx, int dy) override;
};

// Editing tools, as in other edit suites: what a click or drag on a clip does.
enum class TimelineTool { Selection, Razor, Ripple, Roll, Slip, Slide };

class TimelineWidget final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit TimelineWidget(QWidget *parent = nullptr);
    void setProject(const QJsonObject &project, double duration, double fps);
    void setSelectedClip(const QString &id);
    // Every selected clip (⌘-click or ⇧-click adds), the primary one among them.
    void setSelectedClips(const QStringList &ids);
    QStringList selectedClips() const { return m_selectedClips; }
    void setPlayhead(double time);
    void showSkimmer(double time);
    double skimmerTime() const { return m_skimmerTime; }
    void setZoom(double zoom);
    void fitToWidth(int width);
    // The scale, in points per second of the sequence: the header's zoom
    // control sets it between these, and hears of every change (a fit to
    // the width included).
    static constexpr double kMinScale = 20.0;
    static constexpr double kMaxScale = 480.0;
    void setScale(double pixelsPerSecond);
    double scale() const { return pixelsPerSecond(); }
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
    // Every track back to its default height.
    void resetTrackHeights();
    // Every track at `height` (clamped to Mini and the tallest), which new
    // tracks take too.
    void setAllTrackHeights(int height);
    // The height every track was last set to together; 0 when tracks
    // start at their defaults.
    int allTrackHeight() const { return m_allHeight; }
    // Every track `steps` steps of kTrackHeightSteps taller (or shorter, when
    // negative) than it is now, so tracks sized apart (tall picture, short
    // sound, one tall hero layer) keep their order and none shrinks on a
    // step taller; tracks added later take the step from the all-tracks
    // height. Option-Up and Option-Down, and (keeping the track under the
    // pointer in place) Option-scroll over the headers.
    void stepAllTrackHeights(int steps);
    // The header bar's track height controls: Mini, Medium (the defaults)
    // and Large, checked when every track is at that height, and Fit.
    QList<QAction *> trackHeightActions() const;
    // Remember track heights for the project at `projectPath` in the
    // editor's settings (never in the project, which collaborators share),
    // and restore the ones remembered for it. An empty path forgets nothing
    // and remembers nothing.
    void setHeightsKey(const QString &projectPath);
    QSize minimumSizeHint() const override { return minimumSize(); }
    bool addLaneVisible() const { return m_addLaneVisible; }
    QPoint addLaneButtonCenter() const { return addLaneButtonRect().center(); }
    QString skimLabel() const { return m_hasSkimmer ? prettyTime(m_skimmerTime) : QString(); }
    int trackRowCount() const { return static_cast<int>(m_rows.size()); }
    QString trackRowLabel(int row) const;
    QRect trackHeaderRect(int row) const;
    // The row at widget `y`, or -1.
    int trackRowAt(int y) const { return rowAt(y); }
    // A header's track number badge.
    QRect headerBadgeRect(int row) const;

signals:
    void scaleChanged(double pixelsPerSecond);
    void seekRequested(double time);
    void hoverSeekRequested(double time);
    void hoverSeekEnded();
    void clipSelected(const QString &clipId);
    // ⌘-click (`toggle`) or ⇧-click on a clip: add it to the selection, or
    // with ⌘ take it out again.
    void clipSelectionExtended(const QString &clipId, bool toggle);
    // A click on an empty lane, or Escape, with a clip selected.
    void deselectRequested();
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
    bool event(QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void moveEvent(QMoveEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void contextMenuEvent(QContextMenuEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;
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
    QString rowKey(int row) const;

    // Classic order: playback draws a later picture track over earlier ones,
    // so picture tracks stack upward from V1, which sits on the sound bank.
    // Sound tracks run downward from A1.
    void layoutRows();
    void relayout();
    // Lay out again keeping `row`'s top where it is in the scroll area's
    // view (when the timeline is in one), so resizing many tracks does not
    // carry what was being looked at out of sight.
    void relayoutKeeping(int row);
    // The row at the top of the view, under the ruler.
    int firstVisibleRow() const;
    // setAllTrackHeights(), keeping `row` in place.
    void setAllTrackHeightsKeeping(int height, int row);
    // stepAllTrackHeights(), keeping `row` in place.
    void stepAllTrackHeightsKeeping(int steps, int row);
    void setTrackHeight(int row, int height);
    // A track's height when it has none of its own.
    int defaultTrackHeight(bool audio) const;
    void updateHeightActions();
    void saveHeights() const;
    int tracksBottom() const { return m_rows.isEmpty() ? kRulerHeight : m_rows.constLast().bottom(); }
    // A binary search over the rows' tops.
    int rowAt(int y) const;
    // The rows that meet [top, bottom], as first and one-past-last indexes.
    std::pair<int, int> rowsBetween(int top, int bottom) const;

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
    // A header's badge and toggles: 16px, smaller on rows too short for that.
    static int headerControlSize(int rowHeight);
    // The track number badge's font.
    QFont badgeFont() const;
    // The top of a header's line of controls.
    int headerControlTop(int row) const;
    void paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track, bool audio);
    // The clip being dragged, at its preview position over its neighbours.
    void paintDraggedClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track,
                          bool audio);
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
    // Lanes are cached for the part of the timeline in view, so a repaint
    // that only moves the playhead or the skimmer blits them.
    void ensureLaneCache(const QRect &view);
    bool laneCacheFits(const QRect &view) const;
    // The lanes in `area` drawn into a pixmap, as the cache is, or null
    // when it is too large to hold.
    QPixmap renderLanes(const QRect &area);
    void invalidateLanes() { m_laneCache = QPixmap(); }
    // Repaint the lanes in `rect` alone, in the cache too: a clip the
    // pointer enters or leaves, or one selected.
    void refreshLanes(const QRect &rect);
    // A name in `font`, elided to `rect`'s width and centred in its height
    // (and its width when `centred`). Names are laid out once and kept: a
    // dense timeline repaints hundreds.
    void drawName(QPainter &p, const QRectF &rect, const QFont &font, const QString &text, bool centred = false);
    // That name laid out, elided to `width`.
    QStaticText laidName(const QFont &font, const QString &text, qreal width);
    qreal timeToX(double time) const;
    // A filmstrip tile: frame `frame` of `sourceId`'s strip (its poster when
    // -1, or until the strip arrives) scaled once to the tile's size.
    QImage filmstripTile(const QString &sourceId, int frame, const QSize &size, qreal dpr);
    // Where a clip's name goes and where its filmstrip or waveform does.
    // With artwork to show and too short for both one over the other, the
    // body is the whole clip and the name sits on a chip over it.
    struct ClipLayout {
        QRectF band;
        QRectF body;
        bool chip = false;
    };
    static ClipLayout clipLayout(const QRectF &clip, bool artwork);
    // The source seconds `offset` seconds into `clip`.
    static double clipSourceTime(const QJsonObject &clip, double offset);

    // Clips sit on whole pixels at their exact edit points, so adjacent clips
    // butt together the way the cut plays.
    QRectF clipRect(int row, double start, double duration) const;
    double dragOrigin() const;
    QRect headerPreviewRect(int row, int left) const;
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
    // `snapPoint`, when given, is set to that edit point, or -1 if none was in reach.
    double snapped(double time, double length, double *snapPoint = nullptr) const;

    MediaArtwork *m_artwork = nullptr;
    QJsonObject m_project;
    QJsonObject m_proposal;
    QVector<TrackRow> m_rows;
    // Track heights are a view preference keyed by track id, so they survive
    // snapshots and reordering without touching the project.
    QHash<QString, int> m_trackHeights;
    // The height all tracks were last set to together, which tracks without
    // one of their own take; 0 for the defaults.
    int m_allHeight = 0;
    // Where track heights are remembered (a settings group), or empty.
    QString m_heightsKey;
    QString m_heightsPath;
    QAction *m_miniTracks = nullptr;
    QAction *m_mediumTracks = nullptr;
    QAction *m_largeTracks = nullptr;
    QAction *m_fitTracks = nullptr;
    // Option-drag of a header's edge: every track of the grabbed one's kind.
    bool m_resizeKind = false;
    // Every badge is as wide as the widest track number needs (V100 and up
    // in a big edit), so the toggles after it line up down the column.
    int m_badgeWidth = 22;
    int m_wheelSteps = 0;
    QString m_selectedClip;
    QStringList m_selectedClips;
    bool isSelected(const QString &id) const { return id == m_selectedClip || m_selectedClips.contains(id); }
    QString m_hoverClipId;
    QRect m_hoverRect;
    QString m_dragClipId;
    int m_dragRow = 0;
    int m_hoverEdge = 0;
    int m_hoverGrip = -1;
    // The header toggle under the pointer (its row × 2 + its index), or -1.
    int m_hoverToggle = -1;
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
    // The edit point the current drag has snapped to, shown as a line; -1 when none.
    double m_snapTime = -1.0;
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
    // The part of the timeline the lane cache holds, in widget coordinates.
    QRect m_laneCacheRect;
    struct FilmstripTileKey {
        QString source;
        int frame = -1;
        QSize size;
        int ratio = 100; // The device pixel ratio, in hundredths.
        bool operator==(const FilmstripTileKey &other) const {
            return frame == other.frame && ratio == other.ratio && size == other.size && source == other.source;
        }
        friend size_t qHash(const FilmstripTileKey &key, size_t seed = 0) {
            return qHashMulti(seed, key.source, key.frame, key.size.width(), key.size.height(), key.ratio);
        }
    };
    QHash<FilmstripTileKey, QImage> m_filmstripTiles;
    QHash<QString, QStaticText> m_names;
    // Every clip edge, marker and the playhead, sorted, while a clip is
    // dragged: what it snaps to.
    QVector<double> m_snapPoints;
    // Lanes painted since the counter was last cleared (the tests read it).
    int m_lanesPainted = 0;
};

// The timeline header's zoom: − slider +, as the structure the owner chose
// puts it beside the track heights. The slider runs over the timeline's
// scale on a log scale; the buttons step it; it follows the timeline's own
// changes (a fit to the width).
class TimelineZoomControl final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kSteps = 100;
    explicit TimelineZoomControl(TimelineWidget *timeline, QWidget *parent = nullptr);
    QSlider *slider() const { return m_slider; }
    QToolButton *zoomOutButton() const { return m_out; }
    QToolButton *zoomInButton() const { return m_in; }
    // A scale's place on the slider, and back.
    static int position(double pixelsPerSecond);
    static double scaleAt(int position);

private:
    TimelineWidget *m_timeline;
    QSlider *m_slider = nullptr;
    QToolButton *m_out = nullptr;
    QToolButton *m_in = nullptr;
};

} // namespace kerf
