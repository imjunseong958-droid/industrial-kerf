#include "timeline_widget.h"
#include "media_artwork.h"

#include <QAction>
#include <QApplication>
#include <QFontMetrics>
#include <QJsonArray>
#include <QLinearGradient>
#include <QMenu>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace cut {

TimelineWidget::TimelineWidget(QWidget *parent) : QWidget(parent) {
    setFocusPolicy(Qt::StrongFocus);
    setAttribute(Qt::WA_OpaquePaintEvent);
    setSizePolicy(QSizePolicy::Minimum, QSizePolicy::MinimumExpanding);
    updateContentSize();
    setMouseTracking(true);
}

void TimelineWidget::setProject(const QJsonObject &project, double duration, double fps) {
    m_project = project;
    m_duration = std::max(duration, 1.0);
    m_fps = fps > 0.0 ? fps : 24.0;
    layoutRows();
    invalidateLanes();
    updateContentSize();
    update();
}

void TimelineWidget::setSelectedClip(const QString &id) {
    if (m_selectedClip == id) return;
    m_selectedClip = id;
    invalidateLanes();
    update();
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
    m_zoom = std::clamp(zoom, 0.35, 3.5);
    m_fitWidth = 0;
    invalidateLanes();
    updateContentSize();
    update();
}

void TimelineWidget::fitToWidth(int width) {
    m_fitWidth = std::max(kTimelineLabelWidth + 32, width);
    invalidateLanes();
    updateContentSize();
    update();
}

void TimelineWidget::setArtwork(MediaArtwork *artwork) {
    m_artwork = artwork;
    connect(artwork, &MediaArtwork::changed, this, [this] {
        m_waveTints.clear();
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
    const int available = viewportRect().height() - kRulerHeight - kTrackHeight - 8;
    const int each = std::clamp(available / static_cast<int>(m_rows.size()), kMinTrackHeight, kMaxTrackHeight);
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (const TrackRow &row : m_rows)
        m_trackHeights.insert(trackKey(tracks.at(row.track).toObject(), row.track), each);
    relayout();
}

void TimelineWidget::resetTrackHeights() {
    m_trackHeights.clear();
    relayout();
}

QString TimelineWidget::trackRowLabel(int row) const {
    const TrackRow track = m_rows.value(row);
    return (track.audio ? QStringLiteral("A") : QStringLiteral("V")) + QString::number(track.number);
}

QRect TimelineWidget::trackHeaderRect(int row) const {
    const TrackRow track = m_rows.value(row);
    return QRect(stickyOrigin().x(), track.top, kTimelineLabelWidth, track.height);
}

void TimelineWidget::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    const QRect dirty = event->region().boundingRect();
    // Lanes come from a cache; the ruler, track headers, and playhead are
    // painted on top at the scroll position so they stay pinned.
    if (lanesAreCacheable()) {
        const bool rebuilt = m_laneCache.isNull();
        ensureLaneCache();
        if (!m_laneCache.isNull()) {
            blitLaneCache(p, m_laneCache, event->region());
            p.setClipRegion(event->region());
            paintOverlay(p, dirty);
            // A resize can expose only the new edge. The rest would keep the
            // previous scale until the pointer moved across it.
            if (rebuilt && event->region() != QRegion(rect())) update();
            return;
        }
    }
    for (const QRect &visible : event->region())
        paintLanes(p, visible);
    p.setClipRegion(event->region());
    paintOverlay(p, dirty);
}

void TimelineWidget::resizeEvent(QResizeEvent *event) {
    invalidateLanes();
    QWidget::resizeEvent(event);
    update();
}

void TimelineWidget::moveEvent(QMoveEvent *event) {
    QWidget::moveEvent(event);
    // A scroll area scrolls by moving this widget. Repaint so the pinned
    // ruler and headers are drawn at the new offset.
    if (!isWindow()) update();
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
        beginScrub(event->position());
        return;
    }
    if (m_tool == TimelineTool::Razor) {
        emit razorRequested(hit.id, static_cast<double>(std::llround(timeAt(event->position().x()) * m_fps)));
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
    m_dragClipId = hit.id;
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
        setTrackHeight(m_resizeRow, m_resizeStartHeight + event->position().toPoint().y() - m_resizeAnchor);
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
            m_hoverClipId = hit.id;
            m_hoverEdge = edge;
            invalidateLanes();
            update();
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
                            body && m_tool != TimelineTool::Slip ? m_dragDuration : 0.0);
    setCursor((m_dragLeading || m_dragTrailing) ? Qt::SizeHorCursor : Qt::ClosedHandCursor);
    invalidateLanes();
    update();
}

void TimelineWidget::mouseReleaseEvent(QMouseEvent *event) {
    if (m_resizeRow >= 0) {
        m_resizeRow = -1;
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
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    const int row = rowAt(pos.y());
    QMenu menu(this);
    // Keys, not row indexes: a snapshot can relayout rows while the menu is open.
    const auto addPresets = [&](QMenu *target, const QStringList &keys, int current) {
        for (const auto &preset : kTrackHeightPresets) {
            QAction *action = target->addAction(QString::fromLatin1(preset.name));
            action->setCheckable(true);
            action->setChecked(current == preset.height);
            connect(action, &QAction::triggered, this, [this, keys, height = preset.height] {
                for (const QString &key : keys) m_trackHeights.insert(key, height);
                relayout();
            });
        }
    };
    QStringList allKeys;
    for (const TrackRow &track : m_rows) allKeys << trackKey(tracks.at(track.track).toObject(), track.track);
    if (row >= 0) {
        const TrackRow track = m_rows.at(row);
        addPresets(menu.addMenu(QStringLiteral("Track height")),
                   {trackKey(tracks.at(track.track).toObject(), track.track)}, track.height);
    }
    if (!m_rows.isEmpty()) {
        addPresets(menu.addMenu(QStringLiteral("All tracks")), allKeys, -1);
        menu.addSeparator();
        menu.addAction(QStringLiteral("Fit tracks to panel"), this, &TimelineWidget::fitTracksToView);
        menu.addAction(QStringLiteral("Reset track heights"), this, &TimelineWidget::resetTrackHeights);
    }
    if (!menu.isEmpty()) menu.exec(event->globalPos());
}

void TimelineWidget::leaveEvent(QEvent *) {
    if (m_scrubbing || m_dragging || m_resizeRow >= 0) return;
    clearHoverClip();
    m_hoverGrip = -1;
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

void TimelineWidget::layoutRows() {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    QVector<int> picture;
    QVector<int> sound;
    for (int index = 0; index < tracks.size(); ++index)
        (isAudioTrack(tracks.at(index).toObject()) ? sound : picture).append(index);
    m_rows.clear();
    int top = kRulerHeight;
    const auto add = [&](int index, bool audio, int number) {
        const int height = m_trackHeights.value(trackKey(tracks.at(index).toObject(), index),
                                                audio ? kTrackHeight : kVideoTrackHeight);
        m_rows.append(TrackRow{index, audio, number, top, height});
        top += height;
    };
    for (int number = static_cast<int>(picture.size()); number >= 1; --number)
        add(picture.at(number - 1), false, number);
    for (int number = 1; number <= static_cast<int>(sound.size()); ++number)
        add(sound.at(number - 1), true, number);
}

void TimelineWidget::relayout() {
    layoutRows();
    invalidateLanes();
    updateContentSize();
    update();
}

void TimelineWidget::setTrackHeight(int row, int height) {
    if (row < 0 || row >= m_rows.size()) return;
    const TrackRow track = m_rows.at(row);
    const int clamped = std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
    if (clamped == track.height) return;
    const QJsonObject object = m_project.value(QStringLiteral("tracks")).toArray().at(track.track).toObject();
    m_trackHeights.insert(trackKey(object, track.track), clamped);
    relayout();
}

int TimelineWidget::rowAt(int y) const {
    for (int row = 0; row < m_rows.size(); ++row)
        if (y >= m_rows.at(row).top && y < m_rows.at(row).bottom()) return row;
    return -1;
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
    if (!inHeader(point) || inRuler(point)) return -1;
    for (int row = 0; row < m_rows.size(); ++row)
        if (std::abs(point.y() - m_rows.at(row).bottom()) <= kResizeGrip) return row;
    return -1;
}

void TimelineWidget::clearHoverClip() {
    if (m_hoverClipId.isEmpty()) return;
    m_hoverClipId.clear();
    m_hoverEdge = 0;
    invalidateLanes();
    update();
}

void TimelineWidget::paintLanes(QPainter &p, const QRect &visible) {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    p.setRenderHint(QPainter::Antialiasing, false);
    // Below the last track is an empty well, so spare height never reads as a track.
    p.fillRect(visible, QColor("#121212"));
    for (int row = 0; row < m_rows.size(); ++row) {
        const TrackRow &track = m_rows.at(row);
        if (track.bottom() <= visible.top() || track.top > visible.bottom()) continue;
        paintLane(p, visible, row, tracks.at(track.track).toObject());
    }
}

void TimelineWidget::paintLane(QPainter &p, const QRect &visible, int row, const QJsonObject &track) {
    const TrackRow &layout = m_rows.at(row);
    const QColor accent = trackAccent(track, layout.track);
    const QRect lane(kTimelineLabelWidth, layout.top, std::max(0, width() - kTimelineLabelWidth), layout.height - 1);
    p.fillRect(lane, QColor("#1c1c1c"));
    if (track.value(QStringLiteral("locked")).toBool())
        p.fillRect(lane, QBrush(QColor(255, 255, 255, 14), Qt::BDiagPattern));

    p.save();
    p.setClipRect(lane.adjusted(0, 0, 0, 1).intersected(visible));
    if (!track.value(QStringLiteral("enabled")).toBool(true)) p.setOpacity(0.4);
    for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray())
        paintClip(p, visible, row, value.toObject(), accent, layout.audio);
    if (!m_proposal.isEmpty()) paintProposal(p, row, track);
    p.restore();

    p.fillRect(lane.left(), layout.bottom() - 1, lane.width(), 1, QColor("#0c0c0c"));
    // Picture and sound read as two banks, the way a conform is laid out.
    if (row > 0 && m_rows.at(row - 1).audio != layout.audio)
        p.fillRect(lane.left(), layout.top, lane.width(), 2, QColor("#050505"));
}

void TimelineWidget::paintHeaders(QPainter &p, const QRect &dirty) {
    const QRect column = headerColumnRect();
    if (!column.intersects(dirty)) return;
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    p.save();
    p.setClipRect(column, Qt::IntersectClip);
    p.fillRect(column, QColor("#171717"));
    for (int row = 0; row < m_rows.size(); ++row) {
        const TrackRow &track = m_rows.at(row);
        if (track.bottom() <= column.top() || track.top > dirty.bottom()) continue;
        paintTrackHeader(p, row, tracks.at(track.track).toObject(), column.left());
    }
    const int grip = m_resizeRow >= 0 ? m_resizeRow : m_hoverGrip;
    if (grip >= 0 && grip < m_rows.size())
        p.fillRect(column.left(), m_rows.at(grip).bottom() - 2, column.width() - 1, 2, QColor("#8a8a8a"));
    p.fillRect(column.left() + kTimelineLabelWidth - 1, column.top(), 1, column.height(), QColor("#0a0a0a"));
    p.restore();
}

void TimelineWidget::paintTrackHeader(QPainter &p, int row, const QJsonObject &track, int left) {
    const TrackRow &layout = m_rows.at(row);
    const QColor accent = trackAccent(track, layout.track);
    const bool enabled = track.value(QStringLiteral("enabled")).toBool(true);
    const QRect header(left, layout.top, kTimelineLabelWidth - 1, layout.height - 1);
    p.fillRect(header, QColor("#1f1f1f"));
    p.fillRect(QRect(left, layout.top, header.width(), 1), QColor("#282828"));
    p.fillRect(QRect(left, layout.top, 3, header.height()), enabled ? accent : accent.darker(240));
    p.fillRect(QRect(left, layout.bottom() - 1, kTimelineLabelWidth, 1), QColor("#0c0c0c"));
    if (row > 0 && m_rows.at(row - 1).audio != layout.audio)
        p.fillRect(QRect(left, layout.top, kTimelineLabelWidth, 2), QColor("#050505"));

    // Tall rows show a name line over a detail line; short rows keep only
    // the track number and name, centered.
    const bool detailed = layout.height >= 40;
    const int blockTop = detailed ? layout.top + (header.height() - 31) / 2
                                  : layout.top + (header.height() - 16) / 2;
    QFont codeFont(QStringLiteral("Menlo"));
    codeFont.setStyleHint(QFont::Monospace);
    codeFont.setPixelSize(10);
    codeFont.setWeight(QFont::Bold);
    const QRect badge(left + 10, blockTop, 22, 16);
    p.fillRect(badge, QColor("#2a2a2a"));
    frameInside(p, badge, 1, QColor("#454545"));
    p.setFont(codeFont);
    p.setPen(enabled ? QColor("#ececec") : QColor("#7a7a7a"));
    p.drawText(badge, Qt::AlignCenter, trackRowLabel(row));

    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    QImage preview;
    if (detailed && m_artwork && !clips.isEmpty()) {
        const QString sourceId = clips.first().toObject().value(QStringLiteral("sourceId")).toString();
        if (layout.audio && m_artwork->waves.contains(sourceId)) {
            preview = tintedWave(sourceId, clipColors(accent, false, false).wave);
        } else if (!layout.audio) {
            preview = m_artwork->frames.value(sourceId);
        }
    }
    const QRect slot = headerPreviewRect(row, left);
    if (!preview.isNull()) {
        p.fillRect(slot, QColor("#0e0e0e"));
        p.drawImage(slot.adjusted(1, 1, -1, -1), preview);
        frameInside(p, slot, 1, QColor("#0a0a0a"));
    }

    QFont nameFont = font();
    nameFont.setPixelSize(11);
    nameFont.setWeight(QFont::Medium);
    p.setFont(nameFont);
    // Output (or mute) and lock toggles sit after the track number.
    const auto drawToggle = [&](int index, bool on, const QString &field) {
        const QRect box = headerToggleRect(row, index).translated(left - stickyOrigin().x(), 0);
        p.save();
        p.fillRect(box, on ? QColor("#e6b450") : QColor("#2a2a2a"));
        frameInside(p, box, 1, on ? QColor("#e6b450") : QColor("#454545"));
        p.setPen(QPen(on ? QColor("#141414") : QColor("#9a9a9a"), 1.2));
        p.setBrush(Qt::NoBrush);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QPointF c = QRectF(box).center();
        if (field == QStringLiteral("locked")) {
            p.drawArc(QRectF(c.x() - 3, c.y() - 6, 6, 7), 0, 180 * 16);
            p.drawRect(QRectF(c.x() - 4, c.y() - 2, 8, 6));
        } else if (field == QStringLiteral("muted")) {
            QFont mute(QStringLiteral("Menlo"));
            mute.setPixelSize(9);
            mute.setWeight(QFont::Bold);
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
    p.setPen(enabled ? QColor("#d8d8d8") : QColor("#808080"));
    p.drawText(QRect(nameLeft, badge.top(), std::max(1, nameRight - nameLeft), badge.height()),
               Qt::AlignLeft | Qt::AlignVCenter | Qt::TextSingleLine,
               QFontMetrics(nameFont).elidedText(name, Qt::ElideRight, std::max(1, nameRight - nameLeft)));
    if (!detailed) return;

    QFont metaFont = nameFont;
    metaFont.setPixelSize(10);
    metaFont.setWeight(QFont::Normal);
    p.setFont(metaFont);
    const QString count = clips.size() == 1 ? QStringLiteral("1 clip") : QStringLiteral("%1 clips").arg(clips.size());
    const QRect meta(badge.left(), blockTop + 19, std::max(1, nameRight - badge.left()), 12);
    p.setPen(QColor("#7c7c7c"));
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
    const QColor accent("#f0b429");
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    for (const QJsonValue &value : proposed) {
        const QJsonObject clip = value.toObject();
        const QString clipId = clip.value(QStringLiteral("id")).toString();
        if (current.value(clipId) == essence(clip)) {
            current.remove(clipId);
            continue;
        }
        current.remove(clipId);
        const QRectF rect = clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(),
                                     clip.value(QStringLiteral("duration")).toDouble()).adjusted(1, 1, -1, -1);
        p.fillRect(rect, QColor(240, 180, 41, 46));
        p.setPen(QPen(accent, 1.5, Qt::DashLine));
        p.setBrush(Qt::NoBrush);
        p.drawRect(rect);
    }
    // Clips the proposal leaves out.
    for (auto it = current.cbegin(); it != current.cend(); ++it) {
        const QRectF rect = clipRect(row, it.value().value(QStringLiteral("timelineStart")).toDouble(),
                                     it.value().value(QStringLiteral("duration")).toDouble());
        p.fillRect(rect, QBrush(QColor(232, 96, 96, 150), Qt::BDiagPattern));
        p.setPen(QPen(QColor("#e86060"), 1.5));
        p.drawRect(rect.adjusted(1, 1, -1, -1));
    }
    p.restore();
}

void TimelineWidget::setTool(TimelineTool tool) {
    m_tool = tool;
    clearHoverClip();
    setCursor(Qt::ArrowCursor);
}

QRect TimelineWidget::headerToggleRect(int row, int index) const {
    const TrackRow &layout = m_rows.at(row);
    const int height = layout.height - 1;
    const int top = layout.height >= 40 ? layout.top + (height - 31) / 2 : layout.top + (height - 16) / 2;
    return QRect(stickyOrigin().x() + 36 + index * 20, top, 16, 16);
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

double TimelineWidget::snapped(double time, double length) const {
    if (!m_snapping) return time;
    // Edit points: the start, the playhead, and every other clip's edges.
    QVector<double> points{0.0, m_playhead};
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray()) {
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("id")).toString() == m_dragClipId) continue;
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            points << start << start + clip.value(QStringLiteral("duration")).toDouble();
        }
    }
    const double reach = 8.0 / pixelsPerSecond();
    double best = time;
    double distance = reach;
    // A moved clip snaps by either end; an edge by itself.
    const QVector<double> edges = length > 0.0 ? QVector<double>{0.0, length} : QVector<double>{0.0};
    for (const double edge : edges) {
        for (const double point : points) {
            const double gap = std::abs(time + edge - point);
            if (gap < distance) {
                distance = gap;
                best = point - edge;
            }
        }
    }
    return std::max(0.0, std::round(best * m_fps) / m_fps);
}

void TimelineWidget::paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QColor &accent, bool audio) {
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
    const bool selected = id == m_selectedClip;
    const bool hovered = id == m_hoverClipId;
    const ClipColors colors = clipColors(accent, selected, hovered);
    // Short rows show only the name bar, as a mini track does.
    const qreal bandHeight = rect.height() >= 26.0 ? 13.0 : rect.height();
    const QRectF band(rect.left(), rect.top(), rect.width(), bandHeight);
    const QRectF body(rect.left(), band.bottom(), rect.width(), rect.height() - bandHeight);

    p.save();
    p.setClipRect(rect, Qt::IntersectClip);
    QLinearGradient bandFill(0, band.top(), 0, band.bottom());
    bandFill.setColorAt(0, colors.band.lighter(112));
    bandFill.setColorAt(1, colors.band);
    p.fillRect(band, bandFill);

    const QString sourceId = clip.value(QStringLiteral("sourceId")).toString();
    if (body.height() > 0.0) {
        QLinearGradient bodyFill(0, body.top(), 0, body.bottom());
        bodyFill.setColorAt(0, colors.body.lighter(125));
        bodyFill.setColorAt(1, colors.body);
        p.fillRect(body, bodyFill);
    }
    if (body.height() > 0.0 && audio) {
        p.fillRect(QRectF(body.left(), std::floor(body.center().y()), body.width(), 1), QColor(0, 0, 0, 70));
        if (m_artwork && m_artwork->waves.contains(sourceId)) {
            const QImage wave = tintedWave(sourceId, colors.wave);
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
    } else if (body.height() > 0.0) {
        const QImage frame = m_artwork ? m_artwork->frames.value(sourceId) : QImage();
        if (!frame.isNull()) {
            const double tileHeight = std::max(1.0, body.height());
            const double tileWidth = std::max(1.0, std::round(tileHeight * 16.0 / 9.0));
            const double firstTile = body.left() + std::max(0.0, std::floor((visible.left() - body.left()) / tileWidth)) * tileWidth;
            const double stop = std::min(body.right(), double(visible.right()) + 1.0);
            for (double x = firstTile; x < stop; x += tileWidth) {
                p.drawImage(QRectF(x, body.top(), tileWidth, tileHeight), frame);
                p.fillRect(QRectF(x + tileWidth - 1, body.top(), 1, tileHeight), QColor(0, 0, 0, 120));
            }
        }
    }

    if (rect.width() > 22.0) {
        qreal textRight = rect.right() - 5;
        const bool effected = !clip.value(QStringLiteral("effects")).toArray().isEmpty() ||
                              !clip.value(QStringLiteral("props")).toObject().isEmpty();
        if (effected && rect.width() > 64.0) {
            QFont badgeFont(QStringLiteral("Menlo"));
            badgeFont.setStyleHint(QFont::Monospace);
            badgeFont.setPixelSize(8);
            badgeFont.setWeight(QFont::Bold);
            const QRectF fx(textRight - 16, band.center().y() - 5.0, 16, 10);
            p.fillRect(fx, QColor(0, 0, 0, 110));
            p.setFont(badgeFont);
            p.setPen(QColor("#ededed"));
            p.drawText(fx, Qt::AlignCenter, QStringLiteral("FX"));
            textRight = fx.left() - 4;
        }
        QFont nameFont = font();
        nameFont.setPixelSize(10);
        nameFont.setWeight(QFont::DemiBold);
        p.setFont(nameFont);
        QString label = clipName(clip);
        const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
        if (std::abs(speed - 1.0) > 0.0005) label += QStringLiteral("  [%1%]").arg(std::lround(speed * 100.0));
        const qreal textLeft = rect.left() + 6;
        p.setPen(QColor("#f6f6f6"));
        p.drawText(QRectF(textLeft, band.top(), std::max(1.0, textRight - textLeft), bandHeight),
                   Qt::AlignLeft | Qt::AlignVCenter | Qt::TextSingleLine,
                   QFontMetrics(nameFont).elidedText(label, Qt::ElideRight, std::max(1, int(textRight - textLeft))));
    }
    p.restore();

    p.fillRect(QRectF(rect.left() + 1, rect.top() + 1, rect.width() - 2, 1), colors.highlight);
    if (selected) {
        frameInside(p, rect, 2, QColor("#f0b429"));
    } else {
        frameInside(p, rect, 1, QColor(0, 0, 0, 200));
    }

    // Trim bracket on the edge under the pointer, or the edge being trimmed.
    int edge = 0;
    if (dragged) edge = m_dragLeading ? -1 : m_dragTrailing ? 1 : 0;
    else if (hovered && !m_dragging) edge = m_hoverEdge;
    if (edge != 0) {
        const QColor bracket("#f0b429");
        const qreal x = edge < 0 ? rect.left() : rect.right() - 3;
        p.fillRect(QRectF(x, rect.top(), 3, rect.height()), bracket);
        const qreal serif = edge < 0 ? rect.left() : rect.right() - 8;
        p.fillRect(QRectF(serif, rect.top(), 8, 2), bracket);
        p.fillRect(QRectF(serif, rect.bottom() - 2, 8, 2), bracket);
    }
}

void TimelineWidget::paintRuler(QPainter &p, const QRect &dirty, int top) {
    const QRect band(dirty.left(), top, dirty.width(), kRulerHeight);
    if (!band.intersects(dirty)) return;
    p.setRenderHint(QPainter::Antialiasing, false);
    QLinearGradient fill(0, top, 0, top + kRulerHeight);
    fill.setColorAt(0, QColor("#2a2a2a"));
    fill.setColorAt(1, QColor("#202020"));
    p.fillRect(band.adjusted(0, 0, 0, -1), fill);
    p.fillRect(QRect(band.left(), top + kRulerHeight - 1, band.width(), 1), QColor("#0a0a0a"));

    QFont font(QStringLiteral("Menlo"));
    font.setPixelSize(9);
    font.setStyleHint(QFont::Monospace);
    p.setFont(font);
    const double pps = pixelsPerSecond();
    const RulerScale scale = rulerScale(pps, m_fps);
    const double major = std::max(scale.majorSeconds, 1e-6);
    const double minor = major / scale.divisions;
    const double startTime = (dirty.left() - kTimelineLabelWidth) / pps;
    const double endTime = (dirty.right() - kTimelineLabelWidth) / pps;
    int first = std::max(0, static_cast<int>(std::floor(startTime / major)) - 1);
    const int limit = static_cast<int>(std::ceil(std::max(m_duration, 0.0) / major)) + 1;
    const int last = std::min(limit, static_cast<int>(std::ceil(endTime / major)) + 1);
    for (int index = first; index <= last; ++index) {
        const double time = index * major;
        const int x = static_cast<int>(std::floor(timeToX(time)));
        p.fillRect(QRect(x, top + 3, 1, kRulerHeight - 4), QColor("#666666"));
        if (time <= m_duration + 0.0001) {
            p.setPen(QColor("#b0b0b0"));
            p.drawText(QRectF(x + 4, top + 1, std::max(12.0, major * pps - 8.0), 12),
                       Qt::AlignLeft | Qt::AlignVCenter,
                       timelineRulerLabel(std::min(time, m_duration), m_fps, m_duration));
        }
        for (int step = 1; step < scale.divisions; ++step) {
            const double tick = time + minor * step;
            if (tick > m_duration) break;
            const bool half = scale.divisions % 2 == 0 && step * 2 == scale.divisions;
            const int length = half ? 8 : 4;
            p.fillRect(QRect(static_cast<int>(std::floor(timeToX(tick))), top + kRulerHeight - 1 - length, 1, length),
                       QColor(half ? "#5a5a5a" : "#474747"));
        }
    }
}

void TimelineWidget::paintOverlay(QPainter &p, const QRect &dirty) {
    const QPoint origin = stickyOrigin();
    const bool skimming = m_hasSkimmer && !m_dragging && !m_scrubbing;
    const QColor playColor = skimming ? QColor("#6a6a6a") : QColor("#f0b429");
    const int laneTop = origin.y() + kRulerHeight;
    const int playX = static_cast<int>(std::floor(timeToX(m_playhead)));
    p.setRenderHint(QPainter::Antialiasing, false);
    // A dark halo keeps the line readable over bright filmstrips.
    if (!skimming) p.fillRect(QRect(playX - 1, laneTop, 3, height() - laneTop), QColor(0, 0, 0, 110));
    p.fillRect(QRect(playX, laneTop, 1, height() - laneTop), playColor);

    paintRuler(p, dirty, origin.y());
    if (skimming) {
        const int skimX = static_cast<int>(std::floor(timeToX(m_skimmerTime)));
        p.fillRect(QRect(skimX, origin.y(), 1, height() - origin.y()), QColor(240, 240, 240, 215));
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

    paintHeaders(p, dirty);
    if (m_addLaneVisible) {
        p.fillRect(addLaneHitRect().adjusted(0, 0, -1, -1), QColor("#1d1d1d"));
        const QRect button = addLaneButtonRect();
        p.fillRect(button, QColor("#2a2a2a"));
        frameInside(p, button, 1, QColor("#555555"));
        p.setPen(QColor("#ededed"));
        QFont addFont = font();
        addFont.setPixelSize(11);
        addFont.setWeight(QFont::Medium);
        p.setFont(addFont);
        p.drawText(button, Qt::AlignCenter, QStringLiteral("+  Track"));
    }
    paintCornerTime(p, origin, skimming ? m_skimmerTime : m_playhead, skimming ? QColor("#f4f4f4") : playColor);
    if (m_dragging && m_dragMoved) paintDragReadout(p);
}

void TimelineWidget::paintDragReadout(QPainter &p) {
    const double delta = m_previewTime - dragOrigin();
    const QString text = QStringLiteral("%1  %2%3")
        .arg(timelineTimecode(m_previewTime, m_fps), delta < 0 ? QStringLiteral("-") : QStringLiteral("+"),
             timelineTimecode(std::abs(delta), m_fps));
    QFont font(QStringLiteral("Menlo"));
    font.setStyleHint(QFont::Monospace);
    font.setPixelSize(10);
    const QFontMetrics metrics(font);
    const qreal x = timeToX(m_previewTime);
    QRectF box(0, 0, metrics.horizontalAdvance(text) + 14, 18);
    box.moveTop(std::max(stickyOrigin().y() + kRulerHeight + 2, m_rows.value(m_dragRow).top - 21));
    box.moveLeft(x + 6);
    if (box.right() > width() - 4) box.moveRight(x - 6);
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(box, QColor(10, 10, 10, 235));
    frameInside(p, box, 1, QColor("#4a4a4a"));
    p.setFont(font);
    p.setPen(QColor("#f2f2f2"));
    p.drawText(box, Qt::AlignCenter, text);
}

void TimelineWidget::paintCornerTime(QPainter &p, const QPoint &origin, double time, const QColor &color) const {
    const QRect corner(origin.x(), origin.y(), kTimelineLabelWidth - 1, kRulerHeight - 1);
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(corner, QColor("#1a1a1a"));
    p.fillRect(QRect(corner.right() + 1, origin.y(), 1, kRulerHeight), QColor("#0a0a0a"));
    p.fillRect(QRect(origin.x(), corner.bottom() + 1, kTimelineLabelWidth, 1), QColor("#0a0a0a"));
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

void TimelineWidget::ensureLaneCache() {
    const qreal dpr = std::max<qreal>(1, devicePixelRatioF());
    if (!m_laneCache.isNull()) {
        const qreal cachedW = m_laneCache.width() / m_laneCache.devicePixelRatio();
        const qreal cachedH = m_laneCache.height() / m_laneCache.devicePixelRatio();
        if (qFuzzyCompare(m_laneCache.devicePixelRatio(), dpr) && qFuzzyCompare(cachedW, qreal(width())) &&
            qFuzzyCompare(cachedH, qreal(height())))
            return;
    }
    const int physicalW = std::max(1, static_cast<int>(std::ceil(width() * dpr)));
    const int physicalH = std::max(1, static_cast<int>(std::ceil(height() * dpr)));
    QPixmap cache(physicalW, physicalH);
    if (cache.isNull()) {
        m_laneCache = QPixmap();
        return;
    }
    cache.setDevicePixelRatio(dpr);
    cache.fill(QColor("#121212"));
    QPainter painter(&cache);
    if (!painter.isActive()) {
        m_laneCache = QPixmap();
        return;
    }
    paintLanes(painter, QRect(0, 0, width(), height()));
    m_laneCache = cache;
}

bool TimelineWidget::lanesAreCacheable() const {
    if (width() <= 0 || height() <= 0) return false;
    const double dpr = std::max(1.0, devicePixelRatioF());
    return double(width()) * double(height()) * dpr * dpr <= 6000000.0;
}

qreal TimelineWidget::timeToX(double time) const {
    return kTimelineLabelWidth + time * pixelsPerSecond();
}

QRectF TimelineWidget::clipRect(int row, double start, double duration) const {
    const TrackRow track = m_rows.value(row);
    const double left = std::round(timeToX(start));
    const double right = std::max(left + 4.0, std::round(timeToX(start + duration)));
    return QRectF(left, track.top + kClipInset, right - left, track.height - 1 - kClipInset * 2);
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
    const double contentWidth = kTimelineLabelWidth + m_duration * pixelsPerSecond() + kTimelineTail;
    const int contentHeight = std::max(tracksBottom() + kTrackHeight, kRulerHeight + 3 * kTrackHeight) + 8;
    setMinimumSize(static_cast<int>(std::min(contentWidth, 16777215.0)), contentHeight);
    updateGeometry();
}

double TimelineWidget::pixelsPerSecond() const {
    if (m_fitWidth <= 0) return 72.0 * m_zoom;
    const double fitted = (m_fitWidth - kTimelineLabelWidth - kTimelineTail) / std::max(m_duration, 0.001);
    return std::max(fitted, kMinPixelsPerSecond);
}

double TimelineWidget::timeAt(qreal x) const {
    return std::clamp((x - kTimelineLabelWidth) / pixelsPerSecond(), 0.0, m_duration);
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
                 tracksBottom() + (kTrackHeight - 20) / 2, 76, 20);
}

QRect TimelineWidget::addLaneHitRect() const {
    return QRect(stickyOrigin().x(), tracksBottom(), kTimelineLabelWidth, kTrackHeight);
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

} // namespace cut
