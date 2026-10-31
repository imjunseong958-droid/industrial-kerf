#include "timeline_widget.h"
#include "editor_theme.h"
#include "media_artwork.h"

#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCryptographicHash>
#include <QDir>
#include <QFileInfo>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QKeyEvent>
#include <QLinearGradient>
#include <QMenu>
#include <QPainterPath>
#include <QScrollBar>
#include <QSettings>
#include <QSignalBlocker>
#include <QSlider>
#include <QToolButton>
#include <QToolTip>
#include <QUrl>
#include <QWheelEvent>

#include <algorithm>
#include <climits>
#include <cmath>

namespace kerf {

namespace {

// A clip's name bar, where its name and FX badge sit (the whole clip on
// short rows). Tall enough that a selected clip's name keeps a pixel of the
// bar between its ascenders and the 3px ring.
constexpr qreal kClipHeaderHeight = 15.0;
// A clip has a body under its name bar from this height, with room for a
// waveform...
constexpr qreal kClipBodyFrom = kClipHeaderHeight + 10.0;
// ...and a filmstrip from this tall a body, where its pictures still read.
constexpr qreal kFilmstripFrom = 14.0;
// The selected clip's ring, 1px dark and 2px light, inside its edge.
constexpr qreal kSelectionRing = 3.0;
qreal clipHeaderHeight(const QRectF &clip) { return clip.height() >= kClipBodyFrom ? kClipHeaderHeight : clip.height(); }
// Between a lane's edges and its clips: 2px, and 1px on rows too short to spare it.
int clipInset(int rowHeight) { return rowHeight >= 24 ? kClipInset : 1; }

// The header bar's track height symbols: lanes at the height they choose,
// and Fit's lanes between arrows.
enum class HeightGlyph { Mini, Medium, Large, Fit };

QIcon trackHeightIcon(HeightGlyph glyph) {
    const auto draw = [glyph](QPainter &painter, const QColor &color) {
        painter.setPen(Qt::NoPen);
        painter.setBrush(color);
        const auto lanes = [&painter](qreal left, int count, qreal thickness, qreal top, qreal bottom) {
            const qreal gap = count > 1 ? (bottom - top - count * thickness) / (count - 1) : 0.0;
            for (int lane = 0; lane < count; ++lane)
                painter.drawRoundedRect(QRectF(left, top + lane * (thickness + gap), 14.0 - left, thickness), 0.8, 0.8);
        };
        switch (glyph) {
        case HeightGlyph::Mini: lanes(2.0, 4, 1.75, 2.5, 13.5); break;
        case HeightGlyph::Medium: lanes(2.0, 3, 2.75, 2.5, 13.5); break;
        case HeightGlyph::Large: lanes(2.0, 2, 4.75, 2.5, 13.5); break;
        case HeightGlyph::Fit:
            lanes(7.0, 3, 2.0, 4.0, 12.0);
            painter.setBrush(Qt::NoBrush);
            painter.setPen(QPen(color, 1.3, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            painter.drawLine(QPointF(3.5, 2.5), QPointF(3.5, 13.5));
            painter.drawPolyline(QPolygonF{{1.5, 4.5}, {3.5, 2.5}, {5.5, 4.5}});
            painter.drawPolyline(QPolygonF{{1.5, 11.5}, {3.5, 13.5}, {5.5, 11.5}});
            break;
        }
    };
    // A chosen height's symbol is the accent, as a chosen mask tool's.
    QIcon icon = glyphIcon(QSize(16, 16), draw);
    const QPixmap chosen = glyphPixmap(QSize(16, 16), draw, editorTheme().accentText);
    icon.addPixmap(chosen, QIcon::Normal, QIcon::On);
    icon.addPixmap(chosen, QIcon::Active, QIcon::On);
    icon.addPixmap(glyphPixmap(QSize(16, 16), draw, editorTheme().glyphDisabled), QIcon::Disabled, QIcon::On);
    return icon;
}

// `height` moved `steps` steps through kTrackHeightSteps: to the next taller
// step for each one up, the next shorter for each one down.
int steppedHeight(int height, int steps) {
    const int *first = std::begin(kTrackHeightSteps);
    const int *last = std::end(kTrackHeightSteps);
    for (; steps > 0; --steps) {
        const int *next = std::upper_bound(first, last, height);
        if (next == last) break;
        height = *next;
    }
    for (; steps < 0; ++steps) {
        const int *at = std::lower_bound(first, last, height);
        if (at == first) break;
        height = *(at - 1);
    }
    return std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
}

// The selected clip's ring: a 2px light line inside a 1px dark one, which
// reads on every clip hue and on both lane colours.
void paintSelectionRing(QPainter &p, const QRectF &rect) {
    const EditorTheme &theme = editorTheme();
    frameInside(p, rect, 1, theme.clipSelectedRingOuter);
    frameInside(p, rect.adjusted(1, 1, -1, -1), 2, theme.clipSelectedRing);
}

// The kind a track's header stands for: its first clip's, so a picture
// track of titles reads as titles, else the track's own.
ClipKind trackKind(const QJsonObject &track) {
    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    return clipKind(track, clips.isEmpty() ? QJsonObject() : clips.first().toObject());
}

} // namespace

TimelineScrollArea::TimelineScrollArea(QWidget *parent) : QScrollArea(parent) {
    // The timeline covers the whole view, so the view can be scrolled in
    // place: what is on screen moves, and only what scrolls in is painted.
    viewport()->setAttribute(Qt::WA_OpaquePaintEvent);
}

void TimelineScrollArea::scrollContentsBy(int dx, int dy) {
    QWidget *content = widget();
    if (content && content->isVisible() && viewport()->isVisible()) {
        // Moves the timeline with what it last painted; the timeline then
        // repaints its pinned ruler and headers (see its moveEvent).
        viewport()->scroll(dx, dy);
    }
    // Lands it exactly where the scroll bars say (a no-op after the scroll).
    QScrollArea::scrollContentsBy(dx, dy);
}

TimelineWidget::TimelineWidget(QWidget *parent) : QWidget(parent) {
    setFocusPolicy(Qt::StrongFocus);
    setAttribute(Qt::WA_OpaquePaintEvent);
    setSizePolicy(QSizePolicy::Minimum, QSizePolicy::MinimumExpanding);
    updateContentSize();
    setMouseTracking(true);
    // The header bar's track heights: every track Mini, at its default, or
    // Large, and Fit. Option-Up/Down and Option-scroll step every track too.
    auto *heights = new QActionGroup(this);
    heights->setExclusionPolicy(QActionGroup::ExclusionPolicy::ExclusiveOptional);
    const QString stepHint = tr("Option-Up and Option-Down, or Option-scroll over the track headers, step every track.");
    const auto heightAction = [&](const char *id, const QString &tip, HeightGlyph glyph, bool checkable) {
        auto *action = new QAction(trackHeightIcon(glyph), tip, this);
        action->setObjectName(QString::fromLatin1(id));
        action->setToolTip(tip + QLatin1Char('\n') + stepHint);
        action->setData(static_cast<int>(glyph));
        if (checkable) {
            action->setCheckable(true);
            heights->addAction(action);
        }
        return action;
    };
    m_miniTracks = heightAction("trackHeightMini", tr("Mini tracks: one line each"), HeightGlyph::Mini, true);
    m_mediumTracks = heightAction("trackHeightMedium", tr("Medium tracks: the default heights"), HeightGlyph::Medium, true);
    m_largeTracks = heightAction("trackHeightLarge", tr("Large tracks"), HeightGlyph::Large, true);
    m_fitTracks = heightAction("trackHeightFit", tr("Fit tracks to the panel"), HeightGlyph::Fit, false);
    // Triggered, not toggled: choosing the checked height again keeps it.
    connect(m_miniTracks, &QAction::triggered, this, [this] { setAllTrackHeights(kMinTrackHeight); });
    connect(m_mediumTracks, &QAction::triggered, this, &TimelineWidget::resetTrackHeights);
    connect(m_largeTracks, &QAction::triggered, this, [this] { setAllTrackHeights(kTrackHeightPresets[3].height); });
    connect(m_fitTracks, &QAction::triggered, this, &TimelineWidget::fitTracksToView);
    updateHeightActions();
    // The lanes, tinted waveforms, filmstrip tiles and symbols are drawn in
    // the old theme's colours.
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
        m_waveTints.clear();
        m_filmstripTiles.clear();
        m_names.clear();
        for (QAction *action : trackHeightActions())
            action->setIcon(trackHeightIcon(static_cast<HeightGlyph>(action->data().toInt())));
        invalidateLanes();
        update();
    });
}

void TimelineWidget::setProject(const QJsonObject &project, double duration, double fps) {
    m_project = project;
    m_duration = std::max(duration, 1.0);
    m_fps = fps > 0.0 ? fps : 24.0;
    layoutRows();
    invalidateLanes();
    updateContentSize();
    // Remembered heights may have come before any track (setHeightsKey),
    // and a snapshot can add or remove the one track sized apart.
    updateHeightActions();
    update();
}

void TimelineWidget::setSelectedClip(const QString &id) {
    if (m_selectedClip == id && m_selectedClips.size() <= 1) return;
    QStringList before = m_selectedClips;
    before << m_selectedClip;
    m_selectedClip = id;
    m_selectedClips = id.isEmpty() ? QStringList{} : QStringList{id};
    for (const QString &clip : std::as_const(before)) refreshLanes(clipBounds(clip));
    refreshLanes(clipBounds(id));
}

void TimelineWidget::setSelectedClips(const QStringList &ids) {
    QStringList before = m_selectedClips;
    before << m_selectedClip;
    m_selectedClips = ids;
    if (!ids.contains(m_selectedClip)) m_selectedClip = ids.isEmpty() ? QString() : ids.constLast();
    for (const QString &clip : std::as_const(before)) refreshLanes(clipBounds(clip));
    for (const QString &clip : ids) refreshLanes(clipBounds(clip));
}

void TimelineWidget::setPlayhead(double time) {
    // The pointer already owns the line while scrubbing. A quantized seek
    // must not pull it back onto a frame boundary.
    if (m_scrubbing) return;
    const double previous = m_playhead;
    m_playhead = std::clamp(time, 0.0, m_duration);
    repaintAt(timeToX(previous));
    repaintAt(timeToX(m_playhead));
}

void TimelineWidget::showSkimmer(double time) {
    const double previous = m_skimmerTime;
    const bool had = m_hasSkimmer;
    const qreal previousX = m_feedbackPoint.x();
    m_skimmerTime = std::clamp(time, 0.0, m_duration);
    m_hasSkimmer = true;
    m_feedbackPoint = QPointF(timeToX(m_skimmerTime), kRulerHeight + kTrackHeight / 2.0);
    if (had) repaintAt(timeToX(previous));
    repaintAt(timeToX(m_skimmerTime));
    repaintAt(previousX);
    repaintAt(m_feedbackPoint.x());
}

void TimelineWidget::setZoom(double zoom) {
    m_zoom = std::clamp(zoom, kMinScale / 72.0, kMaxScale / 72.0);
    m_fitWidth = 0;
    invalidateLanes();
    updateContentSize();
    update();
    emit scaleChanged(pixelsPerSecond());
}

void TimelineWidget::setScale(double pixelsPerSecond) { setZoom(pixelsPerSecond / 72.0); }

void TimelineWidget::fitToWidth(int width) {
    m_fitWidth = std::max(kTimelineLabelWidth + 32, width);
    invalidateLanes();
    updateContentSize();
    update();
    emit scaleChanged(pixelsPerSecond());
}

void TimelineWidget::setArtwork(MediaArtwork *artwork) {
    m_artwork = artwork;
    connect(artwork, &MediaArtwork::changed, this, [this] {
        m_waveTints.clear();
        m_filmstripTiles.clear();
        invalidateLanes();
        update();
    });
}

void TimelineWidget::clearSkimmer() {
    if (!m_hasSkimmer) return;
    const double previous = m_skimmerTime;
    m_hasSkimmer = false;
    m_hoverFrame = -1.0;
    repaintAt(timeToX(previous));
    repaintAt(timeToX(m_playhead));
    repaintAt(m_feedbackPoint.x());
}

void TimelineWidget::fitTracksToView() {
    if (m_rows.isEmpty()) return;
    const int available = viewportRect().height() - kRulerHeight - kAddLaneHeight - 8;
    setAllTrackHeights(available / static_cast<int>(m_rows.size()));
}

void TimelineWidget::resetTrackHeights() {
    const int keep = firstVisibleRow();
    m_trackHeights.clear();
    m_allHeight = 0;
    relayoutKeeping(keep);
    saveHeights();
}

void TimelineWidget::setAllTrackHeights(int height) {
    setAllTrackHeightsKeeping(height, firstVisibleRow());
}

void TimelineWidget::setAllTrackHeightsKeeping(int height, int row) {
    m_trackHeights.clear();
    m_allHeight = std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
    relayoutKeeping(row);
    saveHeights();
}

void TimelineWidget::stepAllTrackHeights(int steps) {
    stepAllTrackHeightsKeeping(steps, firstVisibleRow());
}

void TimelineWidget::stepAllTrackHeightsKeeping(int steps, int row) {
    if (steps == 0 || m_rows.isEmpty()) return;
    // Each track from its own height; tracks added later from the height
    // tracks without one of their own have. Only the tracks that end up
    // apart from that keep a height of their own.
    m_allHeight = steppedHeight(defaultTrackHeight(false), steps);
    QHash<QString, int> heights;
    for (int index = 0; index < m_rows.size(); ++index) {
        const int height = steppedHeight(m_rows.at(index).height, steps);
        if (height != m_allHeight) heights.insert(rowKey(index), height);
    }
    m_trackHeights = heights;
    relayoutKeeping(row);
    saveHeights();
}

QList<QAction *> TimelineWidget::trackHeightActions() const {
    return {m_miniTracks, m_mediumTracks, m_largeTracks, m_fitTracks};
}

int TimelineWidget::defaultTrackHeight(bool audio) const {
    if (m_allHeight > 0) return m_allHeight;
    return audio ? kTrackHeight : kVideoTrackHeight;
}

void TimelineWidget::updateHeightActions() {
    if (!m_miniTracks) return;
    const auto everyTrackAt = [this](int height) {
        return !m_rows.isEmpty() && std::all_of(m_rows.cbegin(), m_rows.cend(), [height](const TrackRow &row) {
            return row.height == height;
        });
    };
    QAction *chosen = m_trackHeights.isEmpty() && m_allHeight == 0 ? m_mediumTracks
        : everyTrackAt(kMinTrackHeight)                            ? m_miniTracks
        : everyTrackAt(kTrackHeightPresets[3].height)              ? m_largeTracks
                                                                   : nullptr;
    for (QAction *action : {m_miniTracks, m_mediumTracks, m_largeTracks}) action->setChecked(action == chosen);
}

void TimelineWidget::setHeightsKey(const QString &projectPath) {
    m_heightsPath = projectPath.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(projectPath).absoluteFilePath());
    m_heightsKey = m_heightsPath.isEmpty()
        ? QString()
        : QStringLiteral("timelineTrackHeights/") +
              QString::fromLatin1(QCryptographicHash::hash(m_heightsPath.toUtf8(), QCryptographicHash::Sha1).toHex());
    if (m_heightsKey.isEmpty()) return;
    QSettings settings;
    settings.beginGroup(m_heightsKey);
    const int all = settings.value(QStringLiteral("all")).toInt();
    m_allHeight = all > 0 ? std::clamp(all, kMinTrackHeight, kMaxTrackHeight) : 0;
    m_trackHeights.clear();
    settings.beginGroup(QStringLiteral("tracks"));
    for (const QString &key : settings.childKeys()) {
        const int height = settings.value(key).toInt();
        if (height > 0)
            m_trackHeights.insert(QUrl::fromPercentEncoding(key.toUtf8()), std::clamp(height, kMinTrackHeight, kMaxTrackHeight));
    }
    relayout();
}

void TimelineWidget::saveHeights() const {
    if (m_heightsKey.isEmpty()) return;
    QSettings settings;
    settings.remove(m_heightsKey);
    settings.beginGroup(m_heightsKey);
    // The path itself, so the group can be told apart by a person reading it.
    settings.setValue(QStringLiteral("path"), m_heightsPath);
    if (m_allHeight > 0) settings.setValue(QStringLiteral("all"), m_allHeight);
    settings.beginGroup(QStringLiteral("tracks"));
    for (auto it = m_trackHeights.cbegin(); it != m_trackHeights.cend(); ++it)
        settings.setValue(QString::fromLatin1(QUrl::toPercentEncoding(it.key())), it.value());
}

int TimelineWidget::firstVisibleRow() const {
    if (m_rows.isEmpty()) return -1;
    const int row = rowAt(viewportRect().top() + kRulerHeight);
    return row >= 0 ? row : 0;
}

void TimelineWidget::relayoutKeeping(int row) {
    QScrollArea *area = isWindow() || !parentWidget() ? nullptr : qobject_cast<QScrollArea *>(parentWidget()->parentWidget());
    if (!area || area->widget() != this || row < 0 || row >= m_rows.size()) {
        relayout();
        return;
    }
    const int inView = m_rows.at(row).top + pos().y();
    relayout();
    // The scroll area takes the new size now, so its bars can reach the row.
    QCoreApplication::sendPostedEvents(nullptr, QEvent::LayoutRequest);
    area->verticalScrollBar()->setValue(m_rows.value(row).top - inView);
}

QString TimelineWidget::trackRowLabel(int row) const {
    const TrackRow track = m_rows.value(row);
    return (track.audio ? QStringLiteral("A") : QStringLiteral("V")) + QString::number(track.number);
}

QRect TimelineWidget::trackHeaderRect(int row) const {
    const TrackRow track = m_rows.value(row);
    return QRect(stickyOrigin().x(), track.top, kTimelineLabelWidth, track.height);
}

QRect TimelineWidget::headerBadgeRect(int row) const {
    if (row < 0 || row >= m_rows.size()) return {};
    const int size = headerControlSize(m_rows.at(row).height);
    return QRect(stickyOrigin().x() + 10, headerControlTop(row), m_badgeWidth, size);
}

bool TimelineWidget::event(QEvent *event) {
    // The line between two headers says what dragging it does.
    if (event->type() == QEvent::ToolTip) {
        const auto *help = static_cast<QHelpEvent *>(event);
        const int grip = resizeRowAt(help->pos());
        if (grip < 0) {
            QToolTip::hideText();
            event->ignore();
            return true;
        }
        QToolTip::showText(help->globalPos(),
                           tr("Drag to resize %1. Option-drag resizes every %2 track; Option-scroll over the headers "
                              "resizes every track.")
                               .arg(trackRowLabel(grip), m_rows.at(grip).audio ? tr("sound") : tr("picture")),
                           this);
        return true;
    }
    return QWidget::event(event);
}

void TimelineWidget::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    const QRegion region = event->region();
    const QRect dirty = region.boundingRect();
    // Lanes come from a cache of the part in view; the ruler, track headers,
    // and playhead are painted on top at the scroll position so they stay
    // pinned. The cache is rebuilt when a repaint covers most of the view
    // (the lanes changed, or a jump scrolled them away); a strip that
    // scrolls into view, or the band the playhead leaves, outside it is
    // painted directly.
    const QRect view = viewportRect();
    if (!laneCacheFits(view)) {
        qint64 area = 0;
        for (const QRect &rect : region.intersected(view)) area += qint64(rect.width()) * rect.height();
        if (area * 2 >= qint64(view.width()) * view.height()) ensureLaneCache(view);
    }
    QRegion direct = region;
    if (!m_laneCache.isNull()) {
        const QRegion cached = region.intersected(m_laneCacheRect);
        if (!cached.isEmpty()) blitLaneCache(p, m_laneCache, cached, m_laneCacheRect.topLeft());
        direct -= cached;
    }
    for (const QRect &visible : direct) paintLanes(p, visible);
    p.setClipRegion(region);
    paintOverlay(p, dirty);
}

void TimelineWidget::resizeEvent(QResizeEvent *event) {
    invalidateLanes();
    QWidget::resizeEvent(event);
    update();
}

void TimelineWidget::moveEvent(QMoveEvent *event) {
    QWidget::moveEvent(event);
    if (isWindow() || !parentWidget()) return;
    // A scroll area scrolls by moving this widget. What it showed moves
    // with it, so the lanes are already in place: the ruler, pinned to the
    // view's top, and the headers, pinned to its left, are repainted where
    // they now go and where they were left. A drag's readout is pinned too.
    if (m_dragging || m_scrubbing || m_resizeRow >= 0) {
        update();
        return;
    }
    const QPoint before = -event->oldPos();
    const QPoint now = -event->pos();
    const QSize view = parentWidget()->size();
    const int left = std::min(before.x(), now.x());
    const int top = std::min(before.y(), now.y());
    const int wide = view.width() + std::abs(now.x() - before.x());
    const int tall = view.height() + std::abs(now.y() - before.y());
    if (before.y() != now.y()) {
        update(QRect(left, before.y(), wide, kRulerHeight));
        update(QRect(left, now.y(), wide, kRulerHeight));
    }
    if (before.x() != now.x()) {
        update(QRect(before.x(), top, kTimelineLabelWidth, tall));
        update(QRect(now.x(), top, kTimelineLabelWidth, tall));
    }
}

void TimelineWidget::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return;
    setFocus();
    clearSkimmer();
    const QPoint pos = event->position().toPoint();
    const int grip = resizeRowAt(pos);
    if (grip >= 0) {
        m_resizeRow = grip;
        m_resizeAnchor = pos.y();
        m_resizeStartHeight = m_rows.at(grip).height;
        // Option-drag: every track of the grabbed one's kind.
        m_resizeKind = event->modifiers() & Qt::AltModifier;
        return;
    }
    if (addLaneHitRect().contains(pos)) {
        m_addLaneVisible = false;
        update();
        emit addLaneRequested();
        return;
    }
    m_addLaneVisible = false;
    int toggleRow = -1;
    QString field;
    if (toggleAt(pos, &toggleRow, &field)) {
        const QJsonObject track = m_project.value(QStringLiteral("tracks")).toArray().at(m_rows.at(toggleRow).track).toObject();
        const bool current = track.value(field).toBool(field == QStringLiteral("enabled"));
        emit trackToggleRequested(track.value(QStringLiteral("id")).toString(), field, !current);
        return;
    }
    if (inHeader(pos)) return;
    if (inRuler(pos)) {
        beginScrub(event->position());
        return;
    }
    const auto hit = clipAt(pos);
    if (hit.id.isEmpty()) {
        if (!m_selectedClip.isEmpty()) emit deselectRequested();
        beginScrub(event->position());
        return;
    }
    if (m_tool == TimelineTool::Razor) {
        emit razorRequested(hit.id, static_cast<double>(std::llround(timeAt(event->position().x()) * m_fps)));
        return;
    }
    // ⌘-click toggles a clip in the selection, ⇧-click adds it; neither drags.
    if (event->modifiers() & (Qt::ControlModifier | Qt::ShiftModifier)) {
        emit clipSelectionExtended(hit.id, event->modifiers() & Qt::ControlModifier);
        return;
    }
    const bool onEdge = pos.x() - hit.rect.left() < kTrimHandle || hit.rect.right() - pos.x() < kTrimHandle;
    // A tool that has nothing to drag here only selects.
    if (!(onEdge && trimsEdges()) && !dragsBodies()) {
        emit clipSelected(hit.id);
        return;
    }
    m_dragging = true;
    m_dragMoved = false;
    m_snapTime = -1.0;
    m_dragClipId = hit.id;
    // What the drag can snap to, gathered once rather than on every move.
    m_snapPoints = {0.0, m_playhead};
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray()) {
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("id")).toString() == m_dragClipId) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            m_snapPoints << start << start + clip.value(QStringLiteral("duration")).toDouble();
        }
    }
    for (const QJsonValue &value : m_project.value(QStringLiteral("markers")).toArray()) {
        const QJsonObject marker = value.toObject();
        const double at = marker.value(QStringLiteral("time")).toDouble();
        m_snapPoints << at << at + marker.value(QStringLiteral("duration")).toDouble();
    }
    std::sort(m_snapPoints.begin(), m_snapPoints.end());
    m_dragRow = hit.row;
    m_dragStart = hit.start;
    m_dragDuration = hit.duration;
    m_pressTime = timeAt(event->position().x());
    m_dragLeading = trimsEdges() && pos.x() - hit.rect.left() < kTrimHandle;
    m_dragTrailing = trimsEdges() && hit.rect.right() - pos.x() < kTrimHandle;
    // A trailing trim previews the out point; starting from the in point
    // would collapse the clip to one frame until the pointer moves.
    m_previewTime = m_dragTrailing && !m_dragLeading ? hit.start + hit.duration : hit.start;
    emit clipSelected(hit.id);
}

void TimelineWidget::mouseMoveEvent(QMouseEvent *event) {
    if (m_resizeRow >= 0) {
        const int height = m_resizeStartHeight + event->position().toPoint().y() - m_resizeAnchor;
        if (!m_resizeKind) {
            setTrackHeight(m_resizeRow, height);
            return;
        }
        // Rows above the grabbed one can change too, so it is kept under the pointer.
        const int clamped = std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
        const bool audio = m_rows.at(m_resizeRow).audio;
        for (int row = 0; row < m_rows.size(); ++row)
            if (m_rows.at(row).audio == audio) m_trackHeights.insert(rowKey(row), clamped);
        const int grabbedTop = m_rows.at(m_resizeRow).top;
        relayoutKeeping(m_resizeRow);
        m_resizeAnchor += m_rows.at(m_resizeRow).top - grabbedTop;
        return;
    }
    if (m_scrubbing) {
        const double previous = m_playhead;
        m_feedbackPoint = event->position();
        m_playhead = scrubTime(m_feedbackPoint.x());
        repaintAt(timeToX(previous));
        repaintAt(timeToX(m_playhead));
        emit seekRequested(m_playhead);
        return;
    }
    if (!m_dragging) {
        const QPoint pos = event->position().toPoint();
        const int grip = resizeRowAt(pos);
        if (grip != m_hoverGrip) {
            m_hoverGrip = grip;
            update(headerColumnRect());
        }
        int toggleRow = -1;
        QString toggleField;
        const int toggle = grip < 0 && toggleAt(pos, &toggleRow, &toggleField)
                               ? toggleRow * 2 + (toggleField == QStringLiteral("locked") ? 1 : 0)
                               : -1;
        if (toggle != m_hoverToggle) {
            m_hoverToggle = toggle;
            update(headerColumnRect());
        }
        if (grip >= 0) {
            if (m_addLaneVisible) {
                m_addLaneVisible = false;
                update(addLaneHitRect());
            }
            clearHoverClip();
            updateSkimmer(event->position());
            setCursor(Qt::SplitVCursor);
            return;
        }
        updateAddLane(pos);
        if (m_addLaneVisible && addLaneHitRect().contains(pos)) {
            clearHoverClip();
            if (m_hasSkimmer) {
                m_hasSkimmer = false;
                m_hoverFrame = -1.0;
                emit hoverSeekEnded();
            }
            setCursor(Qt::PointingHandCursor);
            update();
            return;
        }
        const ClipHit hit = clipAt(pos);
        const qreal x = event->position().x();
        const int edge = hit.id.isEmpty() ? 0
            : x - hit.rect.left() < kTrimHandle ? -1
            : hit.rect.right() - x < kTrimHandle ? 1 : 0;
        if (hit.id != m_hoverClipId || edge != m_hoverEdge) {
            const QRect before = m_hoverRect;
            m_hoverClipId = hit.id;
            m_hoverEdge = edge;
            m_hoverRect = hit.id.isEmpty() ? QRect() : hit.rect.toAlignedRect();
            refreshLanes(before);
            refreshLanes(m_hoverRect);
        }
        setToolCursor(hit, edge);
        updateSkimmer(event->position());
        return;
    }
    clearSkimmer();
    const double delta = timeAt(event->position().x()) - m_pressTime;
    m_dragMoved = m_dragMoved || std::abs(delta * pixelsPerSecond()) >= QApplication::startDragDistance();
    const bool body = !m_dragLeading && !m_dragTrailing;
    m_previewTime = snapped(std::round(std::max(0.0, dragOrigin() + delta) * m_fps) / m_fps,
                            body && m_tool != TimelineTool::Slip ? m_dragDuration : 0.0, &m_snapTime);
    setCursor((m_dragLeading || m_dragTrailing) ? Qt::SizeHorCursor : Qt::ClosedHandCursor);
    invalidateLanes();
    update();
}

void TimelineWidget::mouseReleaseEvent(QMouseEvent *event) {
    if (m_resizeRow >= 0) {
        m_resizeRow = -1;
        m_resizeKind = false;
        saveHeights();
        update(headerColumnRect());
        return;
    }
    if (m_scrubbing) {
        m_scrubbing = false;
        releaseMouse();
        const double previous = m_playhead;
        m_feedbackPoint = event->position();
        m_playhead = scrubTime(m_feedbackPoint.x());
        repaintAt(timeToX(previous));
        repaintAt(timeToX(m_playhead));
        emit seekRequested(m_playhead);
        return;
    }
    if (!m_dragging) return;
    m_dragging = false;
    m_snapTime = -1.0;
    const double delta = timeAt(event->position().x()) - m_pressTime;
    if (std::abs(delta * pixelsPerSecond()) < QApplication::startDragDistance()) {
        m_dragClipId.clear();
        invalidateLanes();
        update();
        return;
    }
    // The drop lands where the preview showed it, snapping included.
    if (m_dragLeading || m_dragTrailing) {
        emit trimRequested(m_dragClipId, m_dragLeading, static_cast<double>(std::llround(m_previewTime * m_fps)));
    } else {
        emit moveRequested(m_dragClipId, m_previewTime);
    }
    m_dragClipId.clear();
    setCursor(Qt::ArrowCursor);
    invalidateLanes();
    update();
}

void TimelineWidget::mouseDoubleClickEvent(QMouseEvent *event) {
    const ClipHit hit = clipAt(event->position().toPoint());
    if (event->button() != Qt::LeftButton || hit.id.isEmpty()) return;
    if (event->modifiers() & Qt::AltModifier) emit askRequested(hit.id);
    else emit clipActivated(hit.id);
}

void TimelineWidget::contextMenuEvent(QContextMenuEvent *event) {
    const QPoint pos = event->pos();
    if (const ClipHit hit = clipAt(pos); !hit.id.isEmpty()) {
        emit clipSelected(hit.id);
        emit clipMenuRequested(hit.id, event->globalPos());
        return;
    }
    if (!inHeader(pos) || inRuler(pos)) return;
    const int row = rowAt(pos.y());
    QMenu menu(this);
    // Medium is a track's default height.
    const auto presetHeight = [this](const TrackHeightPreset &preset, bool audio) {
        return preset.height > 0 ? preset.height : defaultTrackHeight(audio);
    };
    if (row >= 0) {
        // A key, not a row index: a snapshot can relayout rows while the menu is open.
        const QString key = rowKey(row);
        const TrackRow track = m_rows.at(row);
        QMenu *one = menu.addMenu(QStringLiteral("Track height"));
        for (const auto &preset : kTrackHeightPresets) {
            QAction *action = one->addAction(QString::fromLatin1(preset.name));
            action->setCheckable(true);
            action->setChecked(track.height == presetHeight(preset, track.audio));
            connect(action, &QAction::triggered, this, [this, key, height = preset.height] {
                if (height > 0) m_trackHeights.insert(key, height);
                else m_trackHeights.remove(key);
                relayout();
                saveHeights();
            });
        }
    }
    if (!m_rows.isEmpty()) {
        QMenu *all = menu.addMenu(QStringLiteral("All tracks"));
        for (const auto &preset : kTrackHeightPresets) {
            QAction *action = all->addAction(QString::fromLatin1(preset.name));
            action->setCheckable(true);
            action->setChecked(std::all_of(m_rows.cbegin(), m_rows.cend(), [&](const TrackRow &track) {
                return track.height == presetHeight(preset, track.audio);
            }));
            connect(action, &QAction::triggered, this, [this, height = preset.height] {
                if (height > 0) setAllTrackHeights(height);
                else resetTrackHeights();
            });
        }
        menu.addSeparator();
        menu.addAction(QStringLiteral("Fit tracks to panel"), this, &TimelineWidget::fitTracksToView);
        menu.addAction(QStringLiteral("Reset track heights"), this, &TimelineWidget::resetTrackHeights);
    }
    if (!menu.isEmpty()) menu.exec(event->globalPos());
}

void TimelineWidget::keyPressEvent(QKeyEvent *event) {
    if (event->key() == Qt::Key_Escape && event->modifiers() == Qt::NoModifier && !m_selectedClip.isEmpty()) {
        emit deselectRequested();
        return;
    }
    // Option-Up and Option-Down: every track a step taller or shorter.
    const Qt::KeyboardModifiers modifiers = event->modifiers() & ~Qt::KeypadModifier;
    if (modifiers == Qt::AltModifier && (event->key() == Qt::Key_Up || event->key() == Qt::Key_Down)) {
        stepAllTrackHeights(event->key() == Qt::Key_Up ? 1 : -1);
        return;
    }
    QWidget::keyPressEvent(event);
}

void TimelineWidget::wheelEvent(QWheelEvent *event) {
    const QPoint pos = event->position().toPoint();
    if (!(event->modifiers() & Qt::AltModifier) || !inHeader(pos) || inRuler(pos) || m_rows.isEmpty()) {
        m_wheelSteps = 0;
        // The scroll area scrolls.
        event->ignore();
        return;
    }
    // Option-scroll over the headers steps every track, a step a notch (a
    // trackpad's small deltas add up to one), keeping the track under the
    // pointer in place. Some platforms turn Option-scroll sideways.
    // A trackpad's momentum after the fingers lift would carry on stepping
    // well past the height aimed for.
    event->accept();
    if (event->phase() == Qt::ScrollBegin || event->phase() == Qt::ScrollEnd) m_wheelSteps = 0;
    if (event->phase() == Qt::ScrollMomentum) return;
    const QPoint angle = event->angleDelta();
    m_wheelSteps += angle.y() != 0 ? angle.y() : angle.x();
    const int steps = m_wheelSteps / 120;
    if (steps == 0) return;
    m_wheelSteps -= steps * 120;
    const int row = rowAt(pos.y());
    stepAllTrackHeightsKeeping(steps, row >= 0 ? row : static_cast<int>(m_rows.size()) - 1);
}

void TimelineWidget::leaveEvent(QEvent *) {
    if (m_scrubbing || m_dragging || m_resizeRow >= 0) return;
    clearHoverClip();
    m_hoverGrip = -1;
    if (m_hoverToggle >= 0) {
        m_hoverToggle = -1;
        update(headerColumnRect());
    }
    m_addLaneVisible = false;
    m_hoverFrame = -1.0;
    if (m_hasSkimmer) {
        m_hasSkimmer = false;
        emit hoverSeekEnded();
    }
    setCursor(Qt::ArrowCursor);
    update();
}

QString TimelineWidget::trackKey(const QJsonObject &track, int index) {
    const QString id = track.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("#%1").arg(index) : id;
}

QString TimelineWidget::rowKey(int row) const {
    const int index = m_rows.value(row).track;
    return trackKey(m_project.value(QStringLiteral("tracks")).toArray().at(index).toObject(), index);
}

void TimelineWidget::layoutRows() {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    QVector<int> picture;
    QVector<int> sound;
    for (int index = 0; index < tracks.size(); ++index)
        (isAudioTrack(tracks.at(index).toObject()) ? sound : picture).append(index);
    m_rows.clear();
    m_rows.reserve(tracks.size());
    int top = kRulerHeight;
    const auto add = [&](int index, bool audio, int number) {
        const int height = m_trackHeights.value(trackKey(tracks.at(index).toObject(), index), defaultTrackHeight(audio));
        m_rows.append(TrackRow{index, audio, number, top, height});
        top += height;
    };
    for (int number = static_cast<int>(picture.size()); number >= 1; --number)
        add(picture.at(number - 1), false, number);
    for (int number = 1; number <= static_cast<int>(sound.size()); ++number)
        add(sound.at(number - 1), true, number);
    const QFontMetrics metrics(badgeFont());
    m_badgeWidth = std::max({22, metrics.horizontalAdvance(QStringLiteral("V%1").arg(picture.size())) + 8,
                             metrics.horizontalAdvance(QStringLiteral("A%1").arg(sound.size())) + 8});
}

void TimelineWidget::relayout() {
    layoutRows();
    invalidateLanes();
    updateContentSize();
    updateHeightActions();
    update();
}

void TimelineWidget::setTrackHeight(int row, int height) {
    if (row < 0 || row >= m_rows.size()) return;
    const int clamped = std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
    if (clamped == m_rows.at(row).height) return;
    m_trackHeights.insert(rowKey(row), clamped);
    relayout();
}

int TimelineWidget::rowAt(int y) const {
    // Rows run down without gaps, so the row is the last one starting at or above `y`.
    const auto after = std::upper_bound(m_rows.cbegin(), m_rows.cend(), y,
                                        [](int value, const TrackRow &row) { return value < row.top; });
    if (after == m_rows.cbegin()) return -1;
    const auto row = after - 1;
    return y < row->bottom() ? static_cast<int>(row - m_rows.cbegin()) : -1;
}

std::pair<int, int> TimelineWidget::rowsBetween(int top, int bottom) const {
    const auto first = std::upper_bound(m_rows.cbegin(), m_rows.cend(), top,
                                        [](int value, const TrackRow &row) { return value < row.bottom(); });
    const auto last = std::upper_bound(first, m_rows.cend(), bottom,
                                       [](int value, const TrackRow &row) { return value < row.top; });
    return {static_cast<int>(first - m_rows.cbegin()), static_cast<int>(last - m_rows.cbegin())};
}

QRect TimelineWidget::viewportRect() const {
    if (isWindow() || !parentWidget()) return rect();
    return QRect(-pos(), parentWidget()->size()).intersected(rect());
}

QRect TimelineWidget::headerColumnRect() const {
    const QPoint origin = stickyOrigin();
    return QRect(origin.x(), origin.y() + kRulerHeight, kTimelineLabelWidth,
                 std::max(0, height() - origin.y() - kRulerHeight));
}

int TimelineWidget::resizeRowAt(const QPoint &point) const {
    if (!inHeader(point) || inRuler(point) || m_rows.isEmpty()) return -1;
    // A toggle keeps its clicks, however short its row.
    int toggleRow = -1;
    QString field;
    if (toggleAt(point, &toggleRow, &field)) return -1;
    // The line under a row, and how far from it a press still grabs it: a
    // sixth of the shorter of the two rows it parts, so short rows keep
    // most of their height for their own controls.
    const auto reach = [this](int row) {
        const int below = row + 1 < m_rows.size() ? m_rows.at(row + 1).height : m_rows.at(row).height;
        return std::clamp(std::min(m_rows.at(row).height, below) / 6, 2, kResizeGrip);
    };
    int row = rowAt(point.y());
    if (row < 0) row = point.y() >= tracksBottom() ? static_cast<int>(m_rows.size()) - 1 : -1;
    if (row < 0) return -1;
    int best = -1;
    int nearest = INT_MAX;
    for (const int candidate : {row - 1, row}) {
        if (candidate < 0) continue;
        const int distance = std::abs(point.y() - m_rows.at(candidate).bottom());
        if (distance <= reach(candidate) && distance < nearest) {
            best = candidate;
            nearest = distance;
        }
    }
    return best;
}

void TimelineWidget::clearHoverClip() {
    if (m_hoverClipId.isEmpty()) return;
    m_hoverClipId.clear();
    m_hoverEdge = 0;
    refreshLanes(std::exchange(m_hoverRect, QRect()));
}

void TimelineWidget::refreshLanes(const QRect &rect) {
    if (rect.isEmpty()) return;
    // A selected clip's ring and a trim bracket stay inside the clip.
    const QRect area = rect.adjusted(-1, -1, 1, 1);
    const QRect cached = area.intersected(m_laneCacheRect);
    if (!m_laneCache.isNull() && !cached.isEmpty()) {
        QPainter painter(&m_laneCache);
        painter.translate(-m_laneCacheRect.topLeft());
        painter.setClipRect(cached);
        paintLanes(painter, cached);
    }
    update(area);
}

void TimelineWidget::drawName(QPainter &p, const QRectF &rect, const QFont &font, const QString &text, bool centred) {
    const int width = std::max(1, int(rect.width()));
    const QString key = QString::number(font.pixelSize()) + QLatin1Char('/') + QString::number(font.weight()) +
                        QLatin1Char('/') + QString::number(width) + QLatin1Char('|') + text;
    auto laid = m_names.constFind(key);
    if (laid == m_names.cend()) {
        if (m_names.size() > 8192) m_names.clear();
        QStaticText name(QFontMetrics(font).elidedText(text, Qt::ElideRight, width));
        name.setTextFormat(Qt::PlainText);
        name.setPerformanceHint(QStaticText::AggressiveCaching);
        name.prepare(QTransform(), font);
        laid = m_names.insert(key, name);
    }
    p.setFont(font);
    const qreal left = centred ? rect.left() + (rect.width() - laid->size().width()) / 2.0 : rect.left();
    p.drawStaticText(QPointF(left, rect.top() + (rect.height() - QFontMetricsF(font).height()) / 2.0), laid.value());
}

void TimelineWidget::paintLanes(QPainter &p, const QRect &visible) {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    // Nothing outside the rect: the rest of a row may already be on screen,
    // blitted from the cache or painted for another rect.
    p.save();
    p.setClipRect(visible, Qt::IntersectClip);
    p.setRenderHint(QPainter::Antialiasing, false);
    // Below the last track is an empty well, so spare height never reads as a track.
    p.fillRect(visible, editorTheme().timelineBackground);
    // Only the rows the rect meets, found by binary search.
    const auto [first, last] = rowsBetween(visible.top(), visible.bottom());
    for (int row = first; row < last; ++row) {
        paintLane(p, visible, row, tracks.at(m_rows.at(row).track).toObject());
        ++m_lanesPainted;
    }
    p.restore();
}

void TimelineWidget::paintLane(QPainter &p, const QRect &visible, int row, const QJsonObject &track) {
    const EditorTheme &theme = editorTheme();
    const TrackRow &layout = m_rows.at(row);
    const QRect lane(kTimelineLabelWidth, layout.top, std::max(0, width() - kTimelineLabelWidth), layout.height - 1);
    p.fillRect(lane, theme.trackLane);
    if (track.value(QStringLiteral("locked")).toBool())
        p.fillRect(lane, QBrush(theme.trackLaneLocked, Qt::BDiagPattern));

    p.save();
    p.setClipRect(lane.adjusted(0, 0, 0, 1).intersected(visible));
    if (!track.value(QStringLiteral("enabled")).toBool(true)) p.setOpacity(0.4);
    // The clip being dragged goes over its neighbours, after them.
    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    QJsonObject dragged;
    for (const QJsonValue &value : clips) {
        const QJsonObject clip = value.toObject();
        if (m_dragging && clip.value(QStringLiteral("id")).toString() == m_dragClipId) dragged = clip;
        else paintClip(p, visible, row, clip, track, layout.audio);
    }
    if (!m_proposal.isEmpty()) paintProposal(p, row, track);
    // The selection ring goes over the proposal's marks, so one selected
    // clip never reads as two.
    for (const QJsonValue &value : clips) {
        const QJsonObject clip = value.toObject();
        const QString id = clip.value(QStringLiteral("id")).toString();
        if (!isSelected(id) || (m_dragging && id == m_dragClipId)) continue;
        const QRectF rect = clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(),
                                     clip.value(QStringLiteral("duration")).toDouble());
        if (rect.intersects(visible)) paintSelectionRing(p, rect);
    }
    if (!dragged.isEmpty()) paintDraggedClip(p, visible, row, dragged, track, layout.audio);
    p.restore();

    p.fillRect(lane.left(), layout.bottom() - 1, lane.width(), 1, theme.trackDivider);
    // Picture and sound read as two banks, the way a conform is laid out.
    if (row > 0 && m_rows.at(row - 1).audio != layout.audio)
        p.fillRect(lane.left(), layout.top, lane.width(), 2, theme.trackBankDivider);
}

void TimelineWidget::paintHeaders(QPainter &p, const QRect &dirty) {
    const QRect column = headerColumnRect();
    if (!column.intersects(dirty)) return;
    const EditorTheme &theme = editorTheme();
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    p.save();
    p.setClipRect(column, Qt::IntersectClip);
    p.fillRect(column, theme.trackHeaderColumn);
    const auto [first, last] = rowsBetween(std::max(column.top(), dirty.top()), std::min(column.bottom(), dirty.bottom()));
    for (int row = first; row < last; ++row)
        paintTrackHeader(p, row, tracks.at(m_rows.at(row).track).toObject(), column.left());
    const int grip = m_resizeRow >= 0 ? m_resizeRow : m_hoverGrip;
    if (grip >= 0 && grip < m_rows.size())
        p.fillRect(column.left(), m_rows.at(grip).bottom() - 2, column.width() - 1, 2, theme.trackResizeGrip);
    p.fillRect(column.left() + kTimelineLabelWidth - 1, column.top(), 1, column.height(), theme.divider);
    p.restore();
}

void TimelineWidget::paintTrackHeader(QPainter &p, int row, const QJsonObject &track, int left) {
    const EditorTheme &theme = editorTheme();
    const TrackRow &layout = m_rows.at(row);
    const ClipStyle &style = theme.clip(trackKind(track));
    const bool enabled = track.value(QStringLiteral("enabled")).toBool(true);
    const QRect header(left, layout.top, kTimelineLabelWidth - 1, layout.height - 1);
    p.fillRect(header, theme.trackHeader);
    p.fillRect(QRect(left, layout.top, header.width(), 1), theme.trackHeaderEdge);
    // The stripe takes the colour of the track's clips; a disabled track's fades into the header.
    p.fillRect(QRect(left, layout.top, 3, header.height()),
               enabled ? style.header : mixColor(theme.trackHeader, style.header, 0.35));
    p.fillRect(QRect(left, layout.bottom() - 1, kTimelineLabelWidth, 1), theme.trackDivider);
    if (row > 0 && m_rows.at(row - 1).audio != layout.audio)
        p.fillRect(QRect(left, layout.top, kTimelineLabelWidth, 2), theme.trackBankDivider);

    // Tall rows show a name line over a detail line and a preview; shorter
    // rows are one line: the track number, toggles and name, centred, with
    // the badge and toggles shrinking on rows too short for 16px.
    const bool detailed = layout.height >= kDetailedTrackHeight;
    const int blockTop = headerControlTop(row);
    // The track number as plain secondary text, no fill under it.
    const QRect badge(left + 10, blockTop, m_badgeWidth, headerControlSize(layout.height));
    p.setPen(enabled ? theme.trackBadgeText : theme.trackHeaderTextDisabled);
    drawName(p, badge, badgeFont(), trackRowLabel(row), true);

    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    QImage preview;
    if (detailed && m_artwork && !clips.isEmpty()) {
        const QString sourceId = clips.first().toObject().value(QStringLiteral("sourceId")).toString();
        if (layout.audio && m_artwork->waves.contains(sourceId)) {
            preview = tintedWave(sourceId, style.wave);
        } else if (!layout.audio) {
            preview = m_artwork->frames.value(sourceId);
        }
    }
    const QRect slot = headerPreviewRect(row, left);
    if (!preview.isNull()) {
        p.fillRect(slot, theme.headerPreview);
        p.drawImage(slot.adjusted(1, 1, -1, -1), preview);
        frameInside(p, slot, 1, theme.headerPreviewBorder);
    }

    QFont nameFont = font();
    nameFont.setPixelSize(11);
    nameFont.setWeight(QFont::Medium);
    p.setFont(nameFont);
    // Output (or mute) and lock toggles sit after the track number: bare
    // symbols, the secondary ink when on and faint when off, on a round
    // wash only under the pointer.
    const auto drawToggle = [&](int index, bool on, const QString &field) {
        const QRect box = headerToggleRect(row, index).translated(left - stickyOrigin().x(), 0);
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        if (m_hoverToggle == row * 2 + index) {
            p.setPen(Qt::NoPen);
            p.setBrush(theme.trackToggleHover);
            const double radius = std::max(box.width(), box.height()) / 2.0 + 2.0;
            p.drawEllipse(QRectF(box).center(), radius, radius);
        }
        p.setPen(QPen(on ? theme.trackToggleOn : theme.trackToggle, 1.3));
        p.setBrush(Qt::NoBrush);
        const QPointF c = QRectF(box).center();
        if (field == QStringLiteral("locked")) {
            p.drawArc(QRectF(c.x() - 3, c.y() - 6, 6, 7), 0, 180 * 16);
            p.drawRect(QRectF(c.x() - 4, c.y() - 2, 8, 6));
        } else if (field == QStringLiteral("muted")) {
            QFont mute = font();
            mute.setPixelSize(11);
            mute.setWeight(QFont::DemiBold);
            p.setFont(mute);
            p.drawText(box, Qt::AlignCenter, QStringLiteral("M"));
        } else {
            p.drawEllipse(c, 5.0, 3.0);
            p.drawEllipse(c, 1.2, 1.2);
            if (!on) p.drawLine(QPointF(c.x() - 5, c.y() + 4), QPointF(c.x() + 5, c.y() - 4));
        }
        p.restore();
    };
    if (layout.audio) drawToggle(0, track.value(QStringLiteral("muted")).toBool(), QStringLiteral("muted"));
    else drawToggle(0, enabled, QStringLiteral("enabled"));
    drawToggle(1, track.value(QStringLiteral("locked")).toBool(), QStringLiteral("locked"));
    const int nameLeft = headerToggleRect(row, 1).translated(left - stickyOrigin().x(), 0).right() + 7;
    const int nameRight = preview.isNull() ? header.right() - 8 : slot.left() - 8;
    QString name = track.value(QStringLiteral("name")).toString().trimmed();
    if (name.isEmpty()) name = (layout.audio ? QStringLiteral("Audio %1") : QStringLiteral("Video %1")).arg(layout.number);
    p.setPen(enabled ? theme.trackHeaderText : theme.trackHeaderTextDisabled);
    // On one line the name has the row's height, centred with the badge, so
    // a short row clips none of it; it stays above the row's divider.
    const QRect nameLine = detailed ? QRect(nameLeft, badge.top(), std::max(1, nameRight - nameLeft), badge.height())
                                    : QRect(nameLeft, layout.top + 1, std::max(1, nameRight - nameLeft),
                                            std::max(1, 2 * (badge.center().y() - layout.top) - 1));
    drawName(p, nameLine, nameFont, name);
    if (!detailed) return;

    QFont metaFont = nameFont;
    metaFont.setPixelSize(11);
    metaFont.setWeight(QFont::Normal);
    p.setFont(metaFont);
    const QString count = clips.size() == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(clips.size());
    const QRect meta(badge.left(), blockTop + 19, std::max(1, nameRight - badge.left()), 12);
    p.setPen(theme.trackHeaderDetail);
    p.drawText(meta, Qt::AlignLeft | Qt::AlignVCenter | Qt::TextSingleLine, count);

}

QRect TimelineWidget::clipBounds(const QString &id) const {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (int row = 0; row < m_rows.size(); ++row) {
        for (const QJsonValue &value : tracks.at(m_rows.at(row).track).toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("id")).toString() == id)
                return clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(),
                                clip.value(QStringLiteral("duration")).toDouble()).toAlignedRect();
        }
    }
    return {};
}

QRect TimelineWidget::markerBounds(const QString &id) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("markers")).toArray()) {
        const QJsonObject marker = value.toObject();
        if (marker.value(QStringLiteral("id")).toString() == id)
            return markerRect(marker, stickyOrigin().y()).toAlignedRect();
    }
    return {};
}

void TimelineWidget::setProposal(const QJsonObject &timeline) {
    if (timeline == m_proposal) return;
    m_proposal = timeline;
    invalidateLanes();
    update();
}

void TimelineWidget::paintProposal(QPainter &p, int row, const QJsonObject &track) {
    const QString id = track.value(QStringLiteral("id")).toString();
    QJsonArray proposed;
    for (const QJsonValue &value : m_proposal.value(QStringLiteral("tracks")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == id) proposed = value.toObject().value(QStringLiteral("clips")).toArray();
    // What a clip is, apart from how it is labelled.
    const auto essence = [](QJsonObject clip) {
        clip.remove(QStringLiteral("name"));
        return clip;
    };
    QHash<QString, QJsonObject> current;
    for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray())
        current.insert(value.toObject().value(QStringLiteral("id")).toString(), essence(value.toObject()));
    const EditorTheme &theme = editorTheme();
    const bool locked = track.value(QStringLiteral("locked")).toBool();
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    for (const QJsonValue &value : proposed) {
        const QJsonObject clip = value.toObject();
        const QString clipId = clip.value(QStringLiteral("id")).toString();
        const auto existing = current.constFind(clipId);
        const bool added = existing == current.cend();
        if (!added && existing.value() == essence(clip)) {
            current.remove(clipId);
            continue;
        }
        current.remove(clipId);
        // Clips the proposal adds, and clips it changes. A change marks the
        // body under the clip's name bar, clear of the 3px selection ring by
        // a pixel or two, so the name and the ring both stay readable.
        const QRectF extent = clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(),
                                       clip.value(QStringLiteral("duration")).toDouble());
        const bool body = !added && clipHeaderHeight(extent) < extent.height();
        QRectF rect = added ? extent.adjusted(1, 1, -1, -1)
                      : body ? extent.adjusted(5, clipHeaderHeight(extent) + 2, -5, -5)
                             : extent.adjusted(5, 5, -5, -5);
        // A clip too small for that inset is marked along its edge.
        if (rect.isEmpty()) rect = extent.adjusted(1, 1, -1, -1);
        // A clip it adds is named on a band of bare lane under its wash, so
        // the name reads the same over a clip already there as over none.
        // A locked lane's hatch goes back over the band, so the lock still
        // shows under the name.
        const bool named = added && rect.width() > 22.0;
        if (named) {
            const QRectF band(rect.left(), rect.top(), rect.width(), std::min(rect.height(), clipHeaderHeight(extent) - 1));
            p.fillRect(band, theme.trackLane);
            if (locked) p.fillRect(band, QBrush(theme.trackLaneLocked, Qt::BDiagPattern));
        }
        p.fillRect(rect, added ? theme.proposalAddFill : theme.proposalChangeFill);
        // The dashes in two tones: purple over a solid rule, so they read on
        // any picture, purple ones too.
        p.setBrush(Qt::NoBrush);
        p.setPen(QPen(theme.proposalUnderlay, 3.0));
        p.drawRect(rect);
        p.setPen(QPen(added ? theme.proposalAdd : theme.proposalChange, 1.5, Qt::DashLine));
        p.drawRect(rect);
        // A clip it adds is named, in the agent's purple, as clips are.
        if (named) {
            QFont nameFont = font();
            nameFont.setPixelSize(11);
            nameFont.setWeight(QFont::DemiBold);
            p.setFont(nameFont);
            p.setPen(theme.assistantText);
            const QRectF band(rect.left() + 6, rect.top() + 1, rect.width() - 12, clipHeaderHeight(extent) - 2);
            p.drawText(band, Qt::AlignLeft | Qt::AlignVCenter | Qt::TextSingleLine,
                       QFontMetrics(nameFont).elidedText(clipName(clip), Qt::ElideRight, std::max(1, int(band.width()))));
        }
    }
    // Clips the proposal leaves out.
    for (auto it = current.cbegin(); it != current.cend(); ++it) {
        const QRectF rect = clipRect(row, it.value().value(QStringLiteral("timelineStart")).toDouble(),
                                     it.value().value(QStringLiteral("duration")).toDouble());
        // Under the name bar only, so the clip's name stays readable.
        const QRectF body = rect.adjusted(0, clipHeaderHeight(rect), 0, 0);
        if (body.height() > 0.0) p.fillRect(body, QBrush(theme.proposalRemoveHatch, Qt::BDiagPattern));
        // Square at every corner, over the clip's own edge.
        frameInside(p, rect, 2, theme.proposalRemove);
    }
    p.restore();
}

void TimelineWidget::setTool(TimelineTool tool) {
    m_tool = tool;
    clearHoverClip();
    setCursor(Qt::ArrowCursor);
}

int TimelineWidget::headerControlSize(int rowHeight) {
    // 16px from a 21px row; down to 12px, which a Mini row centres with a
    // pixel or two to spare above its divider.
    return std::clamp(rowHeight - 5, 12, 16);
}

QFont TimelineWidget::badgeFont() const {
    QFont badge = font();
    badge.setPixelSize(11);
    badge.setWeight(QFont::DemiBold);
    return badge;
}

int TimelineWidget::headerControlTop(int row) const {
    const TrackRow &layout = m_rows.at(row);
    const int height = layout.height - 1;
    return layout.height >= kDetailedTrackHeight ? layout.top + (height - 31) / 2
                                                 : layout.top + (height - headerControlSize(layout.height)) / 2;
}

QRect TimelineWidget::headerToggleRect(int row, int index) const {
    const int size = headerControlSize(m_rows.at(row).height);
    return QRect(stickyOrigin().x() + 14 + m_badgeWidth + index * 20, headerControlTop(row), size, size);
}

bool TimelineWidget::toggleAt(const QPoint &point, int *row, QString *field) const {
    if (!inHeader(point) || inRuler(point)) return false;
    const int at = rowAt(point.y());
    if (at < 0 || at >= m_rows.size()) return false;
    for (int index = 0; index < 2; ++index) {
        if (!headerToggleRect(at, index).contains(point)) continue;
        *row = at;
        *field = index == 1 ? QStringLiteral("locked") : m_rows.at(at).audio ? QStringLiteral("muted") : QStringLiteral("enabled");
        return true;
    }
    return false;
}

void TimelineWidget::setToolCursor(const ClipHit &hit, int edge) {
    if (hit.id.isEmpty()) setCursor(Qt::ArrowCursor);
    else if (m_tool == TimelineTool::Razor) setCursor(Qt::CrossCursor);
    else if (edge != 0 && trimsEdges()) setCursor(m_tool == TimelineTool::Roll ? Qt::SplitHCursor : Qt::SizeHorCursor);
    else if (dragsBodies()) setCursor(m_tool == TimelineTool::Selection ? Qt::OpenHandCursor : Qt::SizeHorCursor);
    else setCursor(Qt::ArrowCursor);
}

double TimelineWidget::snapped(double time, double length, double *snapPoint) const {
    if (snapPoint) *snapPoint = -1.0;
    if (!m_snapping) return time;
    // Edit points, gathered when the drag began: the start, the playhead,
    // every other clip's edges, and the markers. They are sorted, so each
    // edge looks only at its two neighbours.
    const double reach = 8.0 / pixelsPerSecond();
    double best = time;
    double distance = reach;
    // A moved clip snaps by either end; an edge by itself.
    const QVector<double> edges = length > 0.0 ? QVector<double>{0.0, length} : QVector<double>{0.0};
    for (const double edge : edges) {
        const double at = time + edge;
        const auto next = std::lower_bound(m_snapPoints.cbegin(), m_snapPoints.cend(), at);
        for (auto it = next == m_snapPoints.cbegin() ? next : next - 1; it != m_snapPoints.cend() && it <= next; ++it) {
            const double gap = std::abs(at - *it);
            if (gap < distance) {
                distance = gap;
                best = *it - edge;
                if (snapPoint) *snapPoint = *it;
            }
        }
    }
    return std::max(0.0, std::round(best * m_fps) / m_fps);
}

void TimelineWidget::paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track, bool audio) {
    double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    double duration = clip.value(QStringLiteral("duration")).toDouble();
    const QString id = clip.value(QStringLiteral("id")).toString();
    const bool dragged = m_dragging && id == m_dragClipId;
    if (dragged) {
        if (m_dragLeading) { duration = std::max(1.0 / m_fps, m_dragStart + m_dragDuration - m_previewTime); start = m_previewTime; }
        else if (m_dragTrailing) duration = std::max(1.0 / m_fps, m_previewTime - m_dragStart);
        else if (m_tool != TimelineTool::Slip) start = m_previewTime;
    }
    const QRectF rect = clipRect(row, start, duration);
    if (!rect.intersects(visible)) return;
    const bool selected = isSelected(id);
    const bool hovered = id == m_hoverClipId;
    const EditorTheme &theme = editorTheme();
    const ClipStyle &style = theme.clip(clipKind(track, clip));
    // A hovered or selected clip's body leans towards its name bar's colour;
    // the name bar itself stays put, so its label keeps its contrast.
    const QColor fill = mixColor(style.fill, style.header, selected ? 0.18 : hovered ? 0.1 : 0.0);
    // Short rows show only the name bar, as a mini track does.
    const qreal bandHeight = clipHeaderHeight(rect);
    const QRectF band(rect.left(), rect.top(), rect.width(), bandHeight);
    const QRectF body(rect.left(), band.bottom(), rect.width(), rect.height() - bandHeight);

    p.save();
    p.setClipRect(rect, Qt::IntersectClip);
    QLinearGradient bandFill(0, band.top(), 0, band.bottom());
    bandFill.setColorAt(0, style.header.lighter(108));
    bandFill.setColorAt(1, style.header);
    p.fillRect(band, bandFill);

    const QString sourceId = clip.value(QStringLiteral("sourceId")).toString();
    if (body.height() > 0.0) {
        QLinearGradient bodyFill(0, body.top(), 0, body.bottom());
        // lighter() would bleach a light fill to white.
        bodyFill.setColorAt(0, theme.dark ? fill.lighter(125) : mixColor(fill, Qt::white, 0.3));
        bodyFill.setColorAt(1, fill);
        p.fillRect(body, bodyFill);
    }
    if (body.height() > 0.0 && audio) {
        p.fillRect(QRectF(body.left(), std::floor(body.center().y()), body.width(), 1), theme.waveCenter);
        if (m_artwork && m_artwork->waves.contains(sourceId)) {
            const QImage wave = tintedWave(sourceId, style.wave);
            double sourceDuration = 1;
            for (const auto &source : m_project.value(QStringLiteral("sources")).toArray())
                if (source.toObject().value(QStringLiteral("id")).toString() == sourceId)
                    sourceDuration = std::max(0.01, source.toObject().value(QStringLiteral("duration")).toDouble(1));
            const double in = clip.value(QStringLiteral("in")).toDouble();
            const double out = clip.value(QStringLiteral("out")).toDouble(sourceDuration);
            p.drawImage(body.adjusted(0, 1, 0, -1), wave,
                        QRectF(in / sourceDuration * wave.width(), 0,
                               (out - in) / sourceDuration * wave.width(), wave.height()));
        }
    } else if (body.height() >= kFilmstripFrom && m_artwork && m_artwork->frames.contains(sourceId)) {
        // A filmstrip of tiles scaled once per size, not on every repaint.
        const double tileHeight = std::max(1.0, body.height());
        const double tileWidth = std::max(1.0, std::round(tileHeight * 16.0 / 9.0));
        const QImage tile = filmstripTile(sourceId, QSize(int(tileWidth), int(tileHeight)), p.device()->devicePixelRatioF());
        const double firstTile = body.left() + std::max(0.0, std::floor((visible.left() - body.left()) / tileWidth)) * tileWidth;
        const double stop = std::min(body.right(), double(visible.right()) + 1.0);
        for (double x = firstTile; x < stop; x += tileWidth) {
            p.drawImage(QPointF(x, body.top()), tile);
            p.fillRect(QRectF(x + tileWidth - 1, body.top(), 1, tileHeight), theme.filmstripSeam);
        }
    }

    if (rect.width() > 22.0) {
        // A selected clip's name and badge sit below its ring (and above it
        // too, on a row that is all name bar).
        const qreal ring = selected ? kSelectionRing : 0.0;
        const QRectF labelBand = band.adjusted(0, ring, 0, body.height() > 0.0 ? 0.0 : -ring);
        qreal textRight = rect.right() - 5;
        const bool effected = !clip.value(QStringLiteral("effects")).toArray().isEmpty() ||
                              !clip.value(QStringLiteral("props")).toObject().isEmpty();
        if (effected && rect.width() > 64.0) {
            QFont fxFont = font();
            fxFont.setPixelSize(11);
            fxFont.setWeight(QFont::DemiBold);
            const QRectF fx(textRight - 20, labelBand.center().y() - 6.0, 20, 12);
            p.fillRect(fx, theme.clipFxBadge);
            p.setFont(fxFont);
            p.setPen(theme.clipFxBadgeText);
            p.drawText(fx, Qt::AlignCenter, QStringLiteral("FX"));
            textRight = fx.left() - 4;
        }
        QFont nameFont = font();
        nameFont.setPixelSize(11);
        nameFont.setWeight(QFont::DemiBold);
        p.setFont(nameFont);
        QString label = clipName(clip);
        const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
        if (std::abs(speed - 1.0) > 0.0005) label += QStringLiteral("  [%1%]").arg(std::lround(speed * 100.0));
        const qreal textLeft = rect.left() + 6;
        p.setPen(style.label);
        // Centred in the band even where a short row's band (inside a
        // selected clip's ring) is shorter than the text: the clip clips it,
        // and the ring, drawn after, covers no more than a pixel of it.
        drawName(p, QRectF(textLeft, labelBand.top(), std::max(1.0, textRight - textLeft), labelBand.height()), nameFont, label);
    }
    p.restore();

    p.fillRect(QRectF(rect.left() + 1, rect.top() + 1, rect.width() - 2, 1), theme.clipHighlight);
    // The lane draws the selected clip's ring last, over any proposal.
    if (selected) paintSelectionRing(p, rect);
    else frameInside(p, rect, 1, style.border);

    // Trim bracket on the edge under the pointer, or the edge being trimmed.
    int edge = 0;
    if (dragged) edge = m_dragLeading ? -1 : m_dragTrailing ? 1 : 0;
    else if (hovered && !m_dragging) edge = m_hoverEdge;
    if (edge != 0) {
        const QColor bracket = theme.clipTrim;
        const qreal x = edge < 0 ? rect.left() : rect.right() - 3;
        p.fillRect(QRectF(x, rect.top(), 3, rect.height()), bracket);
        const qreal serif = edge < 0 ? rect.left() : rect.right() - 8;
        p.fillRect(QRectF(serif, rect.top(), 8, 2), bracket);
        p.fillRect(QRectF(serif, rect.bottom() - 2, 8, 2), bracket);
    }
}

void TimelineWidget::paintDraggedClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip,
                                      const QJsonObject &track, bool audio) {
    const EditorTheme &theme = editorTheme();
    double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    double duration = clip.value(QStringLiteral("duration")).toDouble();
    if (m_dragLeading) { duration = std::max(1.0 / m_fps, m_dragStart + m_dragDuration - m_previewTime); start = m_previewTime; }
    else if (m_dragTrailing) duration = std::max(1.0 / m_fps, m_previewTime - m_dragStart);
    else if (m_tool != TimelineTool::Slip) start = m_previewTime;
    const QRectF rect = clipRect(row, start, duration);
    // A shadow under it, as a lifted clip. It is opaque, so the names of
    // the clips it covers do not ghost through its own.
    p.fillRect(rect.translated(0, 2), theme.dragShadow);
    paintClip(p, visible, row, clip, track, audio);
    // Where it will overwrite its neighbours, a dashed edge inside its
    // selection ring: white dashes on a dark line, which read on any
    // picture in either appearance.
    const QString id = clip.value(QStringLiteral("id")).toString();
    const qreal ring = isSelected(id) ? kSelectionRing : 0.0;
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.setBrush(Qt::NoBrush);
    for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray()) {
        const QJsonObject other = value.toObject();
        if (other.value(QStringLiteral("id")).toString() == id) continue;
        const QRectF overlap = rect.intersected(clipRect(row, other.value(QStringLiteral("timelineStart")).toDouble(),
                                                         other.value(QStringLiteral("duration")).toDouble()));
        if (overlap.width() < 1.0 + 2 * ring) continue;
        // On the first pixels inside the ring (a 1px pen on the half pixel).
        const QRectF edge = overlap.adjusted(ring + 0.5, ring + 0.5, -ring - 0.5, -ring - 0.5);
        p.setPen(QPen(QColor(0, 0, 0, 150), 1));
        p.drawRect(edge);
        p.setPen(QPen(QColor(255, 255, 255), 1, Qt::DashLine));
        p.drawRect(edge);
    }
    p.restore();
}

void TimelineWidget::paintRuler(QPainter &p, const QRect &dirty, int top) {
    const QRect band(dirty.left(), top, dirty.width(), kRulerHeight);
    if (!band.intersects(dirty)) return;
    const EditorTheme &theme = editorTheme();
    p.setRenderHint(QPainter::Antialiasing, false);
    QLinearGradient fill(0, top, 0, top + kRulerHeight);
    fill.setColorAt(0, theme.ruler);
    fill.setColorAt(1, theme.rulerBottom);
    p.fillRect(band.adjusted(0, 0, 0, -1), fill);
    p.fillRect(QRect(band.left(), top + kRulerHeight - 1, band.width(), 1), theme.divider);

    QFont font(QStringLiteral("Menlo"));
    font.setPixelSize(11);
    font.setStyleHint(QFont::Monospace);
    p.setFont(font);
    const double pps = pixelsPerSecond();
    const RulerScale scale = rulerScale(pps, m_fps);
    const double major = std::max(scale.majorSeconds, 1e-6);
    const double minor = major / scale.divisions;
    const double startTime = (dirty.left() - kTimelineOrigin) / pps;
    const double endTime = (dirty.right() - kTimelineOrigin) / pps;
    int first = std::max(0, static_cast<int>(std::floor(startTime / major)) - 1);
    const int limit = static_cast<int>(std::ceil(std::max(m_duration, 0.0) / major)) + 1;
    const int last = std::min(limit, static_cast<int>(std::ceil(endTime / major)) + 1);
    for (int index = first; index <= last; ++index) {
        const double time = index * major;
        const int x = static_cast<int>(std::floor(timeToX(time)));
        p.fillRect(QRect(x, top + 3, 1, kRulerHeight - 4), theme.rulerTick);
        if (time <= m_duration + 0.0001) {
            // Clear of the playhead's head when it stands on the tick, as
            // it does at 00:00:00 when a project opens.
            p.setPen(theme.rulerText);
            p.drawText(QRectF(x + 7, top + 1, std::max(12.0, major * pps - 11.0), 12),
                       Qt::AlignLeft | Qt::AlignVCenter,
                       timelineRulerLabel(std::min(time, m_duration), m_fps, m_duration));
        }
        for (int step = 1; step < scale.divisions; ++step) {
            const double tick = time + minor * step;
            if (tick > m_duration) break;
            const bool half = scale.divisions % 2 == 0 && step * 2 == scale.divisions;
            const int length = half ? 8 : 4;
            p.fillRect(QRect(static_cast<int>(std::floor(timeToX(tick))), top + kRulerHeight - 1 - length, 1, length),
                       half ? theme.rulerTickHalf : theme.rulerTickMinor);
        }
    }
}

QRectF TimelineWidget::markerRect(const QJsonObject &marker, int top) const {
    // The tag at the top left; a range runs along the bottom to its end.
    const double at = marker.value(QStringLiteral("time")).toDouble();
    const qreal x = std::floor(timeToX(at)) + 0.5;
    const qreal end = std::floor(timeToX(at + marker.value(QStringLiteral("duration")).toDouble())) + 0.5;
    return QRectF(x - 4.5, top + kRulerHeight - 12.0, std::max<qreal>(end - x, 0.0) + 9.0, 11.0);
}

void TimelineWidget::paintMarkers(QPainter &p, const QRect &dirty, int top) {
    const QJsonArray markers = m_project.value(QStringLiteral("markers")).toArray();
    if (markers.isEmpty()) return;
    const EditorTheme &theme = editorTheme();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    for (const QJsonValue &value : markers) {
        const QJsonObject marker = value.toObject();
        const QRectF rect = markerRect(marker, top);
        if (!rect.toAlignedRect().intersects(dirty)) continue;
        // A marker without a colour is green, as in other edit suites.
        const QColor color = theme.marker(marker.value(QStringLiteral("color")).toString());
        const qreal x = rect.left() + 4.5;
        // A range is a solid bar along the ruler's foot, never a wash that
        // could read as the in-to-out range.
        if (rect.width() > 9.0) {
            p.setPen(Qt::NoPen);
            p.setBrush(color);
            p.drawRect(QRectF(x, rect.bottom() - 3.0, rect.width() - 9.0, 3.0));
        }
        // A small tag pointing down at the marker's frame, outlined so it
        // stays apart from the playhead's larger head.
        QPainterPath tag;
        tag.moveTo(x - 4.5, rect.top());
        tag.lineTo(x + 4.5, rect.top());
        tag.lineTo(x + 4.5, rect.top() + 4.5);
        tag.lineTo(x, rect.top() + 7.0);
        tag.lineTo(x - 4.5, rect.top() + 4.5);
        tag.closeSubpath();
        // A dark edge in both appearances: a white one vanishes into the light ruler.
        p.setPen(QPen(theme.dark ? QColor(0, 0, 0, 153) : QColor(0, 0, 0, 90), 1.0));
        p.setBrush(color);
        p.drawPath(tag);
    }
    p.setRenderHint(QPainter::Antialiasing, false);
}

void TimelineWidget::paintOverlay(QPainter &p, const QRect &dirty) {
    const EditorTheme &theme = editorTheme();
    const QPoint origin = stickyOrigin();
    const bool skimming = m_hasSkimmer && !m_dragging && !m_scrubbing;
    const QColor playColor = skimming ? theme.playheadIdle : theme.playhead;
    const int laneTop = origin.y() + kRulerHeight;
    const int playX = static_cast<int>(std::floor(timeToX(m_playhead)));
    p.setRenderHint(QPainter::Antialiasing, false);
    // A halo keeps the line readable over pictures and clips, skimmed or not.
    p.fillRect(QRect(playX - 1, laneTop, 3, height() - laneTop), theme.playheadHalo);
    p.fillRect(QRect(playX, laneTop, 1, height() - laneTop), playColor);
    // The edit point a drag has snapped to.
    if (m_dragging && m_dragMoved && m_snapTime >= 0.0)
        p.fillRect(QRect(static_cast<int>(std::floor(timeToX(m_snapTime))), laneTop, 1, height() - laneTop), theme.snapLine);

    paintRuler(p, dirty, origin.y());
    if (skimming) {
        const int skimX = static_cast<int>(std::floor(timeToX(m_skimmerTime)));
        p.fillRect(QRect(skimX, origin.y(), 1, height() - origin.y()), theme.skimmer);
    }
    p.fillRect(QRect(playX, laneTop - 3, 1, 3), playColor);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    p.setBrush(playColor);
    const qreal cx = playX + 0.5;
    const qreal top = origin.y();
    QPainterPath head;
    head.moveTo(cx - 5.5, top + 3.0);
    head.lineTo(cx + 5.5, top + 3.0);
    head.lineTo(cx + 5.5, top + kRulerHeight - 9.0);
    head.lineTo(cx, top + kRulerHeight - 3.0);
    head.lineTo(cx - 5.5, top + kRulerHeight - 9.0);
    head.closeSubpath();
    p.drawPath(head);
    p.setRenderHint(QPainter::Antialiasing, false);
    // Over the playhead tab, so a marker added at the playhead shows at once.
    paintMarkers(p, dirty, origin.y());

    paintHeaders(p, dirty);
    if (m_addLaneVisible) {
        p.fillRect(addLaneHitRect().adjusted(0, 0, -1, -1), theme.addLane);
        const QRect button = addLaneButtonRect();
        p.fillRect(button, theme.addLaneButton);
        frameInside(p, button, 1, theme.addLaneButtonBorder);
        p.setPen(theme.addLaneText);
        QFont addFont = font();
        addFont.setPixelSize(11);
        addFont.setWeight(QFont::Medium);
        p.setFont(addFont);
        p.drawText(button, Qt::AlignCenter, QStringLiteral("+  Track"));
    }
    // The playhead's time in the accent, deepened where needed to read on the corner.
    paintCornerTime(p, origin, skimming ? m_skimmerTime : m_playhead, skimming ? theme.skimmerText : theme.accentText);
    if (m_dragging && m_dragMoved) paintDragReadout(p);
}

void TimelineWidget::paintDragReadout(QPainter &p) {
    const double delta = m_previewTime - dragOrigin();
    const QString text = QStringLiteral("%1  %2%3")
        .arg(timelineTimecode(m_previewTime, m_fps), delta < 0 ? QStringLiteral("-") : QStringLiteral("+"),
             timelineTimecode(std::abs(delta), m_fps));
    QFont font(QStringLiteral("Menlo"));
    font.setStyleHint(QFont::Monospace);
    font.setPixelSize(11);
    const QFontMetrics metrics(font);
    const qreal x = timeToX(m_previewTime);
    QRectF box(0, 0, metrics.horizontalAdvance(text) + 14, 18);
    box.moveTop(std::max(stickyOrigin().y() + kRulerHeight + 2, m_rows.value(m_dragRow).top - 21));
    box.moveLeft(x + 6);
    if (box.right() > width() - 4) box.moveRight(x - 6);
    const EditorTheme &theme = editorTheme();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(box, theme.readout);
    frameInside(p, box, 1, theme.readoutBorder);
    p.setFont(font);
    p.setPen(theme.readoutText);
    p.drawText(box, Qt::AlignCenter, text);
}

void TimelineWidget::paintCornerTime(QPainter &p, const QPoint &origin, double time, const QColor &color) const {
    const QRect corner(origin.x(), origin.y(), kTimelineLabelWidth - 1, kRulerHeight - 1);
    const EditorTheme &theme = editorTheme();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(corner, theme.rulerCorner);
    p.fillRect(QRect(corner.right() + 1, origin.y(), 1, kRulerHeight), theme.divider);
    p.fillRect(QRect(origin.x(), corner.bottom() + 1, kTimelineLabelWidth, 1), theme.divider);
    QFont font(QStringLiteral("Menlo"));
    font.setPixelSize(13);
    font.setStyleHint(QFont::Monospace);
    font.setWeight(QFont::DemiBold);
    p.setFont(font);
    p.setPen(color);
    p.drawText(corner.adjusted(10, 0, -6, 0), Qt::AlignLeft | Qt::AlignVCenter, timelineTimecode(time, m_fps));
}

void TimelineWidget::repaintAt(qreal x) {
    update(pointerBand(x));
    const QPoint origin = stickyOrigin();
    update(QRect(origin.x(), origin.y(), kTimelineLabelWidth, kRulerHeight));
}

bool TimelineWidget::laneCacheFits(const QRect &view) const {
    return !m_laneCache.isNull() && m_laneCacheRect == view &&
           qFuzzyCompare(m_laneCache.devicePixelRatio(), std::max<qreal>(1, devicePixelRatioF()));
}

void TimelineWidget::ensureLaneCache(const QRect &view) {
    m_laneCache = QPixmap();
    m_laneCacheRect = QRect();
    const qreal dpr = std::max<qreal>(1, devicePixelRatioF());
    // A timeline shown whole (not in a scroll area) can be far larger than
    // a screen; it paints directly.
    if (view.isEmpty() || double(view.width()) * view.height() * dpr * dpr > 12000000.0) return;
    QPixmap cache(std::max(1, int(std::ceil(view.width() * dpr))), std::max(1, int(std::ceil(view.height() * dpr))));
    if (cache.isNull()) return;
    cache.setDevicePixelRatio(dpr);
    cache.fill(editorTheme().timelineBackground);
    QPainter painter(&cache);
    if (!painter.isActive()) return;
    painter.translate(-view.topLeft());
    paintLanes(painter, view);
    painter.end();
    m_laneCache = cache;
    m_laneCacheRect = view;
}

QImage TimelineWidget::filmstripTile(const QString &sourceId, const QSize &size, qreal dpr) {
    const QString key = QStringLiteral("%1/%2x%3@%4").arg(sourceId).arg(size.width()).arg(size.height()).arg(dpr);
    const auto cached = m_filmstripTiles.constFind(key);
    if (cached != m_filmstripTiles.cend()) return cached.value();
    // Pixel for pixel on the screen it is drawn on.
    QImage tile = m_artwork->frames.value(sourceId).scaled(size * dpr, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
    tile.setDevicePixelRatio(dpr);
    m_filmstripTiles.insert(key, tile);
    return tile;
}

qreal TimelineWidget::timeToX(double time) const {
    return kTimelineOrigin + time * pixelsPerSecond();
}

QRectF TimelineWidget::clipRect(int row, double start, double duration) const {
    const TrackRow track = m_rows.value(row);
    const double left = std::round(timeToX(start));
    const double right = std::max(left + 4.0, std::round(timeToX(start + duration)));
    const int inset = clipInset(track.height);
    return QRectF(left, track.top + inset, right - left, track.height - 1 - inset * 2);
}

double TimelineWidget::dragOrigin() const {
    return m_dragTrailing && !m_dragLeading ? m_dragStart + m_dragDuration : m_dragStart;
}

QRect TimelineWidget::headerPreviewRect(int row, int left) const {
    const TrackRow track = m_rows.value(row);
    return QRect(left + kTimelineLabelWidth - 9 - kHeaderPreviewWidth,
                 track.top + (track.height - 1 - kHeaderPreviewHeight) / 2,
                 kHeaderPreviewWidth, kHeaderPreviewHeight);
}

QImage TimelineWidget::tintedWave(const QString &sourceId, const QColor &color) {
    const QString key = sourceId + QLatin1Char('/') + color.name(QColor::HexArgb);
    const auto cached = m_waveTints.constFind(key);
    if (cached != m_waveTints.constEnd()) return cached.value();
    QImage tinted = m_artwork->waves.value(sourceId).convertToFormat(QImage::Format_ARGB32_Premultiplied);
    QPainter painter(&tinted);
    painter.setCompositionMode(QPainter::CompositionMode_SourceIn);
    painter.fillRect(tinted.rect(), color);
    painter.end();
    m_waveTints.insert(key, tinted);
    return tinted;
}

QRect TimelineWidget::pointerBand(qreal x) const {
    const int ix = static_cast<int>(std::lround(x));
    return QRect(ix - 120, 0, 240, std::max(1, height()));
}

void TimelineWidget::updateContentSize() {
    const double contentWidth = kTimelineOrigin + m_duration * pixelsPerSecond() + kTimelineTail;
    const int contentHeight = std::max(tracksBottom() + kAddLaneHeight, kRulerHeight + 3 * kAddLaneHeight) + 8;
    setMinimumSize(static_cast<int>(std::min(contentWidth, 16777215.0)), contentHeight);
    updateGeometry();
}

double TimelineWidget::pixelsPerSecond() const {
    if (m_fitWidth <= 0) return 72.0 * m_zoom;
    const double fitted = (m_fitWidth - kTimelineOrigin - kTimelineTail) / std::max(m_duration, 0.001);
    return std::max(fitted, kMinPixelsPerSecond);
}

double TimelineWidget::timeAt(qreal x) const {
    return std::clamp((x - kTimelineOrigin) / pixelsPerSecond(), 0.0, m_duration);
}

double TimelineWidget::scrubTime(qreal x) const {
    return timeAt(std::max(x, qreal(stickyOrigin().x() + kTimelineLabelWidth)));
}

void TimelineWidget::beginScrub(const QPointF &point) {
    const double previous = m_playhead;
    m_scrubbing = true;
    grabMouse();
    m_feedbackPoint = point;
    m_playhead = scrubTime(point.x());
    repaintAt(timeToX(previous));
    repaintAt(timeToX(m_playhead));
    emit seekRequested(m_playhead);
}

void TimelineWidget::updateSkimmer(const QPointF &point) {
    if (inHeader(point.toPoint())) {
        if (m_hasSkimmer) {
            const double previous = m_skimmerTime;
            const qreal previousX = m_feedbackPoint.x();
            m_hasSkimmer = false;
            m_hoverFrame = -1.0;
            emit hoverSeekEnded();
            repaintAt(timeToX(previous));
            repaintAt(timeToX(m_playhead));
            repaintAt(previousX);
        }
        return;
    }
    const qreal previousX = m_feedbackPoint.x();
    const double previous = m_skimmerTime;
    const bool had = m_hasSkimmer;
    m_feedbackPoint = point;
    const double exact = std::clamp(timeAt(point.x()), 0.0, m_duration);
    const double frameTime = std::clamp(std::round(exact * m_fps) / m_fps, 0.0, m_duration);
    const bool frameChanged = !had || std::abs(m_hoverFrame - frameTime) >= (0.5 / m_fps);
    m_skimmerTime = exact;
    m_hasSkimmer = true;
    if (had) repaintAt(timeToX(previous));
    repaintAt(timeToX(exact));
    repaintAt(timeToX(m_playhead));
    repaintAt(previousX);
    if (frameChanged) {
        m_hoverFrame = frameTime;
        emit hoverSeekRequested(frameTime);
    }
}

QRect TimelineWidget::addLaneButtonRect() const {
    return QRect(stickyOrigin().x() + (kTimelineLabelWidth - 76) / 2,
                 tracksBottom() + (kAddLaneHeight - 20) / 2, 76, 20);
}

QRect TimelineWidget::addLaneHitRect() const {
    return QRect(stickyOrigin().x(), tracksBottom(), kTimelineLabelWidth, kAddLaneHeight);
}

void TimelineWidget::updateAddLane(const QPoint &point) {
    const bool visible = addLaneHitRect().contains(point);
    if (m_addLaneVisible != visible) {
        m_addLaneVisible = visible;
        update(addLaneHitRect());
    }
    if (visible) setCursor(Qt::PointingHandCursor);
}

TimelineWidget::ClipHit TimelineWidget::clipAt(const QPoint &point) const {
    if (inHeader(point) || inRuler(point)) return {};
    const int row = rowAt(point.y());
    if (row < 0) return {};
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    const QJsonArray clips = tracks.at(m_rows.at(row).track).toObject().value(QStringLiteral("clips")).toArray();
    for (const QJsonValue &value : clips) {
        const QJsonObject clip = value.toObject();
        const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
        const double duration = clip.value(QStringLiteral("duration")).toDouble();
        const QRectF rect = clipRect(row, start, duration);
        if (rect.contains(point)) return {clip.value(QStringLiteral("id")).toString(), row, start, duration, rect};
    }
    return {};
}

// ---- TimelineZoomControl -------------------------------------------------

namespace {

// A − or + drawn in code (no SVG on Linux), in secondary ink.
QIcon zoomGlyph(bool plus) {
    const qreal ratio = 2.0;
    QPixmap pixmap(QSize(16, 16) * ratio);
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(editorTheme().textSecondary, 1.5, Qt::SolidLine, Qt::RoundCap));
    painter.drawLine(QPointF(4, 8), QPointF(12, 8));
    if (plus) painter.drawLine(QPointF(8, 4), QPointF(8, 12));
    return QIcon(pixmap);
}

} // namespace

TimelineZoomControl::TimelineZoomControl(TimelineWidget *timeline, QWidget *parent) : QWidget(parent), m_timeline(timeline) {
    setObjectName(QStringLiteral("timelineZoom"));
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(0, 0, 8, 0);
    layout->setSpacing(2);
    const auto glyphButton = [this](bool plus) {
        auto *button = new QToolButton(this);
        button->setObjectName(plus ? QStringLiteral("timelineZoomIn") : QStringLiteral("timelineZoomOut"));
        button->setAutoRaise(true);
        button->setIcon(zoomGlyph(plus));
        button->setIconSize(QSize(16, 16));
        button->setFixedSize(20, 20);
        button->setToolTip(plus ? QStringLiteral("Zoom in") : QStringLiteral("Zoom out"));
        button->setAccessibleName(button->toolTip());
        button->setAutoRepeat(true);
        return button;
    };
    m_out = glyphButton(false);
    m_slider = new QSlider(Qt::Horizontal, this);
    m_slider->setObjectName(QStringLiteral("timelineZoomSlider"));
    m_slider->setRange(0, kSteps);
    m_slider->setFixedWidth(88);
    m_slider->setToolTip(QStringLiteral("Timeline zoom"));
    m_slider->setAccessibleName(QStringLiteral("Timeline zoom"));
    m_slider->setFocusPolicy(Qt::NoFocus);
    m_in = glyphButton(true);
    layout->addWidget(m_out);
    layout->addWidget(m_slider);
    layout->addWidget(m_in);
    m_slider->setValue(position(timeline->scale()));
    connect(m_slider, &QSlider::valueChanged, this, [this](int value) {
        if (position(m_timeline->scale()) != value) m_timeline->setScale(scaleAt(value));
    });
    connect(m_out, &QToolButton::clicked, this, [this] { m_slider->setValue(m_slider->value() - 10); });
    connect(m_in, &QToolButton::clicked, this, [this] { m_slider->setValue(m_slider->value() + 10); });
    connect(timeline, &TimelineWidget::scaleChanged, this, [this](double scale) {
        const QSignalBlocker blocker(m_slider);
        m_slider->setValue(position(scale));
    });
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
        m_out->setIcon(zoomGlyph(false));
        m_in->setIcon(zoomGlyph(true));
    });
}

int TimelineZoomControl::position(double pixelsPerSecond) {
    const double span = std::log(TimelineWidget::kMaxScale / TimelineWidget::kMinScale);
    const double at = std::log(std::clamp(pixelsPerSecond, TimelineWidget::kMinScale, TimelineWidget::kMaxScale) / TimelineWidget::kMinScale);
    return int(std::lround(kSteps * at / span));
}

double TimelineZoomControl::scaleAt(int position) {
    const double span = std::log(TimelineWidget::kMaxScale / TimelineWidget::kMinScale);
    return TimelineWidget::kMinScale * std::exp(span * std::clamp(position, 0, kSteps) / double(kSteps));
}

} // namespace kerf
