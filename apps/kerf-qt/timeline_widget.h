#pragma once

// The timeline (mockup-spec 7): the bar's tools and zoom, 200 pt track heads
// with patch, target, lock, eye or mute and solo, and sync-lock on hover; a
// 32 pt ruler with comment marks, proposal bands and sequence markers over
// its ticks; and flat clips on the well: filmstrips, waveforms with the
// volume rubber band, titles and captions, proposal veils and, while a
// proposal plays as a preview, ghosts where its clips were.
//
// Everything drawn is the snapshot's (clips, tracks, markers, gain keys) or
// what the window feeds it from the engine: comment marks and proposal
// bands from the review (TimelineWiring), QC findings from `check`, the
// source patch from the Source viewer's media. Solo is monitoring only and
// lasts the session. The targets show where a source edit lands (the first
// unlocked track of each kind, as MainWindow::sourceTargets() picks); they
// are read-only until source edits follow them (setTargetsEditable).

#include "editor_common.h"
#include "test_friends.h"

#include <QHash>
#include <QImage>
#include <QJsonObject>
#include <QJsonValue>
#include <QKeySequence>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPixmap>
#include <QPointer>
#include <QPolygonF>
#include <QResizeEvent>
#include <QScrollArea>
#include <QSet>
#include <QStaticText>
#include <QVector>
#include <QWidget>

#include <utility>

class QAction;
class QShortcut;

namespace kerf {

namespace ui {
class GhostButton;
class IconButton;
class SegmentedControl;
class Slider;
} // namespace ui

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
    KERF_TEST_FRIENDS
public:
    // A root comment's mark on the ruler (ReviewModel::LaneMarker): a dot at
    // its time, a bar over its range. People's dots are round, agents' and
    // checks' rounded squares.
    struct CommentMark {
        QString id;
        double at = 0.0;
        double end = 0.0;
        bool ranged = false;
        bool circle = true;
        bool current = false;
        bool resolved = false;
        QString name;
    };
    // An open proposal: its band over the ruler and its veil over the lanes
    // of the tracks it changes, from `start` to `end`; the clips it changes
    // are outlined in the accent; while it plays as a preview its ghosts
    // show where those clips are now.
    struct ProposalBand {
        struct Ghost {
            QString track;
            double start = 0.0;
            double end = 0.0;
        };
        QString id;
        QString author;
        double start = 0.0;
        double end = 0.0;
        QStringList tracks;
        QStringList clips;
        QList<Ghost> ghosts;
    };
    // A track head's controls, left to right.
    enum class HeaderControl { Patch, Target, Sync, Lock, Output, Solo };
    // The track height bar's presets.
    enum class HeightPreset { None = -1, Small = 0, Medium = 1, Large = 2 };

    explicit TimelineWidget(QWidget *parent = nullptr);
    void setProject(const QJsonObject &project, double duration, double fps);
    void setSelectedClip(const QString &id);
    // Every selected clip (⌘-click or ⇧-click adds), the primary one among them.
    void setSelectedClips(const QStringList &ids);
    QStringList selectedClips() const { return m_selectedClips; }
    void setPlayhead(double time);
    double playhead() const { return m_playhead; }
    void showSkimmer(double time);
    double skimmerTime() const { return m_skimmerTime; }
    void setZoom(double zoom);
    void fitToWidth(int width);
    // The scale, in points per second of the sequence: the bar's zoom
    // control sets it between these, and hears of every change (a fit to
    // the width included).
    static constexpr double kMinScale = 20.0;
    static constexpr double kMaxScale = 480.0;
    void setScale(double pixelsPerSecond);
    double scale() const { return pixelsPerSecond(); }
    bool isFitToWidth() const { return m_fitWidth > 0; }
    double fps() const { return m_fps; }
    void setArtwork(MediaArtwork *artwork);
    void clearSkimmer();
    void setTool(TimelineTool tool);
    TimelineTool tool() const { return m_tool; }
    // Dragged edges snap to clip edges, markers, the playhead, and the start.
    void setSnapping(bool snapping);
    bool snapping() const { return m_snapping; }
    // A head's output (index 0: eye on picture, M on sound) and lock (1).
    QRect headerToggleRect(int row, int index) const;
    // Any of a head's controls, in widget coordinates; empty where the row
    // has none (Solo on picture, the patch box with nothing patched).
    QRect headerControlRect(int row, HeaderControl control) const;
    // A clip's rectangle in widget coordinates; empty when it is not shown.
    QRect clipBounds(const QString &id) const;
    // A sequence marker's flag on the ruler, with the span of its duration,
    // in widget coordinates; empty when there is no such marker.
    QRect markerBounds(const QString &id) const;
    // The first pending proposal's timeline (the snapshot's
    // proposalTimeline). Without bands of its own (setProposalBands) the
    // timeline makes one from it. An empty object clears it.
    void setProposal(const QJsonObject &timeline);
    bool hasProposal() const { return !m_proposal.isEmpty() || !m_bands.isEmpty(); }

    // The ruler's comment marks.
    void setCommentMarks(const QList<CommentMark> &marks);
    const QList<CommentMark> &commentMarks() const { return m_marks; }
    QRect commentMarkBounds(const QString &id) const;
    // Every open proposal's band; empty falls back to setProposal's.
    void setProposalBands(const QList<ProposalBand> &bands);
    QList<ProposalBand> proposalBands() const;
    QRect proposalBandBounds(const QString &id) const;
    // A proposal playing as a preview, with the timeline it would make:
    // its clips are drawn where it puts them, ghosts where they are now. An
    // empty id ends it.
    void setPreview(const QString &proposalId, const QJsonObject &timeline);
    QString previewProposal() const { return m_previewId; }
    // Clips a QC check found a problem in: a --neg dot in their name.
    void setQcClips(const QSet<QString> &clips);
    QSet<QString> qcClips() const { return m_qcClips; }
    // The Source viewer's channels and where they go: track id → "v1", "a1".
    void setSourcePatch(const QHash<QString, QString> &patch);
    QHash<QString, QString> sourcePatch() const { return m_patch; }
    // The targeted tracks: the first unlocked picture and sound tracks, or,
    // once the targets are editable, those chosen with the target toggles.
    QStringList targetTracks() const;
    void setTargetTracks(const QStringList &tracks);
    // Whether a click on a head's target toggles it. Off (the default) the
    // targets only show where source edits land, since Insert and Overwrite
    // write to MainWindow::sourceTargets(); turn it on once those follow
    // targetTracks().
    void setTargetsEditable(bool editable);
    bool targetsEditable() const { return m_targetsEditable; }
    // The soloed sound tracks (monitoring only, for this session).
    QStringList soloTracks() const { return m_solo; }
    void setSoloTracks(const QStringList &tracks);

    // Every track shares the visible panel height, the way Pro Tools fits
    // tracks to the window.
    void fitTracksToView();
    // Every track back to its kind's default height (M).
    void resetTrackHeights();
    // Every track at `height` (clamped to S and the tallest), which new
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
    // S, M or L: every track at 24, at its default, or at 48.
    void applyHeightPreset(HeightPreset preset);
    // The preset every track is at, or None.
    HeightPreset heightPreset() const;
    // Deprecated, removed in W3: the heights are the bar's segmented
    // control (TimelineZoomControl), so the dock gets no buttons of them.
    QList<QAction *> trackHeightActions() const { return {}; }
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
    QString trackRowId(int row) const;
    QRect trackHeaderRect(int row) const;
    // The row at widget `y`, or -1.
    int trackRowAt(int y) const { return rowAt(y); }
    // The volume rubber band of a sound clip, in widget coordinates: one
    // point per key (or the clip's ends for a level without keys).
    QPolygonF gainBand(const QString &clipId) const;
    // A gain in dB as the band draws it in a clip `height` tall, and back.
    static qreal gainY(double decibels, const QRectF &clip);
    static double gainAt(qreal y, const QRectF &clip);

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
    // A head's lock, eye (`enabled`), mute or sync-lock (`syncLocked`).
    void trackToggleRequested(const QString &trackId, const QString &field, bool value);
    // Double-click: open the clip's media in the source monitor.
    void clipActivated(const QString &clipId);
    // Option-double-click: ask Kerf about the clip.
    void askRequested(const QString &clipId);
    void clipMenuRequested(const QString &clipId, const QPoint &globalPosition);
    void addLaneRequested();
    // The S toggles: the tracks to hear alone (empty: all).
    void soloChanged(const QStringList &tracks);
    void targetsChanged(const QStringList &tracks);
    // An effect card (application/x-kerf-fx, from the Effects browser)
    // dropped on a clip.
    void effectDropped(const QString &clipId, const QString &fx);
    // The rubber band dragged: a clip's gain, a level in dB or
    // {keys: [{at: frame, value, interp}]}.
    void gainRequested(const QString &clipId, const QJsonValue &gain);
    // A comment's mark on the ruler clicked.
    void commentMarkActivated(const QString &commentId);
    void toolChanged(TimelineTool tool);
    void snappingChanged(bool snapping);
    // Track heights changed (a preset, a drag, a step, Fit).
    void heightsChanged();

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
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dragMoveEvent(QDragMoveEvent *event) override;
    void dragLeaveEvent(QDragLeaveEvent *event) override;
    void dropEvent(QDropEvent *event) override;

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

    // The rubber band being dragged: its clip, the key held (-1: the whole
    // level), and the values the drag started from and shows now.
    struct BandDrag {
        QString clip;
        int row = -1;
        int key = -1;
        qreal pressY = 0.0;
        bool keyed = false;
        QVector<double> from;
        QVector<double> values;
        bool moved = false;
    };

    static QString trackKey(const QJsonObject &track, int index);
    QString rowKey(int row) const;
    QJsonObject rowTrack(int row) const;

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
    void saveHeights() const;
    int tracksBottom() const { return m_rows.isEmpty() ? kRulerHeight : m_rows.constLast().bottom(); }
    // The last picture row, whose line parts picture from sound; -1 with none.
    int lastPictureRow() const { return m_lastPicture; }
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
    void paintTrackHeader(QPainter &p, int row, const QJsonObject &track, int left, const QStringList &targets);
    void paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track, bool audio,
                   bool proposed);
    // The clip being dragged, at its preview position over its neighbours.
    void paintDraggedClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track,
                          bool audio);
    // A clip's 16 pt name bar: QC dot, name, its secondary text, fx glyph.
    void paintNameBar(QPainter &p, const QRectF &bar, const QString &name, const QString &secondary, bool qc, bool fx,
                      bool shadow, const QColor &ink);
    void paintGainBand(QPainter &p, const QRectF &rect, const QJsonObject &clip);
    void paintRuler(QPainter &p, const QRect &dirty, int top);
    // The ruler's marker row: proposal bands, comment marks, sequence markers.
    void paintMarkerRow(QPainter &p, const QRect &dirty, int top);
    QRectF markerRect(const QJsonObject &marker, int top) const;
    QRectF commentMarkRect(const CommentMark &mark, int top) const;
    QRectF bandRect(const ProposalBand &band, int top) const;
    // The mark or band under `point` in the ruler, or -1.
    int commentMarkAt(const QPoint &point) const;
    int bandAt(const QPoint &point) const;
    void paintCorner(QPainter &p, const QPoint &origin) const;
    // Deprecated, removed in W3: the hardware heads' engraved badge font,
    // which theme_test still reads. The flat heads draw none.
    QFont badgeFont() const { return engravedFont(font(), QStringLiteral("V1")); }

    // Painted every frame on top of the lane cache, in stacking order: the
    // playhead, the pinned ruler with its marks and the playhead's head,
    // the pinned headers, the corner, and transient feedback.
    void paintOverlay(QPainter &p, const QRect &dirty);

    // While a clip moves or trims, show where the edge lands and how far it went.
    void paintDragReadout(QPainter &p);
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
    // An icon of the set, drawn once per colour, size and pixel ratio.
    QPixmap iconPixmap(const QString &name, const QColor &color, qreal px, qreal dpr);
    // The source seconds `offset` seconds into `clip`.
    static double clipSourceTime(const QJsonObject &clip, double offset);

    // Clips sit on whole pixels at their edit points, a pixel short of the
    // next (the mockup's width = LX(end) − x − 1), one in from the lane's
    // top and bottom.
    QRectF clipRect(int row, double start, double duration) const;
    double dragOrigin() const;
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
    // The head control under the pointer, and its row.
    bool controlAt(const QPoint &point, int *row, HeaderControl *control) const;
    // The header toggle under the pointer: its row and the track field it sets.
    bool toggleAt(const QPoint &point, int *row, QString *field) const;
    void pressControl(int row, HeaderControl control);
    // A sound clip's rubber band key under `point` (-1: the line), or
    // false when the point is not on its band.
    bool bandAt(const QPoint &point, const ClipHit &hit, int *key) const;
    // The gain keys of `clip` (seconds into the clip and dB), or its level.
    struct GainKey {
        double at = 0.0;
        qint64 frame = 0;
        double value = 0.0;
        QString interp;
    };
    QVector<GainKey> gainKeys(const QJsonObject &clip, bool *keyed) const;
    void beginBandDrag(const ClipHit &hit, int key, qreal y, bool addKey, double time);
    void finishBandDrag();
    bool trimsEdges() const { return m_tool == TimelineTool::Selection || m_tool == TimelineTool::Ripple || m_tool == TimelineTool::Roll; }
    bool dragsBodies() const { return m_tool == TimelineTool::Selection || m_tool == TimelineTool::Slip || m_tool == TimelineTool::Slide; }
    void setToolCursor(const ClipHit &hit, int edge);
    // The dragged edge or start, snapped to the nearest edit point in reach.
    // `snapPoint`, when given, is set to that edit point, or -1 if none was in reach.
    double snapped(double time, double length, double *snapPoint = nullptr) const;
    // The bands with the clips they change on `trackId` (a cache rebuilt with them).
    bool movedByProposal(const QString &clipId) const { return m_proposedClips.contains(clipId); }
    void rebuildBandIndex();

    MediaArtwork *m_artwork = nullptr;
    QJsonObject m_project;
    QJsonObject m_proposal;
    QList<ProposalBand> m_bands;
    // The bands in effect: m_bands, else the one made from m_proposal.
    QList<ProposalBand> m_effectiveBands;
    QSet<QString> m_proposedClips;
    QString m_previewId;
    // The clip an effect card is held over, ringed in the accent.
    QString m_dropClip;
    QRectF m_dropRect;
    QJsonObject m_previewTimeline;
    QHash<QString, QJsonObject> m_previewTracks;
    QList<CommentMark> m_marks;
    QSet<QString> m_qcClips;
    QHash<QString, QString> m_patch;
    QStringList m_targets;
    bool m_targetsChosen = false;
    bool m_targetsEditable = false;
    QStringList m_solo;
    QVector<TrackRow> m_rows;
    int m_lastPicture = -1;
    // Track heights are a view preference keyed by track id, so they survive
    // snapshots and reordering without touching the project.
    QHash<QString, int> m_trackHeights;
    // The height all tracks were last set to together, which tracks without
    // one of their own take; 0 for the defaults.
    int m_allHeight = 0;
    // Where track heights are remembered (a settings group), or empty.
    QString m_heightsKey;
    QString m_heightsPath;
    // Option-drag of a header's edge: every track of the grabbed one's kind.
    bool m_resizeKind = false;
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
    // The head under the pointer (its row), and the control (row × 8 + control), or -1.
    int m_hoverRow = -1;
    int m_hoverControl = -1;
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
    BandDrag m_band;
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
    qsizetype m_filmstripTileBytes = 0;
    QHash<QString, QStaticText> m_names;
    QHash<QString, QPixmap> m_icons;
    // Every clip edge, marker and the playhead, sorted, while a clip is
    // dragged: what it snaps to.
    QVector<double> m_snapPoints;
    // Lanes painted since the counter was last cleared (the tests read it).
    int m_lanesPainted = 0;
};

// The timeline bar's left half (mockup-spec 7.1), after "Timeline" and its
// "N tracks · duration": the tools (Selection V, Razor C, Ripple B, Roll N,
// Slip Y, Slide U), Snap, Add marker, Add track, and the Ask ghost, which
// reads "Ask · N notes" while the comments panel is closed and notes are
// open. It follows the timeline's tool and snapping; what it asks for goes
// out as signals (TimelineWiring runs the commands), and a tool or snap
// nobody listens for is set on the timeline directly.
class TimelineToolStrip final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit TimelineToolStrip(TimelineWidget *timeline, QWidget *parent = nullptr);
    ui::SegmentedControl *tools() const { return m_tools; }
    ui::IconButton *snapButton() const { return m_snap; }
    ui::IconButton *markerButton() const { return m_marker; }
    ui::IconButton *trackButton() const { return m_track; }
    ui::GhostButton *askButton() const { return m_ask; }
    // A control as the registry's command for it stands: enabled, and its
    // tip "name", or "name · why" while the command is unavailable, with
    // the command's shortcut in the keymap in use. TimelineWiring calls it
    // whenever the command's QAction changes; until then a control has no
    // shortcut in its tip.
    enum class Control { Snap, Marker, Track, Ask };
    void followTool(TimelineTool tool, bool enabled, const QString &reason, const QString &shortcut);
    void followControl(Control control, bool enabled, const QString &reason, const QString &shortcut);
    static QString toolName(TimelineTool tool);
    static QString controlName(Control control);
    // Open notes while the panel is closed; 0 reads plain "Ask".
    void setAskNotes(int notes);
    QString askText() const;
    static int toolIndex(TimelineTool tool);
    static TimelineTool toolAt(int index);

signals:
    void toolRequested(TimelineTool tool);
    void snapRequested(bool snapping);
    void addMarkerRequested();
    void addTrackRequested();
    void askRequested();

private:
    TimelineWidget *m_timeline;
    ui::SegmentedControl *m_tools = nullptr;
    ui::IconButton *m_snap = nullptr;
    ui::IconButton *m_marker = nullptr;
    ui::IconButton *m_track = nullptr;
    ui::GhostButton *m_ask = nullptr;
};

// The timeline bar's right half: Zoom out, a 64 pt slider and Zoom in,
// then the track heights S, M, L and Fit. The slider runs over the
// timeline's scale on a log scale; the keys step it, as do Command-= and
// Command-minus while the timeline has focus (shortcuts on the timeline,
// owned by this control); it follows the timeline's own changes (a fit to
// the width), and the heights follow its tracks.
class TimelineZoomControl final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kSteps = 100;
    explicit TimelineZoomControl(TimelineWidget *timeline, QWidget *parent = nullptr);
    ~TimelineZoomControl() override;
    ui::Slider *slider() const { return m_slider; }
    ui::IconButton *zoomOutButton() const { return m_out; }
    ui::IconButton *zoomInButton() const { return m_in; }
    ui::SegmentedControl *heights() const { return m_heights; }
    // A scale's place on the slider, and back.
    static int position(double pixelsPerSecond);
    static double scaleAt(int position);
    // The zoom keys' shortcuts: Command-= (and Command-+), Command-minus.
    static QList<QKeySequence> zoomInKeys();
    static QList<QKeySequence> zoomOutKeys();
    // One step of the keys (a tenth of the slider) in or out.
    void step(int direction);

private:
    void syncHeights();
    TimelineWidget *m_timeline;
    QList<QPointer<QShortcut>> m_shortcuts;
    ui::Slider *m_slider = nullptr;
    ui::IconButton *m_out = nullptr;
    ui::IconButton *m_in = nullptr;
    ui::SegmentedControl *m_heights = nullptr;
};

} // namespace kerf
