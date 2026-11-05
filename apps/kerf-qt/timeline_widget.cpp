#include "timeline_widget.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_theme.h"
#include "icons.h"
#include "media_artwork.h"
#include "review_model.h"

#include <QAbstractAnimation>
#include <QVariantAnimation>
#include <QApplication>
#include <QCryptographicHash>
#include <QDir>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QFileInfo>
#include <QFontMetrics>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QKeyEvent>
#include <QMetaMethod>
#include <QMimeData>
#include <QMenu>
#include <QPainterPath>
#include <QScrollBar>
#include <QSettings>
#include <QShortcut>
#include <QSignalBlocker>
#include <QTimer>
#include <QToolTip>
#include <QUrl>
#include <QWheelEvent>

#include <algorithm>
#include <climits>
#include <cmath>

namespace kerf {

namespace {

namespace tk = tokens;

QColor C(QRgb rgba) { return tk::color(rgba); }
QColor withAlpha(QRgb rgb, double alpha) {
    QColor color = tk::color(rgb);
    color.setAlphaF(float(alpha));
    return color;
}

// The type the timeline draws in (mockup-spec 0.3).
const QFont &metaFont() {
    static const QFont font = tk::font(tk::meta);
    return font;
}
const QFont &trackNameFont() {
    static const QFont font = tk::font(tk::controlStrong);
    return font;
}
const QFont &monoSmallFont() {
    static const QFont font = tk::font(tk::monoSmall);
    return font;
}
const QFont &monoSmallStrongFont() {
    static const QFont font = tk::font(tk::TypeStyle{11, 16, 500, true});
    return font;
}
const QFont &toggleFont() {
    static const QFont font = tk::font(tk::metaStrong);
    return font;
}
const QFont &cornerFont() {
    static const QFont font = tk::font(tk::monoLarge);
    return font;
}

// A track head's controls: pad 0 6 0 8; the target at 8 (24 wide), the name 8
// after that, and on hover the row of 20 pt toggles (gap 2) against the right
// padding, over the name's end. Every control is 20 tall, centred in its row.
constexpr int kControlSize = 20;
constexpr int kTargetX = 12;
constexpr int kNameX = kTargetX + 24 + 8;
constexpr int kTogglesX = kTimelineLabelWidth - 6 - (3 * kControlSize + 2 * 2);
constexpr int kToggleStep = kControlSize + 2;
// A clip's name bar, and the narrowest clip that shows one.
constexpr qreal kNameBarHeight = 16.0;
constexpr qreal kNameBarFrom = 40.0;
// A clip's secondary text is left out when it would be cut under this.
constexpr qreal kSecondaryMin = 44.0;
constexpr qreal kClipRadius = tk::radius::small;
// The bytes of scaled filmstrip tiles kept before the cache starts again.
constexpr qsizetype kFilmstripTileBudget = 64 * 1024 * 1024;
// The rubber band's range, bottom to top of the clip.
constexpr double kGainFloor = -60.0;
constexpr double kGainCeiling = 12.0;

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

QString controlField(TimelineWidget::HeaderControl control, bool audio) {
    switch (control) {
    case TimelineWidget::HeaderControl::Lock: return QStringLiteral("locked");
    case TimelineWidget::HeaderControl::Output: return audio ? QStringLiteral("muted") : QStringLiteral("enabled");
    default: return {};
    }
}

// A track field as the snapshot has it, with the model's defaults.
bool trackFlag(const QJsonObject &track, const QString &field) {
    const bool fallback = field == QStringLiteral("enabled") || field == QStringLiteral("syncLocked");
    return track.value(field).toBool(fallback);
}

// What a clip's name bar says after its name.
QString clipSecondary(const QJsonObject &clip, ClipKind kind) {
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    if (kind == ClipKind::Generator) {
        QStringList parts;
        const QString text = props.value(QStringLiteral("text")).toString().simplified();
        if (!text.isEmpty() && text != clipName(clip).simplified()) parts << text;
        const QString blend = props.value(QStringLiteral("blend")).toString();
        if (!blend.isEmpty() && blend != QStringLiteral("normal")) parts << blend.left(1).toUpper() + blend.mid(1);
        const QJsonValue opacity = props.value(QStringLiteral("opacity"));
        if (opacity.isDouble() && opacity.toDouble() < 0.9995) parts << QStringLiteral("%1%").arg(std::lround(opacity.toDouble() * 100.0));
        return parts.join(QStringLiteral(" · "));
    }
    const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
    if (std::abs(speed - 1.0) > 0.0005) return QStringLiteral("%1%").arg(std::lround(std::abs(speed) * 100.0));
    return {};
}

bool isRoomTone(const QJsonObject &track) {
    const QString role = track.value(QStringLiteral("role")).toString().toLower();
    const QString name = track.value(QStringLiteral("name")).toString().toLower();
    return role.contains(QStringLiteral("room")) || name.contains(QStringLiteral("room tone"));
}

// A 10 x 10 flag: polygon(0 0, 100% 0, 100% 100%, 50% 70%, 0 100%).
QPolygonF markerFlag(const QPointF &topLeft) {
    return QPolygonF{topLeft, topLeft + QPointF(10, 0), topLeft + QPointF(10, 10), topLeft + QPointF(5, 7), topLeft + QPointF(0, 10)};
}

QPolygonF diamond(const QPointF &centre) {
    const qreal r = 3.0 * std::sqrt(2.0);
    return QPolygonF{centre + QPointF(0, -r), centre + QPointF(r, 0), centre + QPointF(0, r), centre + QPointF(-r, 0)};
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

// ---- TimelineWidget ------------------------------------------------------

TimelineWidget::TimelineWidget(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("timeline"));
    setFocusPolicy(Qt::StrongFocus);
    setAttribute(Qt::WA_OpaquePaintEvent);
    setSizePolicy(QSizePolicy::Minimum, QSizePolicy::MinimumExpanding);
    updateContentSize();
    setMouseTracking(true);
    setAcceptDrops(true);
}

void TimelineWidget::setProject(const QJsonObject &project, double duration, double fps) {
    const double scaleBefore = pixelsPerSecond();
    const double leastBefore = minScale();
    m_project = project;
    m_duration = duration > 0.0 ? std::max(duration, 1.0) : kEmptySequenceSeconds;
    // A zoomed view stays in reach of a sequence that got shorter.
    if (m_fitWidth <= 0) m_zoom = std::max(m_zoom, minScale() / 72.0);
    m_fps = fps > 0.0 ? fps : 24.0;
    // Targets and solos name tracks that may be gone.
    const auto present = [this](const QString &id) {
        for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
            if (track.toObject().value(QStringLiteral("id")).toString() == id) return true;
        return false;
    };
    m_targets.erase(std::remove_if(m_targets.begin(), m_targets.end(), [&](const QString &id) { return !present(id); }), m_targets.end());
    const int solos = int(m_solo.size());
    m_solo.erase(std::remove_if(m_solo.begin(), m_solo.end(), [&](const QString &id) { return !present(id); }), m_solo.end());
    if (solos != m_solo.size()) emit soloChanged(m_solo);
    rebuildBandIndex();
    relayout();
    // A new length moves the fit, and with it the zoom's scale and range.
    if (pixelsPerSecond() != scaleBefore || minScale() != leastBefore) emit scaleChanged(pixelsPerSecond());
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
    m_zoom = std::clamp(zoom, minScale() / 72.0, kMaxScale / 72.0);
    m_fitWidth = 0;
    invalidateLanes();
    updateContentSize();
    update();
    emit scaleChanged(pixelsPerSecond());
}

void TimelineWidget::setScale(double pixelsPerSecond) { setZoom(pixelsPerSecond / 72.0); }

void TimelineWidget::fitToWidth(int width) {
    m_fitWidth = std::max(kTimelineLabelWidth + 32, width);
    m_viewportWidth = m_fitWidth;
    invalidateLanes();
    updateContentSize();
    update();
    emit scaleChanged(pixelsPerSecond());
}

void TimelineWidget::followViewportWidth(int width) {
    if (isFitToWidth()) {
        fitToWidth(width);
        return;
    }
    const double least = minScale();
    m_viewportWidth = std::max(kTimelineLabelWidth + 32, width);
    if (minScale() != least) emit scaleChanged(pixelsPerSecond());
}

void TimelineWidget::refit() {
    if (m_viewportWidth > 0) fitToWidth(m_viewportWidth);
}

double TimelineWidget::fitScale() const {
    if (m_viewportWidth <= 0) return 0.0;
    const double fitted = (m_viewportWidth - kTimelineOrigin - kTimelineTail) / std::max(m_duration, 0.001);
    return std::max(fitted, kMinPixelsPerSecond);
}

double TimelineWidget::minScale() const {
    const double fit = fitScale();
    return fit > 0.0 ? std::min(kMinScale, fit) : kMinScale;
}

void TimelineWidget::setArtwork(MediaArtwork *artwork) {
    m_artwork = artwork;
    connect(artwork, &MediaArtwork::changed, this, [this] {
        m_filmstripTiles.clear();
        m_filmstripTileBytes = 0;
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

void TimelineWidget::setTool(TimelineTool tool) {
    m_tool = tool;
    clearHoverClip();
    setCursor(Qt::ArrowCursor);
    emit toolChanged(tool);
}

void TimelineWidget::setSnapping(bool snapping) {
    if (m_snapping == snapping) return;
    m_snapping = snapping;
    emit snappingChanged(snapping);
}

// ---- review data ---------------------------------------------------------

void TimelineWidget::setCommentMarks(const QList<CommentMark> &marks) {
    const auto same = [](const CommentMark &a, const CommentMark &b) {
        return a.id == b.id && a.at == b.at && a.end == b.end && a.ranged == b.ranged && a.circle == b.circle &&
               a.current == b.current && a.resolved == b.resolved && a.name == b.name;
    };
    if (marks.size() == m_marks.size() && std::equal(marks.cbegin(), marks.cend(), m_marks.cbegin(), same)) return;
    m_marks = marks;
    const QPoint origin = stickyOrigin();
    update(QRect(origin.x(), origin.y(), width(), kRulerHeight));
}

QRect TimelineWidget::commentMarkBounds(const QString &id) const {
    for (const CommentMark &mark : m_marks)
        if (mark.id == id) return commentMarkRect(mark, stickyOrigin().y()).toAlignedRect();
    return {};
}

void TimelineWidget::setProposalBands(const QList<ProposalBand> &bands) {
    // The review changes often (a search keystroke, the pin order during
    // playback) without changing a band: keep the lane cache then.
    const auto same = [](const ProposalBand &a, const ProposalBand &b) {
        const auto sameGhost = [](const ProposalBand::Ghost &x, const ProposalBand::Ghost &y) {
            return x.track == y.track && x.start == y.start && x.end == y.end;
        };
        return a.id == b.id && a.author == b.author && a.start == b.start && a.end == b.end && a.tracks == b.tracks && a.clips == b.clips &&
               a.ghosts.size() == b.ghosts.size() && std::equal(a.ghosts.cbegin(), a.ghosts.cend(), b.ghosts.cbegin(), sameGhost);
    };
    if (bands.size() == m_bands.size() && std::equal(bands.cbegin(), bands.cend(), m_bands.cbegin(), same)) return;
    m_bands = bands;
    rebuildBandIndex();
    invalidateLanes();
    update();
}

QList<TimelineWidget::ProposalBand> TimelineWidget::proposalBands() const { return m_effectiveBands; }

QRect TimelineWidget::proposalBandBounds(const QString &id) const {
    for (const ProposalBand &band : m_effectiveBands)
        if (band.id == id) return bandRect(band, stickyOrigin().y()).toAlignedRect();
    return {};
}

void TimelineWidget::setProposal(const QJsonObject &timeline) {
    if (timeline == m_proposal) return;
    m_proposal = timeline;
    rebuildBandIndex();
    invalidateLanes();
    update();
}

void TimelineWidget::rebuildBandIndex() {
    m_effectiveBands = m_bands;
    if (m_effectiveBands.isEmpty() && !m_proposal.isEmpty()) {
        // The snapshot's first pending proposal, without the review's author.
        const QList<ReviewModel::ClipChange> changes = ReviewModel::diffTimelines(m_project, m_proposal);
        ProposalBand band;
        band.id = m_proposal.value(QStringLiteral("proposalId")).toString();
        bool ranged = false;
        for (const ReviewModel::ClipChange &change : changes) {
            const double start = change.added ? change.newStart : change.removed ? change.oldStart : std::min(change.oldStart, change.newStart);
            const double end = change.added ? change.newEnd : change.removed ? change.oldEnd : std::max(change.oldEnd, change.newEnd);
            band.start = ranged ? std::min(band.start, start) : start;
            band.end = ranged ? std::max(band.end, end) : end;
            ranged = true;
            if (!change.track.isEmpty() && !band.tracks.contains(change.track)) band.tracks << change.track;
            if (change.transition) continue;
            band.clips << change.clip;
            if (!change.added && (change.moved || change.trimmed || change.removed || change.retracked))
                band.ghosts.append({change.track, change.oldStart, change.oldEnd});
        }
        if (ranged) m_effectiveBands << band;
    }
    m_proposedClips.clear();
    for (const ProposalBand &band : std::as_const(m_effectiveBands))
        for (const QString &clip : band.clips) m_proposedClips.insert(clip);
}

void TimelineWidget::setPreview(const QString &proposalId, const QJsonObject &timeline) {
    if (proposalId == m_previewId && timeline == m_previewTimeline) return;
    m_previewId = proposalId;
    m_previewTimeline = proposalId.isEmpty() ? QJsonObject() : timeline;
    m_previewTracks.clear();
    for (const QJsonValue &value : m_previewTimeline.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = value.toObject();
        m_previewTracks.insert(track.value(QStringLiteral("id")).toString(), track);
    }
    invalidateLanes();
    update();
}

void TimelineWidget::setQcClips(const QSet<QString> &clips) {
    if (clips == m_qcClips) return;
    m_qcClips = clips;
    invalidateLanes();
    update();
}

void TimelineWidget::setWideSequence(bool wide) {
    if (wide == m_wideSequence) return;
    m_wideSequence = wide;
    relayout();
}

void TimelineWidget::setMarkedRange(double in, double out) {
    m_markIn = in;
    m_markOut = out;
    update();
}

QStringList TimelineWidget::targetTracks() const {
    if (m_targetsEditable && m_targetsChosen) return m_targets;
    // The first unlocked track of each kind, where a source's picture and
    // sound go until other targets are chosen.
    QStringList targets;
    for (const bool audio : {false, true}) {
        for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
            const QJsonObject track = value.toObject();
            const QString type = track.value(QStringLiteral("type")).toString();
            if ((audio ? type == QStringLiteral("audio") : type == QStringLiteral("video")) && !track.value(QStringLiteral("locked")).toBool()) {
                targets << track.value(QStringLiteral("id")).toString();
                break;
            }
        }
    }
    return targets;
}

void TimelineWidget::setTargetTracks(const QStringList &tracks) {
    const QStringList before = targetTracks();
    m_targets = tracks;
    m_targetsChosen = true;
    update(headerColumnRect());
    if (targetTracks() != before) emit targetsChanged(targetTracks());
}

void TimelineWidget::setTargetsEditable(bool editable) {
    if (editable == m_targetsEditable) return;
    const QStringList before = targetTracks();
    m_targetsEditable = editable;
    update(headerColumnRect());
    if (targetTracks() != before) emit targetsChanged(targetTracks());
}

void TimelineWidget::setSoloTracks(const QStringList &tracks) {
    if (tracks == m_solo) return;
    m_solo = tracks;
    update(headerColumnRect());
    emit soloChanged(m_solo);
}

// ---- track heights -------------------------------------------------------

void TimelineWidget::fitTracksToView() {
    if (m_rows.isEmpty()) return;
    // Under the ruler, leaving the add-track row in view.
    const int available = viewportRect().height() - kRulerHeight - kAddLaneHeight;
    setAllTrackHeights(available / static_cast<int>(m_rows.size()));
}

void TimelineWidget::resetTrackHeights() {
    const int keep = firstVisibleRow();
    m_trackHeights.clear();
    m_allHeight = 0;
    relayoutKeeping(keep);
    saveHeights();
}

void TimelineWidget::setAllTrackHeights(int height) { setAllTrackHeightsKeeping(height, firstVisibleRow()); }

void TimelineWidget::setAllTrackHeightsKeeping(int height, int row) {
    m_trackHeights.clear();
    m_allHeight = std::clamp(height, kMinTrackHeight, kMaxTrackHeight);
    relayoutKeeping(row);
    saveHeights();
}

void TimelineWidget::stepAllTrackHeights(int steps) { stepAllTrackHeightsKeeping(steps, firstVisibleRow()); }

void TimelineWidget::stepAllTrackHeightsKeeping(int steps, int row) {
    if (steps == 0 || m_rows.isEmpty()) return;
    // Each track from its own height; tracks added later from the height
    // tracks without one of their own have. Only the tracks that end up
    // apart from that keep a height of their own.
    m_allHeight = steppedHeight(defaultTrackHeight(false, 1), steps);
    QHash<QString, int> heights;
    for (int index = 0; index < m_rows.size(); ++index) {
        const int height = steppedHeight(m_rows.at(index).height, steps);
        if (height != m_allHeight) heights.insert(rowKey(index), height);
    }
    m_trackHeights = heights;
    relayoutKeeping(row);
    saveHeights();
}

int TimelineWidget::defaultTrackHeight(bool audio, int number) const {
    if (m_allHeight > 0) return m_allHeight;
    // The picture lane large enough to see what is on it.
    if (m_roomyLanes) return audio ? 28 : number == 1 ? 64 : 40;
    if (m_wideSequence) return audio ? kWideAudioHeight : number == 1 ? kWideV1Height : number == 2 ? kWideV2Height : kWideVideoHeight;
    return audio ? kTrackHeight : kVideoTrackHeight;
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

QString TimelineWidget::trackRowId(int row) const { return rowTrack(row).value(QStringLiteral("id")).toString(); }

QRect TimelineWidget::trackHeaderRect(int row) const {
    const TrackRow track = m_rows.value(row);
    return QRect(stickyOrigin().x(), track.top, kTimelineLabelWidth, track.height);
}

QString TimelineWidget::trackKey(const QJsonObject &track, int index) {
    const QString id = track.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("#%1").arg(index) : id;
}

QString TimelineWidget::rowKey(int row) const {
    const int index = m_rows.value(row).track;
    return trackKey(m_project.value(QStringLiteral("tracks")).toArray().at(index).toObject(), index);
}

QJsonObject TimelineWidget::rowTrack(int row) const {
    if (row < 0 || row >= m_rows.size()) return {};
    return m_project.value(QStringLiteral("tracks")).toArray().at(m_rows.at(row).track).toObject();
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
        const int height = m_trackHeights.value(trackKey(tracks.at(index).toObject(), index), defaultTrackHeight(audio, number));
        m_rows.append(TrackRow{index, audio, number, top, height});
        top += height;
    };
    for (int number = static_cast<int>(picture.size()); number >= 1; --number) add(picture.at(number - 1), false, number);
    for (int number = 1; number <= static_cast<int>(sound.size()); ++number) add(sound.at(number - 1), true, number);
    m_lastPicture = static_cast<int>(picture.size()) - 1;
}

void TimelineWidget::relayout() {
    layoutRows();
    invalidateLanes();
    updateContentSize();
    update();
    emit heightsChanged();
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
    const auto after = std::upper_bound(m_rows.cbegin(), m_rows.cend(), y, [](int value, const TrackRow &row) { return value < row.top; });
    if (after == m_rows.cbegin()) return -1;
    const auto row = after - 1;
    return y < row->bottom() ? static_cast<int>(row - m_rows.cbegin()) : -1;
}

std::pair<int, int> TimelineWidget::rowsBetween(int top, int bottom) const {
    const auto first = std::upper_bound(m_rows.cbegin(), m_rows.cend(), top, [](int value, const TrackRow &row) { return value < row.bottom(); });
    const auto last = std::upper_bound(first, m_rows.cend(), bottom, [](int value, const TrackRow &row) { return value < row.top; });
    return {static_cast<int>(first - m_rows.cbegin()), static_cast<int>(last - m_rows.cbegin())};
}

QRect TimelineWidget::viewportRect() const {
    if (isWindow() || !parentWidget()) return rect();
    return QRect(-pos(), parentWidget()->size()).intersected(rect());
}

QRect TimelineWidget::headerColumnRect() const {
    const QPoint origin = stickyOrigin();
    return QRect(origin.x(), origin.y() + kRulerHeight, kTimelineLabelWidth, std::max(0, height() - origin.y() - kRulerHeight));
}

// ---- head controls -------------------------------------------------------

QRect TimelineWidget::headerControlRect(int row, HeaderControl control) const {
    if (row < 0 || row >= m_rows.size()) return {};
    const TrackRow &layout = m_rows.at(row);
    const int top = layout.top + (layout.height - kControlSize) / 2;
    const int left = stickyOrigin().x();
    switch (control) {
    case HeaderControl::Target: return QRect(left + kTargetX, top, 24, kControlSize);
    // Lock and the eye against the padding on picture; lock, mute and solo on sound.
    case HeaderControl::Lock: return QRect(left + (layout.audio ? kTogglesX : kTogglesX + kToggleStep), top, kControlSize, kControlSize);
    case HeaderControl::Output: return QRect(left + (layout.audio ? kTogglesX + kToggleStep : kTogglesX + 2 * kToggleStep), top, kControlSize, kControlSize);
    case HeaderControl::Solo:
        if (!layout.audio) return {};
        return QRect(left + kTogglesX + 2 * kToggleStep, top, kControlSize, kControlSize);
    }
    return {};
}

QRect TimelineWidget::headerToggleRect(int row, int index) const {
    return headerControlRect(row, index == 1 ? HeaderControl::Lock : HeaderControl::Output);
}

bool TimelineWidget::controlAt(const QPoint &point, int *row, HeaderControl *control) const {
    if (!inHeader(point) || inRuler(point)) return false;
    const int at = rowAt(point.y());
    if (at < 0) return false;
    for (const HeaderControl candidate : {HeaderControl::Target, HeaderControl::Lock, HeaderControl::Output, HeaderControl::Solo}) {
        if (!headerControlRect(at, candidate).contains(point)) continue;
        *row = at;
        *control = candidate;
        return true;
    }
    return false;
}

bool TimelineWidget::toggleAt(const QPoint &point, int *row, QString *field) const {
    HeaderControl control{};
    if (!controlAt(point, row, &control)) return false;
    *field = controlField(control, m_rows.at(*row).audio);
    return !field->isEmpty();
}

void TimelineWidget::pressControl(int row, HeaderControl control) {
    const QJsonObject track = rowTrack(row);
    const QString id = track.value(QStringLiteral("id")).toString();
    switch (control) {
    case HeaderControl::Lock:
    case HeaderControl::Output: {
        const QString field = controlField(control, m_rows.at(row).audio);
        emit trackToggleRequested(id, field, !trackFlag(track, field));
        break;
    }
    case HeaderControl::Target: {
        // Read-only until source edits follow the targets.
        if (!m_targetsEditable) break;
        QStringList targets = targetTracks();
        if (targets.contains(id)) targets.removeAll(id);
        else targets << id;
        setTargetTracks(targets);
        break;
    }
    case HeaderControl::Solo: {
        QStringList solo = m_solo;
        if (solo.contains(id)) solo.removeAll(id);
        else solo << id;
        setSoloTracks(solo);
        break;
    }
    }
}

int TimelineWidget::resizeRowAt(const QPoint &point) const {
    if (!inHeader(point) || inRuler(point) || m_rows.isEmpty()) return -1;
    // A control keeps its clicks, however short its row.
    int controlRow = -1;
    HeaderControl control{};
    if (controlAt(point, &controlRow, &control)) return -1;
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

// ---- events --------------------------------------------------------------

bool TimelineWidget::event(QEvent *event) {
    if (event->type() != QEvent::ToolTip) return QWidget::event(event);
    const auto *help = static_cast<QHelpEvent *>(event);
    const QPoint pos = help->pos();
    const auto show = [&](const QString &name, const QString &key, const QRect &area) {
        QToolTip::showText(help->globalPos(), key.isEmpty() ? name : name + QStringLiteral("  ·  ") + key, this, area);
        return true;
    };
    // The line between two headers says what dragging it does.
    if (const int grip = resizeRowAt(pos); grip >= 0)
        return show(tr("Drag to resize %1. Option-drag resizes every %2 track; Option-scroll over the headers resizes every track.")
                        .arg(trackRowLabel(grip), m_rows.at(grip).audio ? tr("sound") : tr("picture")),
                    {}, {});
    int row = -1;
    HeaderControl control{};
    if (controlAt(pos, &row, &control)) {
        const QJsonObject track = rowTrack(row);
        const QRect area = headerControlRect(row, control);
        switch (control) {
        case HeaderControl::Target:
            if (m_targetsEditable)
                return show(targetTracks().contains(track.value(QStringLiteral("id")).toString()) ? tr("Targeted") : tr("Target track"), {}, area);
            return show(targetTracks().contains(track.value(QStringLiteral("id")).toString())
                            ? tr("Targeted: source edits land here (the first unlocked %1 track)").arg(m_rows.at(row).audio ? tr("sound") : tr("picture"))
                            : tr("Source edits land on the first unlocked %1 track").arg(m_rows.at(row).audio ? tr("sound") : tr("picture")),
                        {}, area);
        case HeaderControl::Lock: return show(trackFlag(track, QStringLiteral("locked")) ? tr("Unlock track") : tr("Lock track"), {}, area);
        case HeaderControl::Output:
            if (m_rows.at(row).audio) return show(trackFlag(track, QStringLiteral("muted")) ? tr("Unmute") : tr("Mute"), {}, area);
            return show(trackFlag(track, QStringLiteral("enabled")) ? tr("Hide track") : tr("Show track"), {}, area);
        case HeaderControl::Solo: return show(tr("Solo (monitoring only)"), {}, area);
        }
    }
    if (inRuler(pos) && !inHeader(pos)) {
        if (const int mark = commentMarkAt(pos); mark >= 0)
            return show(m_marks.at(mark).name, timelineTimecode(m_marks.at(mark).at, m_fps), commentMarkRect(m_marks.at(mark), stickyOrigin().y()).toAlignedRect());
        for (const QJsonValue &value : m_project.value(QStringLiteral("markers")).toArray()) {
            const QJsonObject marker = value.toObject();
            const QRectF flag(markerRect(marker, stickyOrigin().y()).topLeft(), QSizeF(10, 10));
            if (!flag.contains(pos)) continue;
            const QString name = marker.value(QStringLiteral("name")).toString();
            return show(name.isEmpty() ? tr("Sequence marker") : tr("Sequence marker · %1").arg(name),
                        timelineTimecode(marker.value(QStringLiteral("time")).toDouble(), m_fps), flag.toAlignedRect());
        }
        if (const int band = bandAt(pos); band >= 0) {
            const QString author = m_effectiveBands.at(band).author;
            return show(author.isEmpty() ? tr("Proposal") : tr("Proposal · %1").arg(author), {},
                        bandRect(m_effectiveBands.at(band), stickyOrigin().y()).toAlignedRect());
        }
    }
    if (const ClipHit hit = clipAt(pos); !hit.id.isEmpty()) {
        const QJsonObject clip = [&] {
            for (const QJsonValue &value : rowTrack(hit.row).value(QStringLiteral("clips")).toArray())
                if (value.toObject().value(QStringLiteral("id")).toString() == hit.id) return value.toObject();
            return QJsonObject();
        }();
        return show(clipName(clip),
                    timelineTimecode(hit.start, m_fps) + QStringLiteral("–") + timelineTimecode(hit.start + hit.duration, m_fps),
                    hit.rect.toAlignedRect());
    }
    QToolTip::hideText();
    event->ignore();
    return true;
}

void TimelineWidget::setAgentCursor(double seconds) {
    if (std::isnan(seconds) && std::isnan(m_agentCursor)) return;
    if (seconds == m_agentCursor) return;
    m_agentCursor = seconds;
    update();
}

void TimelineWidget::setFocusClips(const QSet<QString> &clips) {
    if (clips == m_focusClips) return;
    m_focusClips = clips;
    // Those looked at rise off their lanes.
    m_focusLift = 0.0;
    const int length = ui::motionDuration(tk::motion::stage);
    if (clips.isEmpty() || length <= 0) {
        m_focusLift = clips.isEmpty() ? 0.0 : 8.0;
        update();
        return;
    }
    auto *rise = new QVariantAnimation(this);
    rise->setDuration(length);
    rise->setStartValue(0.0);
    rise->setEndValue(8.0);
    rise->setEasingCurve(tk::easing());
    connect(rise, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_focusLift = value.toReal();
        update();
    });
    rise->start(QAbstractAnimation::DeleteWhenStopped);
}

QRect TimelineWidget::laneRect(const QString &track, double start, double end) const {
    if (m_rows.isEmpty()) return {};
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    int row = 0;
    for (int index = 0; index < m_rows.size(); ++index)
        if (tracks.at(m_rows.at(index).track).toObject().value(QStringLiteral("id")).toString() == track) {
            row = index;
            break;
        }
    return clipRect(row, start, std::max(0.0, end - start)).toAlignedRect();
}

bool TimelineWidget::isAudioLane(const QString &track) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject object = value.toObject();
        if (object.value(QStringLiteral("id")).toString() == track) return isAudioTrack(object);
    }
    return false;
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
    // A strip outside the cache goes through a pixmap too, so it matches
    // the cache pixel for pixel: names drawn on a window's backing store
    // and on a pixmap can rasterise differently (FreeType on Linux).
    for (const QRect &visible : direct) {
        const QPixmap lanes = renderLanes(visible);
        if (lanes.isNull()) paintLanes(p, visible);
        else blitLaneCache(p, lanes, visible, visible.topLeft());
    }
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
    if (m_dragging || m_scrubbing || m_resizeRow >= 0 || !m_band.clip.isEmpty()) {
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
    // The skim ends here, so the viewer goes back to the playhead's frame.
    const bool skimming = m_hasSkimmer;
    clearSkimmer();
    if (skimming) emit hoverSeekEnded();
    if (m_showsProposedCut) {
        beginScrub(event->position());
        return;
    }
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
    int controlRow = -1;
    HeaderControl control{};
    if (controlAt(pos, &controlRow, &control)) {
        pressControl(controlRow, control);
        return;
    }
    if (inHeader(pos)) return;
    if (inRuler(pos)) {
        if (const int mark = commentMarkAt(pos); mark >= 0) {
            emit commentMarkActivated(m_marks.at(mark).id);
            return;
        }
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
    // A sound clip's rubber band: its keys and its line take the press.
    if (int key = -1; m_tool == TimelineTool::Selection && bandAt(pos, hit, &key)) {
        const bool addKey = (event->modifiers() & Qt::AltModifier) && key < 0;
        beginBandDrag(hit, key, event->position().y(), addKey, timeAt(event->position().x()) - hit.start);
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
    if (!m_band.clip.isEmpty()) {
        const qreal dy = event->position().y() - m_band.pressY;
        const QRect bounds = clipBounds(m_band.clip);
        const QRectF rect(bounds);
        for (int index = 0; index < m_band.values.size(); ++index) {
            if (m_band.key >= 0 && index != m_band.key) continue;
            m_band.values[index] = gainAt(gainY(m_band.from.at(index), rect) + dy, rect);
        }
        m_band.moved = m_band.moved || std::abs(dy) >= 1.0;
        refreshLanes(bounds);
        return;
    }
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
        // The head under the pointer shows its sync-lock; its control under
        // the pointer lights.
        const int hoverRow = inHeader(pos) && !inRuler(pos) ? rowAt(pos.y()) : -1;
        int controlRow = -1;
        HeaderControl control{};
        // A read-only target neither lights nor takes the pointing hand.
        const int hoverControl = grip < 0 && controlAt(pos, &controlRow, &control) && (control != HeaderControl::Target || m_targetsEditable)
                                     ? controlRow * 8 + int(control)
                                     : -1;
        if (hoverRow != m_hoverRow || hoverControl != m_hoverControl) {
            m_hoverRow = hoverRow;
            m_hoverControl = hoverControl;
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
        const int edge = hit.id.isEmpty()                 ? 0
                         : x - hit.rect.left() < kTrimHandle ? -1
                         : hit.rect.right() - x < kTrimHandle ? 1
                                                              : 0;
        if (hit.id != m_hoverClipId || edge != m_hoverEdge) {
            const QRect before = m_hoverRect;
            m_hoverClipId = hit.id;
            m_hoverEdge = edge;
            m_hoverRect = hit.id.isEmpty() ? QRect() : hit.rect.toAlignedRect();
            refreshLanes(before);
            refreshLanes(m_hoverRect);
        }
        int key = -1;
        if (!hit.id.isEmpty() && m_tool == TimelineTool::Selection && bandAt(pos, hit, &key)) setCursor(Qt::SizeVerCursor);
        else if (hoverControl >= 0) setCursor(Qt::PointingHandCursor);
        else setToolCursor(hit, edge);
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
    if (!m_band.clip.isEmpty()) {
        finishBandDrag();
        return;
    }
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
    if (m_dragLeading || m_dragTrailing) emit trimRequested(m_dragClipId, m_dragLeading, static_cast<double>(std::llround(m_previewTime * m_fps)));
    else emit moveRequested(m_dragClipId, m_previewTime);
    m_dragClipId.clear();
    setCursor(Qt::ArrowCursor);
    invalidateLanes();
    update();
}

void TimelineWidget::mouseDoubleClickEvent(QMouseEvent *event) {
    if (m_showsProposedCut) return;
    const ClipHit hit = clipAt(event->position().toPoint());
    if (event->button() != Qt::LeftButton || hit.id.isEmpty()) return;
    if (event->modifiers() & Qt::AltModifier) emit askRequested(hit.id);
    else emit clipActivated(hit.id);
}

void TimelineWidget::contextMenuEvent(QContextMenuEvent *event) {
    if (m_showsProposedCut) return;
    const QPoint pos = event->pos();
    if (const ClipHit hit = clipAt(pos); !hit.id.isEmpty()) {
        emit clipSelected(hit.id);
        emit clipMenuRequested(hit.id, event->globalPos());
        return;
    }
    if (!inHeader(pos) || inRuler(pos)) return;
    const int row = rowAt(pos.y());
    QMenu menu(this);
    ui::styleMenu(&menu);
    // Medium is a track's default height.
    const auto presetHeight = [this](const TrackHeightPreset &preset, const TrackRow &track) {
        return preset.height > 0 ? preset.height : defaultTrackHeight(track.audio, track.number);
    };
    if (row >= 0) {
        // A key, not a row index: a snapshot can relayout rows while the menu is open.
        const QString key = rowKey(row);
        const TrackRow track = m_rows.at(row);
        // Sync lock: whether ripple edits move this track.
        const QJsonObject trackObject = rowTrack(row);
        const QString trackId = trackObject.value(QStringLiteral("id")).toString();
        const bool synced = trackFlag(trackObject, QStringLiteral("syncLocked"));
        QAction *sync = menu.addAction(tr("Sync lock"));
        sync->setObjectName(QStringLiteral("trackSyncLock"));
        sync->setCheckable(true);
        sync->setChecked(synced);
        sync->setToolTip(tr("Ripple edits move this track"));
        connect(sync, &QAction::triggered, this, [this, trackId, synced] {
            emit trackToggleRequested(trackId, QStringLiteral("syncLocked"), !synced);
        });
        menu.addSeparator();
        QMenu *one = menu.addMenu(tr("Track height"));
        ui::styleMenu(one);
        for (const auto &preset : kTrackHeightPresets) {
            QAction *action = one->addAction(tr(preset.name));
            action->setCheckable(true);
            action->setChecked(track.height == presetHeight(preset, track));
            connect(action, &QAction::triggered, this, [this, key, height = preset.height] {
                if (height > 0) m_trackHeights.insert(key, height);
                else m_trackHeights.remove(key);
                relayout();
                saveHeights();
            });
        }
    }
    if (!m_rows.isEmpty()) {
        QMenu *all = menu.addMenu(tr("All tracks"));
        ui::styleMenu(all);
        for (const auto &preset : kTrackHeightPresets) {
            QAction *action = all->addAction(tr(preset.name));
            action->setCheckable(true);
            action->setChecked(std::all_of(m_rows.cbegin(), m_rows.cend(), [&](const TrackRow &track) {
                return track.height == presetHeight(preset, track);
            }));
            connect(action, &QAction::triggered, this, [this, height = preset.height] {
                if (height > 0) setAllTrackHeights(height);
                else resetTrackHeights();
            });
        }
        menu.addSeparator();
        menu.addAction(tr("Fit tracks to panel"), this, &TimelineWidget::fitTracksToView);
        menu.addAction(tr("Reset track heights"), this, &TimelineWidget::resetTrackHeights);
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
    // pointer in place. Some platforms turn Option-scroll sideways. A
    // trackpad's momentum after the fingers lift would carry on stepping
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
    if (m_scrubbing || m_dragging || m_resizeRow >= 0 || !m_band.clip.isEmpty()) return;
    clearHoverClip();
    m_hoverGrip = -1;
    if (m_hoverControl >= 0 || m_hoverRow >= 0) {
        m_hoverControl = -1;
        m_hoverRow = -1;
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

void TimelineWidget::clearHoverClip() {
    if (m_hoverClipId.isEmpty()) return;
    m_hoverClipId.clear();
    m_hoverEdge = 0;
    refreshLanes(std::exchange(m_hoverRect, QRect()));
}

void TimelineWidget::refreshLanes(const QRect &rect) {
    if (rect.isEmpty()) return;
    // A clip's lines and trim bracket stay inside it; a band's keys reach a
    // few points past its ends (and a point over its top, within its lane),
    // so a refresh never takes in the rows above and below.
    const QRect area = rect.adjusted(-5, -1, 5, 1);
    const QRect cached = area.intersected(m_laneCacheRect);
    if (!m_laneCache.isNull() && !cached.isEmpty()) {
        QPainter painter(&m_laneCache);
        painter.translate(-m_laneCacheRect.topLeft());
        painter.setClipRect(cached);
        paintLanes(painter, cached);
    }
    update(area);
}

QStaticText TimelineWidget::laidName(const QFont &font, const QString &text, qreal width) {
    const int whole = std::max(1, int(width));
    const QString key = QString::number(font.pixelSize()) + QLatin1Char('/') + QString::number(font.weight()) + QLatin1Char('/') +
                        QString::number(whole) + QLatin1Char('|') + text;
    auto laid = m_names.constFind(key);
    if (laid == m_names.cend()) {
        if (m_names.size() > 8192) m_names.clear();
        QStaticText name(QFontMetrics(font).elidedText(text, Qt::ElideRight, whole));
        name.setTextFormat(Qt::PlainText);
        name.setPerformanceHint(QStaticText::AggressiveCaching);
        name.prepare(QTransform(), font);
        laid = m_names.insert(key, name);
    }
    return laid.value();
}

void TimelineWidget::drawName(QPainter &p, const QRectF &rect, const QFont &font, const QString &text, bool centred) {
    const QStaticText laid = laidName(font, text, rect.width());
    p.setFont(font);
    const qreal left = centred ? rect.left() + (rect.width() - laid.size().width()) / 2.0 : rect.left();
    p.drawStaticText(QPointF(left, rect.top() + (rect.height() - QFontMetricsF(font).height()) / 2.0), laid);
}

QPixmap TimelineWidget::iconPixmap(const QString &name, const QColor &color, qreal px, qreal dpr) {
    const QString key = name + QLatin1Char('|') + QString::number(color.rgba(), 16) + QLatin1Char('|') + QString::number(px) +
                        QLatin1Char('|') + QString::number(dpr);
    auto found = m_icons.constFind(key);
    if (found == m_icons.cend()) found = m_icons.insert(key, icons::pixmap(name, color, px, dpr));
    return found.value();
}

// ---- lanes ---------------------------------------------------------------

void TimelineWidget::paintLanes(QPainter &p, const QRect &visible) {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    // Nothing outside the rect: the rest of a row may already be on screen,
    // blitted from the cache or painted for another rect.
    p.save();
    p.setClipRect(visible, Qt::IntersectClip);
    p.setRenderHint(QPainter::Antialiasing, false);
    // The well, under the tracks and below the last one.
    p.fillRect(visible, C(tk::well));
    // Only the rows the rect meets, found by binary search.
    const auto [first, last] = rowsBetween(visible.top(), visible.bottom());
    for (int row = first; row < last; ++row) {
        paintLane(p, visible, row, tracks.at(m_rows.at(row).track).toObject());
        ++m_lanesPainted;
    }
    p.restore();
}

void TimelineWidget::paintLane(QPainter &p, const QRect &visible, int row, const QJsonObject &track) {
    const TrackRow &layout = m_rows.at(row);
    const QString trackId = track.value(QStringLiteral("id")).toString();
    const QRect lane(kTimelineLabelWidth, layout.top, std::max(0, width() - kTimelineLabelWidth), layout.height);
    const double pps = pixelsPerSecond();
    // Proposal veils over the tracks each open proposal changes.
    for (const ProposalBand &band : std::as_const(m_effectiveBands)) {
        if (!band.tracks.contains(trackId)) continue;
        const QRect veil(int(std::lround(timeToX(band.start))), lane.top(), int(std::lround((band.end - band.start) * pps)), lane.height());
        if (veil.intersects(visible)) p.fillRect(veil, withAlpha(qRgb(255, 255, 255), 0.03));
    }
    p.save();
    p.setClipRect(lane.intersected(visible));
    if (!trackFlag(track, QStringLiteral("enabled"))) p.setOpacity(0.4);
    // While a proposal plays as a preview: where its clips were, dashed,
    // under the clips as it would leave them.
    const bool previewing = !m_previewId.isEmpty() && !m_showsProposedCut && m_previewTracks.contains(trackId);
    if (!m_previewId.isEmpty()) {
        for (const ProposalBand &band : std::as_const(m_effectiveBands)) {
            if (band.id != m_previewId) continue;
            for (const ProposalBand::Ghost &ghost : band.ghosts) {
                if (ghost.track != trackId) continue;
                const qreal left = std::lround(timeToX(ghost.start));
                const QRectF rect(left, layout.top + 1, std::max<qreal>(1.0, std::lround((ghost.end - ghost.start) * pps) - 1), layout.height - 2);
                if (!rect.intersects(visible)) continue;
                p.save();
                p.setRenderHint(QPainter::Antialiasing, true);
                QPainterPath path;
                path.addRoundedRect(rect, kClipRadius, kClipRadius);
                p.fillPath(path, withAlpha(qRgb(255, 255, 255), 0.03));
                QPen dashed(withAlpha(qRgb(255, 255, 255), 0.30), 1, Qt::CustomDashLine);
                dashed.setDashPattern({3, 3});
                p.setPen(dashed);
                p.setBrush(Qt::NoBrush);
                p.drawRoundedRect(rect.adjusted(0.5, 0.5, -0.5, -0.5), kClipRadius - 0.5, kClipRadius - 0.5);
                p.restore();
            }
        }
    }
    // The clip being dragged goes over its neighbours, after them.
    const QJsonArray clips = (previewing ? m_previewTracks.value(trackId) : track).value(QStringLiteral("clips")).toArray();
    QJsonObject dragged;
    for (const QJsonValue &value : clips) {
        const QJsonObject clip = value.toObject();
        if (m_dragging && clip.value(QStringLiteral("id")).toString() == m_dragClipId) dragged = clip;
        else paintClip(p, visible, row, clip, track, layout.audio, previewing);
    }
    if (!dragged.isEmpty()) paintDraggedClip(p, visible, row, dragged, track, layout.audio);
    p.restore();
    // A locked track's lane is hatched over its clips.
    if (trackFlag(track, QStringLiteral("locked"))) p.fillRect(lane.intersected(visible), QBrush(withAlpha(qRgb(255, 255, 255), 0.05), Qt::BDiagPattern));
    // Lines part the lanes; line2 parts picture from sound; the last has none.
    if (row + 1 < m_rows.size())
        p.fillRect(lane.left(), layout.bottom() - 1, lane.width(), 1, C(row == lastPictureRow() ? tk::line2 : tk::line));
}

QRectF TimelineWidget::clipRect(int row, double start, double duration) const {
    const TrackRow track = m_rows.value(row);
    const double left = std::round(timeToX(start));
    const double right = std::round(timeToX(start + duration));
    return QRectF(left, track.top + 1, std::max(1.0, right - left - 1.0), std::max(1, track.height - 2));
}

QVector<TimelineWidget::GainKey> TimelineWidget::gainKeys(const QJsonObject &clip, bool *keyed) const {
    const QJsonValue gain = clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("gain"));
    QVector<GainKey> keys;
    if (gain.isArray()) {
        for (const QJsonValue &value : gain.toArray()) {
            const QJsonObject key = value.toObject();
            const double at = key.value(QStringLiteral("at")).toDouble();
            const qint64 frame = key.contains(QStringLiteral("frame")) ? qint64(key.value(QStringLiteral("frame")).toDouble())
                                                                      : std::llround(at * m_fps);
            keys.append({at, frame, key.value(QStringLiteral("value")).toDouble(), key.value(QStringLiteral("interp")).toString(QStringLiteral("linear"))});
        }
    }
    if (keyed) *keyed = !keys.isEmpty();
    if (keys.isEmpty()) keys.append({0.0, 0, gain.toDouble(0.0), QStringLiteral("linear")});
    return keys;
}

qreal TimelineWidget::gainY(double decibels, const QRectF &clip) {
    const double fraction = (std::clamp(decibels, kGainFloor, kGainCeiling) - kGainFloor) / (kGainCeiling - kGainFloor);
    return clip.top() + 2.0 + (1.0 - fraction) * std::max<qreal>(1.0, clip.height() - 4.0);
}

double TimelineWidget::gainAt(qreal y, const QRectF &clip) {
    const double fraction = 1.0 - (y - clip.top() - 2.0) / std::max<qreal>(1.0, clip.height() - 4.0);
    const double decibels = std::clamp(kGainFloor + fraction * (kGainCeiling - kGainFloor), kGainFloor, kGainCeiling);
    return std::round(decibels * 10.0) / 10.0;
}

QPolygonF TimelineWidget::gainBand(const QString &clipId) const {
    for (int row = 0; row < m_rows.size(); ++row) {
        if (!m_rows.at(row).audio) continue;
        for (const QJsonValue &value : rowTrack(row).value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("id")).toString() != clipId) continue;
            const QRectF rect = clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(), clip.value(QStringLiteral("duration")).toDouble());
            bool keyed = false;
            QVector<GainKey> keys = gainKeys(clip, &keyed);
            if (m_band.clip == clipId)
                for (int index = 0; index < keys.size() && index < m_band.values.size(); ++index) keys[index].value = m_band.values.at(index);
            QPolygonF band;
            if (!keyed) return QPolygonF{QPointF(rect.left(), gainY(keys.first().value, rect)), QPointF(rect.right(), gainY(keys.first().value, rect))};
            for (const GainKey &key : keys) band << QPointF(rect.left() + key.at * pixelsPerSecond(), gainY(key.value, rect));
            return band;
        }
    }
    return {};
}

bool TimelineWidget::bandAt(const QPoint &point, const ClipHit &hit, int *key) const {
    if (hit.id.isEmpty() || !m_rows.value(hit.row).audio) return false;
    const QPolygonF band = gainBand(hit.id);
    if (band.isEmpty()) return false;
    bool keyed = false;
    for (const QJsonValue &value : rowTrack(hit.row).value(QStringLiteral("clips")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == hit.id) gainKeys(value.toObject(), &keyed);
    if (keyed)
        for (int index = 0; index < band.size(); ++index)
            if (QLineF(band.at(index), point).length() <= 5.0) {
                *key = index;
                return true;
            }
    // The line's height at the pointer: flat before the first key and after the last.
    qreal y = band.first().y();
    if (point.x() >= band.last().x()) y = band.last().y();
    else
        for (int index = 1; index < band.size(); ++index) {
            if (point.x() > band.at(index).x()) continue;
            const QPointF a = band.at(index - 1);
            const QPointF b = band.at(index);
            y = b.x() > a.x() && point.x() >= a.x() ? a.y() + (b.y() - a.y()) * (point.x() - a.x()) / (b.x() - a.x()) : a.y();
            break;
        }
    if (std::abs(point.y() - y) > 3.0) return false;
    *key = -1;
    return true;
}

void TimelineWidget::beginBandDrag(const ClipHit &hit, int key, qreal y, bool addKey, double time) {
    QJsonObject clip;
    for (const QJsonValue &value : rowTrack(hit.row).value(QStringLiteral("clips")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == hit.id) clip = value.toObject();
    bool keyed = false;
    QVector<GainKey> keys = gainKeys(clip, &keyed);
    if (addKey) {
        // Option-click on the line adds a key there, at the level it reads.
        const qint64 frame = std::llround(std::clamp(time, 0.0, hit.duration) * m_fps);
        const double level = gainAt(y, hit.rect);
        if (!keyed) keys.clear();
        const auto at = std::find_if(keys.begin(), keys.end(), [frame](const GainKey &k) { return k.frame >= frame; });
        key = int(at - keys.begin());
        if (at == keys.end() || at->frame != frame) keys.insert(at, GainKey{double(frame) / m_fps, frame, level, QStringLiteral("linear")});
        keyed = true;
    }
    m_band = BandDrag{};
    m_band.clip = hit.id;
    m_band.row = hit.row;
    m_band.key = keyed ? key : -1;
    m_band.pressY = y;
    m_band.keyed = keyed;
    m_band.moved = addKey;
    // The keys as they will be written, values the drag changes.
    m_band.from.clear();
    for (const GainKey &k : std::as_const(keys)) m_band.from << k.value;
    m_band.values = m_band.from;
    m_snapPoints.clear();
    // The keys' frames and interpolation ride along in the clip's props.
    if (addKey) {
        QJsonArray written;
        for (const GainKey &k : std::as_const(keys))
            written.append(QJsonObject{{QStringLiteral("at"), k.at}, {QStringLiteral("frame"), k.frame},
                                       {QStringLiteral("value"), k.value}, {QStringLiteral("interp"), k.interp}});
        // Stand the new key in the project, so the band draws it while dragging.
        QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
        QJsonObject track = tracks.at(m_rows.at(hit.row).track).toObject();
        QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
        for (int index = 0; index < clips.size(); ++index) {
            QJsonObject entry = clips.at(index).toObject();
            if (entry.value(QStringLiteral("id")).toString() != hit.id) continue;
            QJsonObject props = entry.value(QStringLiteral("props")).toObject();
            props.insert(QStringLiteral("gain"), written);
            entry.insert(QStringLiteral("props"), props);
            clips.replace(index, entry);
        }
        track.insert(QStringLiteral("clips"), clips);
        tracks.replace(m_rows.at(hit.row).track, track);
        m_project.insert(QStringLiteral("tracks"), tracks);
    }
    setCursor(Qt::SizeVerCursor);
    refreshLanes(hit.rect.toAlignedRect());
}

void TimelineWidget::finishBandDrag() {
    const BandDrag band = std::exchange(m_band, BandDrag{});
    QJsonObject clip;
    for (const QJsonValue &value : rowTrack(band.row).value(QStringLiteral("clips")).toArray())
        if (value.toObject().value(QStringLiteral("id")).toString() == band.clip) clip = value.toObject();
    refreshLanes(clipBounds(band.clip));
    if (!band.moved || clip.isEmpty()) return;
    bool keyed = false;
    const QVector<GainKey> keys = gainKeys(clip, &keyed);
    if (!keyed) {
        emit gainRequested(band.clip, band.values.value(0));
        return;
    }
    QJsonArray written;
    for (int index = 0; index < keys.size(); ++index)
        written.append(QJsonObject{{QStringLiteral("at"), keys.at(index).frame},
                                   {QStringLiteral("value"), band.values.value(index, keys.at(index).value)},
                                   {QStringLiteral("interp"), keys.at(index).interp}});
    emit gainRequested(band.clip, QJsonObject{{QStringLiteral("keys"), written}});
}

void TimelineWidget::paintNameBar(QPainter &p, const QRectF &bar, const QString &name, const QString &secondary, bool qc, bool fx,
                                  bool shadow, const QColor &ink) {
    qreal x = bar.left() + 6.0;
    qreal right = bar.right() - 6.0;
    const qreal mid = bar.center().y();
    const qreal dpr = p.device()->devicePixelRatioF();
    if (fx && right - x > 34.0) {
        const QPixmap glyph = iconPixmap(QStringLiteral("g-fx"), C(tk::ink2), 14, dpr);
        p.drawPixmap(QPointF(right - 14.0, std::round(mid - 7.0)), glyph);
        right -= 14.0 + 6.0;
    }
    if (qc && right - x > 12.0) {
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setPen(Qt::NoPen);
        p.setBrush(C(tk::neg));
        p.drawEllipse(QRectF(x, mid - 3.0, 6.0, 6.0));
        p.restore();
        x += 6.0 + 6.0;
    }
    if (right - x < 4.0) return;
    const QFont &font = metaFont();
    const QStaticText laid = laidName(font, name, right - x);
    const qreal top = std::round(mid - QFontMetricsF(font).height() / 2.0);
    p.setFont(font);
    if (shadow) {
        p.setPen(withAlpha(qRgb(0, 0, 0), 0.8));
        p.drawStaticText(QPointF(x, top + 1), laid);
    }
    p.setPen(ink);
    p.drawStaticText(QPointF(x, top), laid);
    x += laid.size().width() + 6.0;
    if (secondary.isEmpty()) return;
    const qreal room = right - x;
    const qreal full = QFontMetricsF(font).horizontalAdvance(secondary);
    // Left out whole when it would be cut to less than 44.
    if (room < full && room < kSecondaryMin) return;
    const QStaticText rest = laidName(font, secondary, std::min(room, full + 1.0));
    if (shadow) {
        p.setPen(withAlpha(qRgb(0, 0, 0), 0.8));
        p.drawStaticText(QPointF(x, top + 1), rest);
    }
    p.setPen(C(tk::ink3));
    p.drawStaticText(QPointF(x, top), rest);
}

void TimelineWidget::paintGainBand(QPainter &p, const QRectF &rect, const QJsonObject &clip) {
    const QString id = clip.value(QStringLiteral("id")).toString();
    bool keyed = false;
    QVector<GainKey> keys = gainKeys(clip, &keyed);
    // A level at 0 dB without keys shows only for the clip under the
    // pointer or selected, where it can be dragged.
    const bool shown = keyed || std::abs(keys.first().value) > 0.05 || id == m_hoverClipId || isSelected(id) || m_band.clip == id;
    if (!shown) return;
    if (m_band.clip == id)
        for (int index = 0; index < keys.size() && index < m_band.values.size(); ++index) keys[index].value = m_band.values.at(index);
    QPolygonF line;
    line << QPointF(rect.left(), gainY(keys.first().value, rect));
    if (keyed)
        for (const GainKey &key : std::as_const(keys)) line << QPointF(rect.left() + key.at * pixelsPerSecond(), gainY(key.value, rect));
    line << QPointF(rect.right(), gainY(keys.last().value, rect));
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(QPen(C(tk::overlay::rubberBand), 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawPolyline(line);
    if (keyed) {
        p.setPen(Qt::NoPen);
        p.setBrush(C(tk::overlay::keyframe));
        for (int index = 1; index + 1 < line.size(); ++index) p.drawPolygon(diamond(line.at(index)));
    }
    p.restore();
}

void TimelineWidget::paintClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track, bool audio,
                               bool proposed) {
    double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    double duration = clip.value(QStringLiteral("duration")).toDouble();
    const QString id = clip.value(QStringLiteral("id")).toString();
    const bool dragged = m_dragging && id == m_dragClipId;
    if (dragged) {
        if (m_dragLeading) {
            duration = std::max(1.0 / m_fps, m_dragStart + m_dragDuration - m_previewTime);
            start = m_previewTime;
        } else if (m_dragTrailing) {
            duration = std::max(1.0 / m_fps, m_previewTime - m_dragStart);
        } else if (m_tool != TimelineTool::Slip) {
            start = m_previewTime;
        }
    }
    const QRectF rect = clipRect(row, start, duration);
    if (!rect.intersects(visible)) return;
    const ClipKind kind = clipKind(track, clip);
    const bool card = kind == ClipKind::Generator || kind == ClipKind::Caption;
    const bool selected = isSelected(id) && !proposed;
    const bool hovered = id == m_hoverClipId && !proposed;
    const bool qc = m_qcClips.contains(id);
    const bool fx = !clip.value(QStringLiteral("effects")).toArray().isEmpty();
    const QString name = clipName(clip);
    // What the clip has to show: its sound's peaks, or frames along its
    // picture (its poster alone until they arrive).
    const QString sourceId = clip.value(QStringLiteral("sourceId")).toString();
    const AudioPeaks *peaks = nullptr;
    const Filmstrip *strip = nullptr;
    const QImage *poster = nullptr;
    if (m_artwork && !sourceId.isEmpty() && !card) {
        if (audio) {
            const auto found = m_artwork->peaks.constFind(sourceId);
            if (found != m_artwork->peaks.cend() && !found->isEmpty()) peaks = &found.value();
        } else {
            const auto found = m_artwork->strips.constFind(sourceId);
            if (found != m_artwork->strips.cend() && !found->isEmpty()) strip = &found.value();
            const auto frame = m_artwork->frames.constFind(sourceId);
            if (frame != m_artwork->frames.cend() && !frame->isNull()) poster = &frame.value();
        }
    }
    QPainterPath shape;
    shape.addRoundedRect(rect, kClipRadius, kClipRadius);
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.fillPath(shape, C(card ? tk::raised : tk::card));
    p.setRenderHint(QPainter::Antialiasing, false);
    p.setClipPath(shape, Qt::IntersectClip);
    // The source seconds under the clip's left edge and its right: a leading
    // trim being dragged moves its in point with its start.
    const double headShift = dragged && m_dragLeading ? start - m_dragStart : 0.0;
    const auto sourceAt = [&clip, headShift](double offset) { return clipSourceTime(clip, offset + headShift); };
    const double pps = pixelsPerSecond();
    if (!card && !audio && (strip || poster)) {
        // A filmstrip at the clip's height: 16:9 frames with a 1 pt black
        // gap between them, each the frame under its left edge.
        p.fillRect(rect, Qt::black);
        const double tileHeight = rect.height();
        const double tileWidth = std::max(6.0, std::round(tileHeight * 16.0 / 9.0));
        const double step = tileWidth + 1.0;
        const qreal ratio = p.device()->devicePixelRatioF();
        const double first = rect.left() + std::max(0.0, std::floor((visible.left() - rect.left()) / step)) * step;
        const double stop = std::min(rect.right(), double(visible.right()) + 1.0);
        for (double x = first; x < stop; x += step) {
            const int frame = strip ? strip->frameAt(sourceAt((x - rect.left()) / pps)) : -1;
            p.drawImage(QPointF(x, rect.top()), filmstripTile(sourceId, frame, QSize(int(tileWidth), int(tileHeight)), ratio));
        }
    } else if (audio && peaks) {
        // Peak and RMS, mirrored about the middle; room tone in a darker grey.
        const bool room = isRoomTone(track);
        const QJsonValue gain = clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("gain"));
        drawWaveform(p, rect.adjusted(0, 1, 0, -1), *peaks, sourceAt(0.0), sourceAt(duration),
                     C(room ? tk::overlay::roomTonePeak : tk::overlay::waveformPeak),
                     C(room ? tk::overlay::roomToneRms : tk::overlay::waveformRms),
                     gain.isDouble() ? std::pow(10.0, gain.toDouble() / 20.0) : 1.0, visible);
    }
    if (kind == ClipKind::Caption) {
        // A caption's text, centred, with its QC dot.
        QString text = clip.value(QStringLiteral("text")).toString();
        if (text.isEmpty()) text = clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("text")).toString();
        if (text.isEmpty()) text = name;
        if (rect.width() >= 16.0)
            paintNameBar(p, QRectF(rect.left(), rect.top() + std::floor((rect.height() - kNameBarHeight) / 2.0), rect.width(), kNameBarHeight),
                         text.simplified(), {}, qc, false, false, C(tk::ink2));
    } else if (kind == ClipKind::Generator) {
        if (rect.width() >= kNameBarFrom)
            paintNameBar(p, QRectF(rect.left(), rect.top() + std::floor((rect.height() - kNameBarHeight) / 2.0), rect.width(), kNameBarHeight),
                         name, clipSecondary(clip, kind), qc, fx, false, C(tk::ink));
    } else if (rect.width() >= kNameBarFrom) {
        const QRectF bar(rect.left(), rect.top(), rect.width(), std::min(kNameBarHeight, rect.height()));
        if (!audio) p.fillRect(bar, withAlpha(qRgb(0, 0, 0), 0.55));
        paintNameBar(p, bar, name, clipSecondary(clip, kind), qc, fx, audio, C(tk::ink));
    }
    p.restore();
    if (audio && !card) paintGainBand(p, rect, clip);
    // Its outline: line2; the accent when an open proposal changes it; 1.5 ink when selected.
    if (selected) ui::paintInsetLine(p, rect, kClipRadius, C(tk::ink), 1.5);
    else ui::paintInsetLine(p, rect, kClipRadius, C(movedByProposal(id) ? tk::accent : tk::line2), 1.0);
    // The edge under the pointer, or the edge being trimmed.
    int edge = 0;
    if (dragged) edge = m_dragLeading ? -1 : m_dragTrailing ? 1 : 0;
    else if (hovered && !m_dragging) edge = m_hoverEdge;
    if (edge != 0 && trimsEdges()) {
        p.save();
        p.setClipPath(shape, Qt::IntersectClip);
        p.fillRect(QRectF(edge < 0 ? rect.left() : rect.right() - 2.0, rect.top(), 2.0, rect.height()), C(tk::ink));
        p.restore();
    }
}

void TimelineWidget::paintDraggedClip(QPainter &p, const QRect &visible, int row, const QJsonObject &clip, const QJsonObject &track,
                                      bool audio) {
    paintClip(p, visible, row, clip, track, audio, false);
    // Where it will overwrite its neighbours, a dashed edge inside it.
    double start = clip.value(QStringLiteral("timelineStart")).toDouble();
    double duration = clip.value(QStringLiteral("duration")).toDouble();
    if (m_dragLeading) {
        duration = std::max(1.0 / m_fps, m_dragStart + m_dragDuration - m_previewTime);
        start = m_previewTime;
    } else if (m_dragTrailing) {
        duration = std::max(1.0 / m_fps, m_previewTime - m_dragStart);
    } else if (m_tool != TimelineTool::Slip) {
        start = m_previewTime;
    }
    const QRectF rect = clipRect(row, start, duration);
    const QString id = clip.value(QStringLiteral("id")).toString();
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.setBrush(Qt::NoBrush);
    for (const QJsonValue &value : track.value(QStringLiteral("clips")).toArray()) {
        const QJsonObject other = value.toObject();
        if (other.value(QStringLiteral("id")).toString() == id) continue;
        const QRectF overlap = rect.intersected(clipRect(row, other.value(QStringLiteral("timelineStart")).toDouble(),
                                                         other.value(QStringLiteral("duration")).toDouble()));
        if (overlap.width() < 6.0) continue;
        p.setPen(QPen(C(tk::ink), 1, Qt::DashLine));
        p.drawRect(overlap.adjusted(2.5, 2.5, -2.5, -2.5));
    }
    p.restore();
}

// ---- ruler, headers, overlay ---------------------------------------------

QRectF TimelineWidget::markerRect(const QJsonObject &marker, int top) const {
    // The flag over its frame; a duration reaches on to its end.
    const double at = marker.value(QStringLiteral("time")).toDouble();
    const qreal x = std::round(timeToX(at));
    const qreal end = std::round(timeToX(at + marker.value(QStringLiteral("duration")).toDouble()));
    return QRectF(x - 5.0, top + 1.0, std::max<qreal>(end - x, 0.0) + 10.0, 10.0);
}

QRectF TimelineWidget::commentMarkRect(const CommentMark &mark, int top) const {
    const qreal x = std::round(timeToX(mark.at));
    const qreal end = mark.ranged ? std::round(timeToX(mark.end)) : x;
    return QRectF(x - 5.0, top + 1.0, std::max<qreal>(end - x, 0.0) + 10.0, 10.0);
}

QRectF TimelineWidget::bandRect(const ProposalBand &band, int top) const {
    return QRectF(std::round(timeToX(band.start)), top, std::round((band.end - band.start) * pixelsPerSecond()), kRulerMarkerRow);
}

int TimelineWidget::commentMarkAt(const QPoint &point) const {
    const int top = stickyOrigin().y();
    // The dot first (the nearest), then a range's bar.
    int best = -1;
    qreal nearest = 1e9;
    for (int index = 0; index < m_marks.size(); ++index) {
        const qreal x = std::round(timeToX(m_marks.at(index).at));
        const QRectF dot(x - 5.0, top + 1.0, 10.0, 10.0);
        if (dot.adjusted(-1, -1, 1, 1).contains(point) && std::abs(point.x() - x) < nearest) {
            nearest = std::abs(point.x() - x);
            best = index;
        }
    }
    if (best >= 0) return best;
    for (int index = 0; index < m_marks.size(); ++index)
        if (m_marks.at(index).ranged && commentMarkRect(m_marks.at(index), top).contains(point)) return index;
    return -1;
}

int TimelineWidget::bandAt(const QPoint &point) const {
    for (int index = 0; index < m_effectiveBands.size(); ++index)
        if (bandRect(m_effectiveBands.at(index), stickyOrigin().y()).contains(point)) return index;
    return -1;
}

void TimelineWidget::paintMarkerRow(QPainter &p, const QRect &dirty, int top) {
    p.save();
    // Proposal bands under everything.
    for (const ProposalBand &band : std::as_const(m_effectiveBands)) {
        const QRectF rect = bandRect(band, top);
        if (rect.toAlignedRect().intersects(dirty)) p.fillRect(rect, withAlpha(qRgb(255, 255, 255), 0.06));
    }
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    const double pps = pixelsPerSecond();
    for (const CommentMark &mark : std::as_const(m_marks)) {
        const qreal x = std::round(timeToX(mark.at));
        if (mark.ranged) {
            const QRectF bar(x, top + 5.0, std::round((mark.end - mark.at) * pps), 2.0);
            if (bar.toAlignedRect().adjusted(-1, -1, 1, 1).intersects(dirty)) {
                p.setOpacity(mark.resolved ? 0.35 : 1.0);
                p.setBrush(C(mark.current ? tk::accent : tk::ink3));
                p.drawRoundedRect(bar, 1.0, 1.0);
            }
        }
        const QRectF dot(x - 5.0, top + 1.0, 10.0, 10.0);
        if (!dot.toAlignedRect().adjusted(-1, -1, 1, 1).intersects(dirty)) continue;
        p.setOpacity(mark.resolved ? 0.4 : 1.0);
        p.setBrush(C(mark.current ? tk::ink : tk::ink3));
        if (mark.circle) p.drawEllipse(dot);
        else p.drawRoundedRect(dot, 3.0, 3.0);
    }
    p.setOpacity(1.0);
    // Sequence markers: a flag on their frame.
    p.setBrush(C(tk::ink2));
    for (const QJsonValue &value : m_project.value(QStringLiteral("markers")).toArray()) {
        const QRectF rect = markerRect(value.toObject(), top);
        if (!rect.toAlignedRect().intersects(dirty)) continue;
        p.drawPolygon(markerFlag(rect.topLeft()));
    }
    p.restore();
}

void TimelineWidget::paintRuler(QPainter &p, const QRect &dirty, int top) {
    const QRect band(dirty.left(), top, dirty.width(), kRulerHeight);
    if (!band.intersects(dirty)) return;
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(band, C(tk::chrome));
    p.fillRect(QRect(band.left(), top + kRulerHeight - 1, band.width(), 1), C(tk::line));
    const double pps = pixelsPerSecond();
    const RulerTicks ticks = rulerTicks(pps, m_fps);
    const qint64 lastFrame = std::llround(m_duration * m_fps);
    const auto xOf = [this](qint64 frame) { return static_cast<int>(std::lround(timeToX(double(frame) / m_fps))); };
    // From a label's width before the strip, so a label that reaches into it is drawn.
    const double fromTime = std::max(0.0, (dirty.left() - 60 - kTimelineOrigin) / pps);
    qint64 frame = std::max<qint64>(0, qint64(std::floor(fromTime * m_fps / double(ticks.minorFrames))) * ticks.minorFrames);
    const qint64 untilFrame = std::min(lastFrame, qint64(std::ceil((dirty.right() + 1 - kTimelineOrigin) / pps * m_fps)) + ticks.minorFrames);
    const QFont &font = monoSmallFont();
    const QFontMetrics metrics(font);
    p.setFont(font);
    const int playX = static_cast<int>(std::floor(timeToX(m_playhead)));
    int labelX = INT_MIN;
    int labelEnd = INT_MIN;
    for (; frame <= untilFrame; frame += ticks.minorFrames) {
        const int x = xOf(frame);
        if (frame % ticks.labelFrames == 0) {
            labelX = x;
            labelEnd = x;
            if (frame < lastFrame) {
                const QString text = rulerLabel(frame, m_fps);
                const int w = metrics.horizontalAdvance(text);
                labelEnd = x + 4 + w + 4;
                // Faded out within 24 of the playhead.
                if (std::abs(x + 4 + w / 2 - playX) >= 24) {
                    p.setPen(C(tk::ink3));
                    p.drawText(QRect(x + 4, top + 13, w + 2, 16), Qt::AlignLeft | Qt::AlignVCenter, text);
                }
            }
        }
        if (frame % ticks.majorFrames == 0) {
            p.fillRect(QRect(x, top + kRulerHeight - 8, 1, 8), C(tk::track2));
        } else if (!(x > labelX && x < labelEnd)) {
            // Minor ticks stay out from under a label.
            p.fillRect(QRect(x, top + kRulerHeight - 4, 1, 4), C(tk::track));
        }
    }
    paintMarkerRow(p, dirty, top);
    // The program's in and out marks: the range between them, and each mark's line.
    if (!std::isnan(m_markIn) || !std::isnan(m_markOut)) {
        const int from = int(std::lround(timeToX(std::isnan(m_markIn) ? 0.0 : m_markIn)));
        const int to = int(std::lround(timeToX(std::isnan(m_markOut) ? m_duration : m_markOut)));
        p.fillRect(QRect(from, top + kRulerHeight - 4, std::max(0, to - from), 3), withAlpha(qRgb(255, 255, 255), 0.20));
        if (!std::isnan(m_markIn)) p.fillRect(QRect(from, top + kRulerHeight - 10, 1, 10), C(tk::ink));
        if (!std::isnan(m_markOut)) p.fillRect(QRect(to, top + kRulerHeight - 10, 1, 10), C(tk::ink));
    }
}

void TimelineWidget::paintHeaders(QPainter &p, const QRect &dirty) {
    const QRect column = headerColumnRect();
    if (!column.intersects(dirty)) return;
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    p.save();
    p.setClipRect(column, Qt::IntersectClip);
    p.fillRect(column, C(tk::chrome));
    const auto [first, last] = rowsBetween(std::max(column.top(), dirty.top()), std::min(column.bottom(), dirty.bottom()));
    const QStringList targets = targetTracks();
    for (int row = first; row < last; ++row) paintTrackHeader(p, row, tracks.at(m_rows.at(row).track).toObject(), column.left(), targets);
    const int grip = m_resizeRow >= 0 ? m_resizeRow : m_hoverGrip;
    if (grip >= 0 && grip < m_rows.size()) p.fillRect(column.left(), m_rows.at(grip).bottom() - 2, column.width() - 1, 2, C(tk::track2));
    if (m_addLaneVisible) {
        const QRect button = addLaneButtonRect();
        p.setRenderHint(QPainter::Antialiasing, true);
        QPainterPath path;
        path.addRoundedRect(QRectF(button), tk::radius::control, tk::radius::control);
        p.fillPath(path, C(tk::raised));
        const qreal dpr = p.device()->devicePixelRatioF();
        const QString label = tr("Add track");
        const QFont font = tk::font(tk::control);
        const int textWidth = QFontMetrics(font).horizontalAdvance(label);
        const int left = button.left() + (button.width() - 16 - 4 - textWidth) / 2;
        p.drawPixmap(QPointF(left, button.top() + 4), iconPixmap(QStringLiteral("g-addtrack"), C(tk::ink), 16, dpr));
        p.setFont(font);
        p.setPen(C(tk::ink));
        p.drawText(QRect(left + 20, button.top(), textWidth + 2, button.height()), Qt::AlignLeft | Qt::AlignVCenter, label);
    }
    // The column's right line.
    p.fillRect(column.left() + kTimelineLabelWidth - 1, column.top(), 1, column.height(), C(tk::line));
    p.restore();
}

void TimelineWidget::paintTrackHeader(QPainter &p, int row, const QJsonObject &track, int left, const QStringList &targets) {
    const TrackRow &layout = m_rows.at(row);
    const QString id = track.value(QStringLiteral("id")).toString();
    const qreal dpr = p.device()->devicePixelRatioF();
    // Its bottom line: line2 under the last picture track.
    p.fillRect(QRect(left, layout.bottom() - 1, kTimelineLabelWidth, 1), C(row == lastPictureRow() ? tk::line2 : tk::line));
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    const auto box = [&](const QRect &rect, const QColor &fill, const QColor &outline) {
        QPainterPath path;
        path.addRoundedRect(QRectF(rect), tk::radius::small, tk::radius::small);
        if (fill.alpha() > 0) p.fillPath(path, fill);
        if (outline.alpha() > 0) ui::paintInsetLine(p, QRectF(rect), tk::radius::small, outline, 1.0);
    };
    const auto hovered = [&](HeaderControl control) { return m_hoverControl == row * 8 + int(control); };
    // The target: the track's id, held on when targeted.
    {
        const QRect rect = headerControlRect(row, HeaderControl::Target).translated(left - stickyOrigin().x(), 0);
        const bool targeted = targets.contains(id);
        if (targeted) box(rect, C(tk::on), withAlpha(qRgb(255, 255, 255), 0.20));
        else box(rect, hovered(HeaderControl::Target) ? C(tk::raised) : QColor(Qt::transparent), C(tk::line));
        p.setFont(targeted ? monoSmallStrongFont() : monoSmallFont());
        p.setPen(C(targeted || hovered(HeaderControl::Target) ? tk::ink : tk::ink3));
        p.drawText(rect.adjusted(0, 2, 0, 0), Qt::AlignHCenter | Qt::AlignTop, trackRowLabel(row));
    }
    // The name, to the head's end; on hover, to the toggles. A muted or hidden
    // track reads dim, so its state shows with the toggles out of sight.
    const bool hoverRow = row == m_hoverRow;
    QString name = track.value(QStringLiteral("name")).toString().trimmed();
    if (name.isEmpty()) name = isCaptionTrack(track) ? tr("Captions") : (layout.audio ? tr("Audio %1") : tr("Video %1")).arg(layout.number);
    const bool enabled = trackFlag(track, QStringLiteral("enabled"));
    p.setPen(C(enabled && !(layout.audio && trackFlag(track, QStringLiteral("muted"))) ? tk::ink : tk::ink3));
    const int nameLeft = left + kNameX;
    const int nameRight = hoverRow ? left + (layout.audio ? kTogglesX : kTogglesX + kToggleStep) : left + kTimelineLabelWidth - 6;
    drawName(p, QRectF(nameLeft, layout.top, nameRight - nameLeft, layout.height - 1), trackNameFont(), name);
    // The toggles: 20 pt, ink3; raised and ink under the pointer; on and ink when on.
    const auto toggle = [&](HeaderControl control, bool on, const QString &icon, const QString &text) {
        const QRect rect = headerControlRect(row, control).translated(left - stickyOrigin().x(), 0);
        if (rect.isEmpty()) return;
        const bool hover = hovered(control);
        box(rect, on ? C(tk::on) : hover ? C(tk::raised) : QColor(Qt::transparent), Qt::transparent);
        const QColor ink = C(on || hover ? tk::ink : tk::ink3);
        if (!icon.isEmpty()) {
            p.drawPixmap(QPointF(rect.left() + 3, rect.top() + 3), iconPixmap(icon, ink, 14, dpr));
        } else {
            p.setFont(toggleFont());
            p.setPen(ink);
            p.drawText(rect, Qt::AlignCenter, text);
        }
    };
    if (hoverRow) {
        const bool locked = trackFlag(track, QStringLiteral("locked"));
        if (layout.audio) {
            toggle(HeaderControl::Lock, locked, locked ? QStringLiteral("g-lock") : QStringLiteral("g-unlock"), {});
            toggle(HeaderControl::Output, trackFlag(track, QStringLiteral("muted")), {}, QStringLiteral("M"));
            toggle(HeaderControl::Solo, m_solo.contains(id), {}, QStringLiteral("S"));
        } else {
            toggle(HeaderControl::Lock, locked, locked ? QStringLiteral("g-lock") : QStringLiteral("g-unlock"), {});
            toggle(HeaderControl::Output, !enabled, enabled ? QStringLiteral("g-eye") : QStringLiteral("g-eyeoff"), {});
        }
    }
    p.restore();
}

void TimelineWidget::paintCorner(QPainter &p, const QPoint &origin) const {
    const QRect corner(origin.x(), origin.y(), kTimelineLabelWidth, kRulerHeight);
    p.setRenderHint(QPainter::Antialiasing, false);
    p.fillRect(corner, C(tk::chrome));
    p.fillRect(QRect(corner.left(), corner.bottom(), kTimelineLabelWidth, 1), C(tk::line));
    p.fillRect(QRect(corner.right(), corner.top(), 1, kRulerHeight), C(tk::line));
    // The playhead's time (the skimmer's while skimming, in ink2), and the rate.
    const bool skimming = m_hasSkimmer && !m_dragging && !m_scrubbing;
    const QString time = timelineTimecode(skimming ? m_skimmerTime : m_playhead, m_fps);
    const QFont &font = cornerFont();
    const int width = QFontMetrics(font).horizontalAdvance(time);
    p.setFont(font);
    p.setPen(C(skimming ? tk::ink2 : tk::ink));
    p.drawText(QRect(corner.left() + 12, corner.top() + 6, width + 2, 20), Qt::AlignLeft | Qt::AlignVCenter, time);
    p.setFont(metaFont());
    p.setPen(C(tk::ink3));
    p.drawText(QRect(corner.left() + 12 + width + 8, corner.top() + 8, 60, 16), Qt::AlignLeft | Qt::AlignVCenter,
               QString::number(m_fps, 'g', 5) + QLatin1Char('p'));
}

void TimelineWidget::paintOverlay(QPainter &p, const QRect &dirty) {
    const QPoint origin = stickyOrigin();
    const bool skimming = m_hasSkimmer && !m_dragging && !m_scrubbing;
    const int laneTop = origin.y() + kRulerHeight;
    const int playX = static_cast<int>(std::floor(timeToX(m_playhead)));
    p.setRenderHint(QPainter::Antialiasing, false);
    // The playhead, over the clips.
    p.fillRect(QRect(playX, laneTop, 1, height() - laneTop), C(tk::playhead));
    // The clip an effect card is held over.
    if (!m_dropClip.isEmpty()) {
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setPen(QPen(C(tk::accent), 1.5));
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(m_dropRect.adjusted(0.75, 0.75, -0.75, -0.75), tk::radius::small, tk::radius::small);
        p.restore();
    }
    // The edit point a drag has snapped to.
    if (m_dragging && m_dragMoved && m_snapTime >= 0.0)
        p.fillRect(QRect(static_cast<int>(std::floor(timeToX(m_snapTime))), laneTop, 1, height() - laneTop), C(tk::accentInk));
    paintRuler(p, dirty, origin.y());
    if (skimming) {
        const int skimX = static_cast<int>(std::floor(timeToX(m_skimmerTime)));
        p.fillRect(QRect(skimX, origin.y() + kRulerMarkerRow, 1, height() - origin.y() - kRulerMarkerRow), C(tk::ink2));
    }
    if (!m_focusClips.isEmpty()) {
        // What is not being looked at falls back; what is, is ringed in light.
        const QJsonObject &shown = m_previewId.isEmpty() || m_showsProposedCut ? m_project : m_previewTimeline;
        const QJsonArray tracks = shown.value(QStringLiteral("tracks")).toArray();
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        for (int row = 0; row < m_rows.size(); ++row) {
            for (const QJsonValue &value : tracks.at(m_rows.at(row).track).toObject().value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = value.toObject();
                const QRectF box = clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(), clip.value(QStringLiteral("duration")).toDouble());
                if (m_focusClips.contains(clip.value(QStringLiteral("id")).toString())) {
                    // It rises off its lane, ringed in light.
                    const QRectF lifted = box.translated(0, m_rows.at(row).audio ? 0 : -m_focusLift);
                    if (lifted != box && !m_laneCache.isNull() && m_laneCacheRect.contains(box.toRect())) {
                        const qreal ratio = m_laneCache.devicePixelRatio();
                        const QRectF source((box.left() - m_laneCacheRect.left()) * ratio, (box.top() - m_laneCacheRect.top()) * ratio, box.width() * ratio, box.height() * ratio);
                        p.setPen(Qt::NoPen);
                        p.setBrush(QColor(51, 102, 230, 70));
                        p.drawRoundedRect(lifted.adjusted(-4, -2, 4, 8), tk::radius::small + 2, tk::radius::small + 2);
                        p.drawPixmap(lifted, m_laneCache, source);
                    }
                    p.setPen(QPen(C(tk::accentInk), 2.0));
                    p.setBrush(Qt::NoBrush);
                    p.drawRoundedRect(lifted.adjusted(-1, -1, 1, 1), tk::radius::small, tk::radius::small);
                } else {
                    p.setPen(Qt::NoPen);
                    p.setBrush(QColor(12, 12, 12, 170));
                    p.drawRect(box);
                }
            }
        }
        p.restore();
    }
    if (!std::isnan(m_agentCursor)) {
        const int agentX = static_cast<int>(std::floor(timeToX(m_agentCursor)));
        const int top = origin.y() + kRulerMarkerRow;
        // Its light, either side of it.
        QLinearGradient light(agentX - 10, 0, agentX + 12, 0);
        QColor lit = C(tk::accentInk);
        lit.setAlphaF(0.0f);
        light.setColorAt(0.0, lit);
        lit.setAlphaF(0.35f);
        light.setColorAt(0.5, lit);
        lit.setAlphaF(0.0f);
        light.setColorAt(1.0, lit);
        p.fillRect(QRect(agentX - 10, top, 22, height() - top), light);
        p.fillRect(QRect(agentX, top, 2, height() - top), C(tk::accentInk));
        const QString name = QStringLiteral("Kerf");
        p.setFont(tk::font(tk::metaStrong));
        const QRect label(agentX + 2, top, p.fontMetrics().horizontalAdvance(name) + 8, 16);
        p.fillRect(label, C(tk::accentInk));
        p.setPen(C(tk::surround));
        p.drawText(label, Qt::AlignCenter, name);
    }
    // The playhead's head, 9 x 12 at the ruler's 13 (the kit's playhead-head).
    icons::paint(p, QStringLiteral("playhead-head"), QRectF(playX - 4, origin.y() + 13, 9, 12), C(tk::playhead), 12);
    paintHeaders(p, dirty);
    paintCorner(p, origin);
    if (m_dragging && m_dragMoved) paintDragReadout(p);
}

void TimelineWidget::paintDragReadout(QPainter &p) {
    const double delta = m_previewTime - dragOrigin();
    const QString text = QStringLiteral("%1  %2%3")
                             .arg(timelineTimecode(m_previewTime, m_fps), delta < 0 ? QStringLiteral("-") : QStringLiteral("+"),
                                  timelineTimecode(std::abs(delta), m_fps));
    const QFont &font = monoSmallFont();
    const QFontMetrics metrics(font);
    const qreal x = timeToX(m_previewTime);
    QRectF box(0, 0, metrics.horizontalAdvance(text) + 16, 20);
    box.moveTop(std::max(stickyOrigin().y() + kRulerHeight + 2, m_rows.value(m_dragRow).top - 22));
    box.moveLeft(x + 6);
    if (box.right() > width() - 4) box.moveRight(x - 6);
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath path;
    path.addRoundedRect(box, tk::radius::control, tk::radius::control);
    p.fillPath(path, C(tk::card));
    ui::paintInsetLine(p, box, tk::radius::control, C(tk::line2));
    p.setFont(font);
    p.setPen(C(tk::ink));
    p.drawText(box, Qt::AlignCenter, text);
    p.restore();
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
    m_laneCache = renderLanes(view);
    m_laneCacheRect = m_laneCache.isNull() ? QRect() : view;
}

QPixmap TimelineWidget::renderLanes(const QRect &area) {
    const qreal dpr = std::max<qreal>(1, devicePixelRatioF());
    // A timeline shown whole (not in a scroll area) can be far larger than
    // a screen; it paints directly.
    if (area.isEmpty() || double(area.width()) * area.height() * dpr * dpr > 12000000.0) return {};
    QPixmap pixmap(std::max(1, int(std::ceil(area.width() * dpr))), std::max(1, int(std::ceil(area.height() * dpr))));
    if (pixmap.isNull()) return {};
    pixmap.setDevicePixelRatio(dpr);
    pixmap.fill(C(tk::well));
    QPainter painter(&pixmap);
    if (!painter.isActive()) return {};
    painter.translate(-area.topLeft());
    paintLanes(painter, area);
    painter.end();
    return pixmap;
}

QImage TimelineWidget::filmstripTile(const QString &sourceId, int frame, const QSize &size, qreal dpr) {
    const FilmstripTileKey key{sourceId, frame, size, int(std::lround(dpr * 100.0))};
    const auto cached = m_filmstripTiles.constFind(key);
    if (cached != m_filmstripTiles.cend()) return cached.value();
    QImage source;
    if (frame >= 0) source = m_artwork->strips.value(sourceId).frames.value(frame);
    if (source.isNull()) source = m_artwork->frames.value(sourceId);
    if (source.isNull()) return {};
    // Pixel for pixel on the screen it is drawn on, averaged down from the
    // larger frame rather than sampled, so fine detail does not shimmer.
    // Filled, not stretched: a picture of another shape than the 16:9 tile
    // gives up its edges, keeping its middle. Rounded up, so at a
    // fractional scale (125%) a tile reaches its gap.
    const QSize target(int(std::ceil(size.width() * dpr)), int(std::ceil(size.height() * dpr)));
    QImage tile = source.scaled(target, Qt::KeepAspectRatioByExpanding, Qt::SmoothTransformation);
    if (tile.size() != target)
        tile = tile.copy((tile.width() - target.width()) / 2, (tile.height() - target.height()) / 2, target.width(), target.height());
    tile.setDevicePixelRatio(dpr);
    if (m_filmstripTileBytes + tile.sizeInBytes() > kFilmstripTileBudget) {
        m_filmstripTiles.clear();
        m_filmstripTileBytes = 0;
    }
    m_filmstripTiles.insert(key, tile);
    m_filmstripTileBytes += tile.sizeInBytes();
    return tile;
}

double TimelineWidget::clipSourceTime(const QJsonObject &clip, double offset) {
    const double duration = clip.value(QStringLiteral("duration")).toDouble();
    const double speed = clip.value(QStringLiteral("speed")).toDouble(1.0);
    const double in = clip.value(QStringLiteral("in")).toDouble();
    const double out = clip.value(QStringLiteral("out")).toDouble(in + duration * std::abs(speed));
    // From its in point towards its out, which already allow for its speed;
    // backwards from the out point when it plays in reverse (the bridge
    // sends the speed's size and a separate "reverse"; a negative speed is
    // read as reverse too).
    const bool reverse = clip.value(QStringLiteral("reverse")).toBool() || speed < 0.0;
    const double rate = duration > 0.0 ? (out - in) / duration : std::abs(speed);
    return reverse ? out - offset * rate : in + offset * rate;
}

qreal TimelineWidget::timeToX(double time) const { return kTimelineOrigin + time * pixelsPerSecond(); }

QRect TimelineWidget::clipBounds(const QString &id) const {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (int row = 0; row < m_rows.size(); ++row) {
        for (const QJsonValue &value : tracks.at(m_rows.at(row).track).toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            if (clip.value(QStringLiteral("id")).toString() == id)
                return clipRect(row, clip.value(QStringLiteral("timelineStart")).toDouble(), clip.value(QStringLiteral("duration")).toDouble())
                    .toAlignedRect();
        }
    }
    return {};
}

QRect TimelineWidget::markerBounds(const QString &id) const {
    for (const QJsonValue &value : m_project.value(QStringLiteral("markers")).toArray()) {
        const QJsonObject marker = value.toObject();
        if (marker.value(QStringLiteral("id")).toString() == id) return markerRect(marker, stickyOrigin().y()).toAlignedRect();
    }
    return {};
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

double TimelineWidget::dragOrigin() const { return m_dragTrailing && !m_dragLeading ? m_dragStart + m_dragDuration : m_dragStart; }

QRect TimelineWidget::pointerBand(qreal x) const {
    const int ix = static_cast<int>(std::lround(x));
    return QRect(ix - 120, 0, 240, std::max(1, height()));
}

void TimelineWidget::updateContentSize() {
    const double contentWidth = kTimelineOrigin + m_duration * pixelsPerSecond() + kTimelineTail;
    const int contentHeight = std::max(tracksBottom() + kAddLaneHeight, kRulerHeight + 3 * kAddLaneHeight);
    setMinimumSize(static_cast<int>(std::min(contentWidth, 16777215.0)), contentHeight);
    updateGeometry();
}

double TimelineWidget::pixelsPerSecond() const {
    return m_fitWidth > 0 ? fitScale() : 72.0 * m_zoom;
}

double TimelineWidget::timeAt(qreal x) const { return std::clamp((x - kTimelineOrigin) / pixelsPerSecond(), 0.0, m_duration); }

double TimelineWidget::scrubTime(qreal x) const { return timeAt(std::max(x, qreal(stickyOrigin().x() + kTimelineLabelWidth))); }

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
    return QRect(stickyOrigin().x() + (kTimelineLabelWidth - 104) / 2, tracksBottom() + (kAddLaneHeight - 24) / 2, 104, 24);
}

QRect TimelineWidget::addLaneHitRect() const { return QRect(stickyOrigin().x(), tracksBottom(), kTimelineLabelWidth, kAddLaneHeight); }

void TimelineWidget::updateAddLane(const QPoint &point) {
    const bool visible = addLaneHitRect().contains(point);
    if (m_addLaneVisible != visible) {
        m_addLaneVisible = visible;
        update(addLaneHitRect());
    }
    if (visible) setCursor(Qt::PointingHandCursor);
}

// ---- effect drops ------------------------------------------------------------

namespace {
const char *kFxMime = "application/x-kerf-fx";
}

void TimelineWidget::dragEnterEvent(QDragEnterEvent *event) {
    if (isEnabled() && !m_showsProposedCut && event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) event->acceptProposedAction();
}

void TimelineWidget::dragMoveEvent(QDragMoveEvent *event) {
    if (!event->mimeData()->hasFormat(QString::fromLatin1(kFxMime))) return;
    const ClipHit hit = clipAt(event->position().toPoint());
    if (hit.id != m_dropClip) {
        if (!m_dropClip.isEmpty()) update(m_dropRect.toAlignedRect().adjusted(-2, -2, 2, 2));
        m_dropClip = hit.id;
        m_dropRect = hit.rect;
        if (!m_dropClip.isEmpty()) update(m_dropRect.toAlignedRect().adjusted(-2, -2, 2, 2));
    }
    // Only a clip takes an effect.
    if (hit.id.isEmpty()) event->ignore();
    else event->acceptProposedAction();
}

void TimelineWidget::dragLeaveEvent(QDragLeaveEvent *) {
    if (m_dropClip.isEmpty()) return;
    update(m_dropRect.toAlignedRect().adjusted(-2, -2, 2, 2));
    m_dropClip.clear();
}

void TimelineWidget::dropEvent(QDropEvent *event) {
    const QString fx = QString::fromUtf8(event->mimeData()->data(QString::fromLatin1(kFxMime)));
    const ClipHit hit = clipAt(event->position().toPoint());
    if (!m_dropClip.isEmpty()) update(m_dropRect.toAlignedRect().adjusted(-2, -2, 2, 2));
    m_dropClip.clear();
    if (fx.isEmpty() || hit.id.isEmpty()) return;
    event->acceptProposedAction();
    emit effectDropped(hit.id, fx);
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
        // Its pixel of gap and its top and bottom inset still take the press.
        const QRectF rect = clipRect(row, start, duration);
        if (rect.adjusted(0, -1, 1, 1).contains(point)) return {clip.value(QStringLiteral("id")).toString(), row, start, duration, rect};
    }
    return {};
}

// ---- TimelineToolStrip ---------------------------------------------------

namespace {

// The tools the strip shows, in its order. Their shortcuts are the keymap's
// (the registry's tool.* actions), never written here.
struct ToolInfo {
    TimelineTool tool;
    const char *icon;
    const char *name;
};
constexpr ToolInfo kTools[] = {{TimelineTool::Selection, "g-sel", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Selection")},
                               {TimelineTool::Razor, "g-razor", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Razor")}};
struct TransportInfo {
    const char *object;
    const char *icon;
    const char *name;
};
constexpr TransportInfo kTransportInfo[] = {{"timelineGoToStart", "g-tostart", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Go to start")},
                                            {"timelinePrevious", "g-prev", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Previous frame")},
                                            {"timelinePlay", "g-play", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Play")},
                                            {"timelineNext", "g-next", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Next frame")},
                                            {"timelineGoToEnd", "g-toend", QT_TRANSLATE_NOOP("kerf::TimelineToolStrip", "Go to end")}};

// "name", or "name · why" while unavailable.
QString tipWithReason(const QString &name, bool enabled, const QString &reason) {
    return enabled || reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason);
}

} // namespace

int TimelineToolStrip::toolIndex(TimelineTool tool) {
    for (int index = 0; index < int(std::size(kTools)); ++index)
        if (kTools[index].tool == tool) return index;
    return -1;
}

TimelineTool TimelineToolStrip::toolAt(int index) { return kTools[std::clamp(index, 0, int(std::size(kTools)) - 1)].tool; }

TimelineToolStrip::TimelineToolStrip(TimelineWidget *timeline, QWidget *parent) : QWidget(parent), m_timeline(timeline) {
    setObjectName(QStringLiteral("timelineTools"));
    setFixedHeight(tk::layout::strip);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(12, 0, 12, 0);
    layout->setSpacing(0);
    const auto iconButton = [this](const char *object, const char *icon, const QString &name) {
        auto *button = new ui::IconButton(QString::fromLatin1(icon), ui::IconButton::Size::S24, this);
        button->setObjectName(QString::fromLatin1(object));
        button->setTip(name);
        button->setAccessibleName(name);
        return button;
    };
    // The transport first, 4 apart; Play in ink. A group, so it can move under
    // a vertical picture (setTransportHost).
    m_transportGroup = new QWidget(this);
    m_transportGroup->setObjectName(QStringLiteral("timelineTransport"));
    auto *transport = new QHBoxLayout(m_transportGroup);
    transport->setContentsMargins(0, 0, kTransportGap, 0);
    transport->setSpacing(4);
    for (int index = 0; index < kTransports; ++index) {
        const TransportInfo &info = kTransportInfo[index];
        const Transport which = Transport(index);
        m_transport[index] = iconButton(info.object, info.icon, transportName(which));
        m_transport[index]->setCheckable(false);
        connect(m_transport[index], &QAbstractButton::clicked, this, [this, which] { emit transportRequested(which); });
        m_transport[index]->setInkAtRest(which == Transport::Play);
        transport->addWidget(m_transport[index], 0, Qt::AlignVCenter);
    }
    layout->addWidget(m_transportGroup, 0, Qt::AlignVCenter);
    m_tools = new ui::SegmentedControl(this);
    m_tools->setObjectName(QStringLiteral("timelineToolSegments"));
    for (const ToolInfo &info : kTools)
        m_tools->addIconSegment(QString::fromLatin1(info.icon), QCoreApplication::translate("kerf::TimelineToolStrip", info.name));
    m_tools->setCurrentIndex(toolIndex(timeline->tool()), false);
    layout->addWidget(m_tools, 0, Qt::AlignVCenter);
    layout->addSpacing(12);
    m_snap = iconButton("timelineSnap", "g-snap", tr("Snap"));
    m_snap->setOn(timeline->snapping());
    layout->addWidget(m_snap, 0, Qt::AlignVCenter);
    layout->addStretch(1);
    // The zoom at the far end.
    const auto key = [this](bool plus) {
        auto *button = new ui::IconButton(plus ? QStringLiteral("g-zin") : QStringLiteral("g-zout"), ui::IconButton::Size::S24, this);
        button->setObjectName(plus ? QStringLiteral("timelineZoomIn") : QStringLiteral("timelineZoomOut"));
        // The shortcuts below, as this Mac writes them.
        const QKeySequence keys = (plus ? zoomInKeys() : zoomOutKeys()).constFirst();
        button->setTip(plus ? tr("Zoom in") : tr("Zoom out"), keys.toString(QKeySequence::NativeText));
        button->setAccessibleName(plus ? tr("Zoom in") : tr("Zoom out"));
        button->setCheckable(false);
        button->setAutoRepeat(true);
        return button;
    };
    m_out = key(false);
    m_slider = new ui::Slider(this);
    m_slider->setObjectName(QStringLiteral("timelineZoomSlider"));
    m_slider->setRange(0, kSteps);
    m_slider->setSingleStep(1);
    // A 62 track and the knob's overhang, flush with the buttons.
    m_slider->setFixedWidth(72);
    m_slider->setTip(tr("Timeline zoom"));
    m_slider->setAccessibleName(tr("Timeline zoom"));
    m_slider->setFocusPolicy(Qt::NoFocus);
    m_in = key(true);
    layout->addWidget(m_out, 0, Qt::AlignVCenter);
    layout->addWidget(m_slider, 0, Qt::AlignVCenter);
    layout->addWidget(m_in, 0, Qt::AlignVCenter);

    connect(m_tools, &ui::SegmentedControl::activated, this, [this](int index) {
        const TimelineTool tool = toolAt(index);
        if (isSignalConnected(QMetaMethod::fromSignal(&TimelineToolStrip::toolRequested))) emit toolRequested(tool);
        else m_timeline->setTool(tool);
        // The tool in force, whether the request was taken or not.
        m_tools->setCurrentIndex(toolIndex(m_timeline->tool()));
    });
    connect(m_snap, &QAbstractButton::clicked, this, [this](bool checked) {
        if (isSignalConnected(QMetaMethod::fromSignal(&TimelineToolStrip::snapRequested))) emit snapRequested(checked);
        else m_timeline->setSnapping(checked);
        m_snap->setOn(m_timeline->snapping());
    });
    connect(timeline, &TimelineWidget::toolChanged, this, [this](TimelineTool tool) { m_tools->setCurrentIndex(toolIndex(tool)); });
    connect(timeline, &TimelineWidget::snappingChanged, this, [this](bool on) { m_snap->setOn(on); });

    m_slider->setValue(position(timeline->scale(), timeline->minScale()));
    connect(m_slider, &ui::Slider::valueChanged, this, [this](double value) { zoomTo(int(std::lround(value))); });
    connect(m_out, &QAbstractButton::clicked, this, [this] { step(-1); });
    connect(m_in, &QAbstractButton::clicked, this, [this] { step(1); });
    // Command-= and Command-minus while the timeline (or a child) has focus.
    for (const bool plus : {true, false}) {
        auto *shortcut = new QShortcut(timeline);
        shortcut->setKeys(plus ? zoomInKeys() : zoomOutKeys());
        shortcut->setContext(Qt::WidgetWithChildrenShortcut);
        shortcut->setAutoRepeat(true);
        connect(shortcut, &QShortcut::activated, this, [this, plus] { step(plus ? 1 : -1); });
        m_shortcuts << shortcut;
    }
    connect(timeline, &TimelineWidget::scaleChanged, this, [this](double scale) {
        const QSignalBlocker blocker(m_slider);
        m_slider->setValue(position(scale, m_timeline->minScale()));
        m_slider->update();
    });
}

TimelineToolStrip::~TimelineToolStrip() {
    // The shortcuts live on the timeline, which may outlive this strip.
    for (const QPointer<QShortcut> &shortcut : std::as_const(m_shortcuts)) delete shortcut.data();
}

void TimelineToolStrip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.fillRect(rect(), C(tk::chrome));
    p.fillRect(QRect(0, height() - 1, width(), 1), C(tk::line));
}

QString TimelineToolStrip::toolName(TimelineTool tool) {
    switch (tool) {
    case TimelineTool::Selection: return tr("Selection");
    case TimelineTool::Razor: return tr("Razor");
    case TimelineTool::Ripple: return tr("Ripple");
    case TimelineTool::Roll: return tr("Roll");
    case TimelineTool::Slip: return tr("Slip");
    case TimelineTool::Slide: return tr("Slide");
    }
    return {};
}

QString TimelineToolStrip::transportName(Transport which) {
    return QCoreApplication::translate("kerf::TimelineToolStrip", kTransportInfo[int(which)].name);
}

void TimelineToolStrip::followTool(TimelineTool tool, bool enabled, const QString &reason, const QString &shortcut) {
    const int index = toolIndex(tool);
    if (index < 0) return;
    m_tools->setSegmentEnabled(index, enabled);
    m_tools->setSegmentTip(index, tipWithReason(toolName(tool), enabled, reason), shortcut);
}

void TimelineToolStrip::followSnap(bool enabled, const QString &reason, const QString &shortcut) {
    m_snap->setEnabled(enabled);
    m_snap->setTip(tipWithReason(tr("Snap"), enabled, reason), shortcut);
}

void TimelineToolStrip::setTransportHost(QBoxLayout *host) {
    auto *strip = static_cast<QBoxLayout *>(layout());
    m_transportGroup->layout()->setContentsMargins(0, 0, host ? 0 : kTransportGap, 0);
    if (host) {
        strip->removeWidget(m_transportGroup);
        host->addWidget(m_transportGroup, 0, Qt::AlignHCenter);
    } else {
        if (QLayout *held = m_transportGroup->parentWidget() == this ? nullptr : m_transportGroup->parentWidget()->layout())
            held->removeWidget(m_transportGroup);
        strip->insertWidget(0, m_transportGroup, 0, Qt::AlignVCenter);
    }
    m_transportGroup->show();
}

void TimelineToolStrip::followTransport(Transport which, bool enabled, const QString &reason, const QString &shortcut) {
    ui::IconButton *button = m_transport[int(which)];
    button->setEnabled(enabled);
    const QString name = which == Transport::Play && m_playing ? tr("Pause") : transportName(which);
    button->setTip(tipWithReason(name, enabled, reason), shortcut);
}

void TimelineToolStrip::setPlaying(bool playing) {
    if (playing == m_playing) return;
    m_playing = playing;
    ui::IconButton *play = m_transport[int(Transport::Play)];
    play->setIconName(playing ? QStringLiteral("g-pause") : QStringLiteral("g-play"));
    play->setAccessibleName(playing ? tr("Pause") : tr("Play"));
}

QList<QKeySequence> TimelineToolStrip::zoomInKeys() {
    return {QKeySequence(Qt::CTRL | Qt::Key_Equal), QKeySequence(Qt::CTRL | Qt::Key_Plus)};
}

QList<QKeySequence> TimelineToolStrip::zoomOutKeys() { return {QKeySequence(Qt::CTRL | Qt::Key_Minus)}; }

int TimelineToolStrip::fitPosition() const {
    const double fit = m_timeline->fitScale();
    return fit > 0.0 ? position(fit, m_timeline->minScale()) : -1;
}

void TimelineToolStrip::zoomTo(int at) {
    // The fit's own place on the slider is the fit: the view comes back to
    // the whole sequence, and stays fitted as the window and the sequence change.
    if (at == fitPosition()) {
        if (!m_timeline->isFitToWidth()) m_timeline->refit();
        return;
    }
    const double least = m_timeline->minScale();
    if (position(m_timeline->scale(), least) != at) m_timeline->setScale(scaleAt(at, least));
}

void TimelineToolStrip::step(int direction) {
    // From the scale in force, and stopping at the fit on the way past it.
    const int at = position(m_timeline->scale(), m_timeline->minScale());
    int next = std::clamp(at + 10 * direction, 0, kSteps);
    const int fit = fitPosition();
    if (fit >= 0 && std::min(at, next) < fit && fit < std::max(at, next)) next = fit;
    zoomTo(next);
}

int TimelineToolStrip::position(double pixelsPerSecond, double least) {
    const double span = std::log(TimelineWidget::kMaxScale / least);
    const double at = std::log(std::clamp(pixelsPerSecond, least, TimelineWidget::kMaxScale) / least);
    return int(std::lround(kSteps * at / span));
}

double TimelineToolStrip::scaleAt(int position, double least) {
    const double span = std::log(TimelineWidget::kMaxScale / least);
    return least * std::exp(span * std::clamp(position, 0, kSteps) / double(kSteps));
}

PictureTransport::PictureTransport(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("pictureTransport"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(tk::space::s12);
    layout->addStretch(1);
    m_time = new QLabel(this);
    m_time->setObjectName(QStringLiteral("pictureTransportTime"));
    m_time->setFont(tk::font(tk::timecodeLarge));
    m_time->setAlignment(Qt::AlignCenter);
    m_time->setStyleSheet(QStringLiteral("color: %1;").arg(C(tk::ink).name()));
    layout->addWidget(m_time);
    m_row = new QVBoxLayout;
    m_row->setContentsMargins(0, 0, 0, 0);
    layout->addLayout(m_row);
    layout->addStretch(1);
}

void PictureTransport::setTime(const QString &time) { m_time->setText(time); }

void PictureTransport::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.fillRect(rect(), C(tk::chrome));
}

} // namespace kerf
