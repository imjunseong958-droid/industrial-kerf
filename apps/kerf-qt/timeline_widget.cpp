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
#include <QStyleOptionSlider>
#include <QTimer>
#include <QToolTip>
#include <QUrl>
#include <QWheelEvent>

#include <algorithm>
#include <optional>
#include <climits>
#include <cmath>

namespace kerf {

namespace {

// A clip's name bar, where its name and FX badge sit (the whole clip on
// short rows). Tall enough that a selected clip's name keeps a pixel of the
// bar between its ascenders and the 3px ring.
constexpr qreal kClipHeaderHeight = 15.0;
// A clip with nothing to show has a body under its name bar from this height.
constexpr qreal kClipBodyFrom = kClipHeaderHeight + 10.0;
// A clip with a picture or sound to show is all filmstrip or waveform from
// this height, its name on a chip over the corner, as the default tracks
// draw them...
constexpr qreal kClipArtworkFrom = 19.0;
// ...until it is tall enough to give the name a bar of its own and still
// keep 29px for the artwork under it (from the Large preset up).
constexpr qreal kClipBandFrom = kClipHeaderHeight + 29.0;
// A picture clip with sound shows it in a band under its frames from this
// body height (the Large preset up): a 24px filmstrip over a 16px waveform.
constexpr qreal kSoundUnderPictureFrom = 40.0;
// The selected clip's ring, 1px dark and 2px light, inside its edge.
constexpr qreal kSelectionRing = 3.0;
// The bytes of scaled filmstrip tiles kept before the cache starts again:
// a tall track's tiles at twice the pixels run to half a megabyte each, so
// a count alone could hold gigabytes.
constexpr qsizetype kFilmstripTileBudget = 64 * 1024 * 1024;
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

// A sound clip's body is a pane of tinted glass, the same in either
// appearance: the tint (its colour, lightened) at this opacity over a fixed
// warm graphite, with the lanes' grid lines carried through it.
constexpr qreal kSoundTint = 0.6;
const QColor kSoundGlass(0x4d, 0x4a, 0x45);
const QColor kSoundGrid(255, 255, 255, 30);

// A clip's body fill: its colour, a shade lighter at the top.
QLinearGradient bodyGradient(const QRectF &body, const QColor &top, const QColor &foot) {
    QLinearGradient gradient(0, body.top(), 0, body.bottom());
    gradient.setColorAt(0, top);
    gradient.setColorAt(1, foot);
    return gradient;
}

// A clip as drawn: a pixel short of its right edge, so clips that meet
// read as two with the lane between them, as Logic's regions and Final
// Cut's clips do, with its corners rounded.
QRectF clipShape(const QRectF &rect) { return rect.width() > 6.0 ? rect.adjusted(0, 0, -1, 0) : rect; }
qreal clipRadius(const QRectF &rect) { return rect.height() >= 18.0 && rect.width() >= 12.0 ? 4.0 : 2.0; }

// A rounded line `thickness` wide inside `rect`, antialiased, so it also
// smooths the corners of what was drawn under the shape's (unsmoothed) clip.
void strokeRoundedInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color, qreal radius) {
    const qreal half = thickness / 2.0;
    p.save();
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(color, thickness));
    const qreal inner = std::max<qreal>(0.0, radius - half);
    p.drawRoundedRect(rect.adjusted(half, half, -half, -half), inner, inner);
    p.restore();
}

// The selected clip's ring: a 2px light line inside a 1px dark one, which
// reads on every clip hue and on both lane colours.
void paintSelectionRing(QPainter &p, const QRectF &rect) {
    const EditorTheme &theme = editorTheme();
    const QRectF shape = clipShape(rect);
    const qreal radius = clipRadius(rect);
    strokeRoundedInside(p, shape, 1, theme.clipSelectedRingOuter, radius);
    strokeRoundedInside(p, shape.adjusted(1, 1, -1, -1), 2, theme.clipSelectedRing, std::max<qreal>(0.0, radius - 1.0));
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

// ---- The track height keys -------------------------------------------------

namespace {

// The height buttons' well stands this far inside their 24px.
constexpr int kHeightBankInset = 2;

// One key of the track height bank in its `button`: the button's share of
// the bank's well (`bank`, in the button's coordinates, may reach past it
// to either side), then its key, 2px inside the well and 2px from its
// neighbours, and its engraved legend: S, M, L, or Fit's lanes between
// arrows.
void paintHeightKey(QPainter &painter, const QRect &button, const QRect &bank, int index, bool latched, bool hovered) {
    const EditorTheme &theme = editorTheme();
    painter.save();
    painter.setClipRect(button);
    const QRectF well = QRectF(bank).adjusted(0, kHeightBankInset, 0, -kHeightBankInset);
    paintKeyBank(painter, well);
    // 2px in from the bank's ends and 1px from a join with the next button;
    // 2px under the well's top and over its foot, which ends above its lip.
    const qreal left = button.left() <= bank.left() ? 2 : 1;
    const qreal right = button.right() >= bank.right() ? 2 : 1;
    const QRectF key(QPointF(button.left() + left, well.top() + 2), QPointF(button.right() + 1 - right, well.bottom() - 3));
    paintRaisedKey(painter, key, latched, hovered, false);
    // A raised key's face stands above its 1px drop.
    const QRectF face = latched ? key : key.adjusted(0, 0, 0, -1);
    if (index < 3) {
        static const char *const legends[] = {QT_TRANSLATE_NOOP("kerf::TimelineWidget", "S"),
                                              QT_TRANSLATE_NOOP("kerf::TimelineWidget", "M"),
                                              QT_TRANSLATE_NOOP("kerf::TimelineWidget", "L")};
        const QString legend = QCoreApplication::translate("kerf::TimelineWidget", legends[index]);
        // Tracking follows every letter, a lone one's too: set it half the
        // tracking right, so the letter itself is centred.
        const qreal tracking = engravedFont(painter.font(), legend).letterSpacing();
        drawEngraved(painter, face.translated(tracking / 2, 0), Qt::AlignCenter, legend);
    } else {
        // Fit's symbol from the header bar's, on a 12px square.
        painter.setRenderHint(QPainter::Antialiasing);
        painter.translate(face.center() - QPointF(6, 6));
        painter.scale(0.75, 0.75);
        painter.setPen(Qt::NoPen);
        painter.setBrush(theme.engravedInk);
        for (int lane = 0; lane < 3; ++lane) painter.drawRoundedRect(QRectF(7.0, 4.0 + lane * 3.0, 7.0, 2.0), 0.8, 0.8);
        painter.setBrush(Qt::NoBrush);
        painter.setPen(QPen(theme.engravedInk, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        painter.drawLine(QPointF(3.5, 2.5), QPointF(3.5, 13.5));
        painter.drawPolyline(QPolygonF{{1.5, 4.5}, {3.5, 2.5}, {5.5, 4.5}});
        painter.drawPolyline(QPolygonF{{1.5, 11.5}, {3.5, 13.5}, {5.5, 11.5}});
    }
    painter.restore();
}

} // namespace

// The dock's title strip makes a button for each height action, and makes
// them again whenever its tab changes; the timeline paints them as one bank
// of latching keys, the chosen height held in.
class TimelineWidget::HeightKeys final : public QObject {
public:
    HeightKeys(const QList<QAction *> &actions, QObject *parent) : QObject(parent), m_actions(actions) {}

    // Takes over the painting of each button for the actions it does not
    // paint yet.
    void adorn() {
        for (QAction *action : std::as_const(m_actions))
            for (QObject *object : action->associatedObjects()) {
                auto *button = qobject_cast<QToolButton *>(object);
                if (!button || button->defaultAction() != action || button->property(kAdorned).toBool()) continue;
                button->setProperty(kAdorned, true);
                button->installEventFilter(this);
                button->update();
            }
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        auto *button = event->type() == QEvent::Paint ? qobject_cast<QToolButton *>(watched) : nullptr;
        const int index = button ? int(m_actions.indexOf(button->defaultAction())) : -1;
        if (index < 0 || !button->parentWidget()) return QObject::eventFilter(watched, event);
        // The bank runs under every height button beside this one.
        QRect bank;
        for (const QToolButton *other : button->parentWidget()->findChildren<QToolButton *>(Qt::FindDirectChildrenOnly))
            if (other->isVisible() && m_actions.contains(other->defaultAction())) bank |= other->geometry();
        QPainter painter(button);
        paintHeightKey(painter, button->rect(), bank.translated(-button->pos()), index,
                       button->isChecked() || button->isDown(), button->underMouse());
        return true;
    }

private:
    static constexpr const char *kAdorned = "kerfHeightKey";
    QList<QAction *> m_actions;
};

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
    m_heightKeys = new HeightKeys(trackHeightActions(), this);
    updateHeightActions();
    // The lanes, waveforms, filmstrip tiles and symbols are drawn in
    // the old theme's colours.
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
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
    // Each linked sound clip by its link, for the gain of the sound drawn
    // under its picture clip.
    m_linkedSound.clear();
    for (const QJsonValue &track : project.value(QStringLiteral("tracks")).toArray()) {
        if (track.toObject().value(QStringLiteral("type")).toString() != QStringLiteral("audio")) continue;
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QString link = value.toObject().value(QStringLiteral("link")).toString();
            if (!link.isEmpty()) m_linkedSound.insert(link, value.toObject());
        }
    }
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
    if (m_heightKeys) m_heightKeys->adorn();
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
    // Shown, the timeline's dock has made its title strip's height buttons
    // (or makes them in this pass of the event loop) for it to paint.
    if (event->type() == QEvent::Show)
        QTimer::singleShot(0, this, [this] { m_heightKeys->adorn(); });
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
    // A strip outside the cache goes through a pixmap too, so it matches
    // the cache pixel for pixel: names drawn on a window's backing store
    // and on a pixmap can rasterise differently (FreeType on Linux).
    for (const QRect &visible : direct) {
        const QPixmap lanes = renderLanes(visible);
        if (lanes.isNull())
            paintLanes(p, visible);
        else
            blitLaneCache(p, lanes, visible, visible.topLeft());
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

QStaticText TimelineWidget::laidName(const QFont &font, const QString &text, qreal width) {
    const int whole = std::max(1, int(width));
    const QString key = QString::number(font.pixelSize()) + QLatin1Char('/') + QString::number(font.weight()) +
                        QLatin1Char('/') + QString::number(whole) + QLatin1Char('|') + text;
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
    // The ruler's major ticks carried faintly down the lane, as Logic's bar
    // lines are: they place a cut by eye, and show through a sound clip.
    {
        const double pps = pixelsPerSecond();
        const double step = rulerScale(pps, m_fps).majorSeconds;
        const QRect span = lane.intersected(visible);
        double time = std::max(0.0, std::floor(((span.left() - kTimelineOrigin) / pps) / step) * step);
        for (qreal x = std::round(timeToX(time)); x <= span.right(); time += step, x = std::round(timeToX(time)))
            if (x >= span.left() && time > 0.0) p.fillRect(QRectF(x, lane.top(), 1, lane.height()), theme.laneGrid);
    }
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
    // The track number on its channel plate, a small well pressed into the
    // header, in the ink of the track's clips.
    const QRect badge(left + 10, blockTop, m_badgeWidth, headerControlSize(layout.height));
    paintWell(p, badge, 3);
    p.setPen(enabled ? badgeInk(track) : theme.trackHeaderTextDisabled);
    // Centred over the well, clear of its lip of light.
    drawName(p, QRectF(badge).adjusted(0, 0, 0, -1), badgeFont(), trackRowLabel(row), true);

    const QJsonArray clips = track.value(QStringLiteral("clips")).toArray();
    const QRect slot = headerPreviewRect(row, left);
    bool previewed = false;
    if (detailed && m_artwork && !clips.isEmpty()) {
        const QString sourceId = clips.first().toObject().value(QStringLiteral("sourceId")).toString();
        const QRectF inside = QRectF(slot).adjusted(1, 1, -1, -1);
        const auto peaks = m_artwork->peaks.constFind(sourceId);
        const auto poster = m_artwork->frames.constFind(sourceId);
        const bool sound = layout.audio && peaks != m_artwork->peaks.cend();
        const bool picture = !layout.audio && poster != m_artwork->frames.cend() && !poster->isNull();
        previewed = sound || picture;
        if (previewed) p.fillRect(slot, sound ? style.fill : theme.headerPreview);
        if (sound) {
            // White on the sound's colour, as its clips are.
            drawWaveform(p, inside.adjusted(0, 2, 0, -2), peaks.value(), 0.0, peaks->duration(), theme.waveformPeak,
                         theme.waveformBody);
        } else if (picture) {
            // At its own shape, on the slot's colour: a vertical picture is
            // not stretched to the slot.
            const QSizeF fitted = QSizeF(poster->size()).scaled(inside.size(), Qt::KeepAspectRatio);
            const QSize size(std::max(1, int(std::round(fitted.width()))), std::max(1, int(std::round(fitted.height()))));
            const QPointF at(std::round(inside.center().x() - size.width() / 2.0), std::round(inside.center().y() - size.height() / 2.0));
            p.drawImage(at, filmstripTile(sourceId, -1, size, p.device()->devicePixelRatioF()));
        }
        if (previewed) frameInside(p, slot, 1, theme.headerPreviewBorder);
    }

    QFont nameFont = font();
    nameFont.setPixelSize(11);
    nameFont.setWeight(QFont::Medium);
    p.setFont(nameFont);
    // Output (or mute) and lock toggles sit after the track number as small
    // latching keys: raised when off, held in when on, brightening under
    // the pointer, their symbols engraved.
    const auto drawToggle = [&](int index, bool on, const QString &field) {
        const QRect box = headerToggleRect(row, index).translated(left - stickyOrigin().x(), 0);
        paintRaisedKey(p, box, on, m_hoverToggle == row * 2 + index, false);
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        // A raised key's face stands above its 1px drop. The symbols are
        // drawn for a 16px key and shrink with keys on shorter rows.
        const QRectF face = on ? QRectF(box) : QRectF(box).adjusted(0, 0, 0, -1);
        p.translate(face.center());
        const qreal scale = box.height() / 16.0;
        p.scale(scale, scale);
        p.setPen(QPen(theme.engravedInk, 1.3));
        p.setBrush(Qt::NoBrush);
        if (field == QStringLiteral("locked")) {
            p.drawArc(QRectF(-2.5, -5, 5, 6), 0, 180 * 16);
            p.drawRect(QRectF(-3.5, -2, 7, 5.5));
        } else if (field == QStringLiteral("muted")) {
            QFont mute = font();
            mute.setPixelSize(11);
            mute.setWeight(QFont::Bold);
            p.setFont(mute);
            p.drawText(QRectF(-8, -8, 16, 16), Qt::AlignCenter, QStringLiteral("M"));
        } else {
            p.drawEllipse(QPointF(), 4.5, 2.75);
            p.drawEllipse(QPointF(), 1.1, 1.1);
            if (!on) p.drawLine(QPointF(-4.5, 3.5), QPointF(4.5, -3.5));
        }
        p.restore();
    };
    if (layout.audio) drawToggle(0, track.value(QStringLiteral("muted")).toBool(), QStringLiteral("muted"));
    else drawToggle(0, enabled, QStringLiteral("enabled"));
    drawToggle(1, track.value(QStringLiteral("locked")).toBool(), QStringLiteral("locked"));
    const int nameLeft = headerToggleRect(row, 1).translated(left - stickyOrigin().x(), 0).right() + 7;
    const int nameRight = previewed ? slot.left() - 8 : header.right() - 8;
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
    badge.setPixelSize(10);
    badge.setWeight(QFont::Bold);
    return badge;
}

QColor TimelineWidget::badgeInk(const QJsonObject &track) {
    const EditorTheme &theme = editorTheme();
    return readableOn(theme.clip(trackKind(track)).header, theme.wellFill, 4.5);
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
    // What the clip has to show: its sound's peaks, or frames along its
    // picture (its poster alone until they arrive).
    const QString sourceId = clip.value(QStringLiteral("sourceId")).toString();
    const AudioPeaks *peaks = nullptr;
    const Filmstrip *strip = nullptr;
    const QImage *poster = nullptr;
    if (m_artwork && !sourceId.isEmpty()) {
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
    // A picture clip whose sound plays in a linked sound clip shows that
    // sound under its frames once it is tall enough, as a Final Cut clip
    // does (see kSoundUnderPictureFrom); once unlinked, or with the sound
    // clip gone, it shows none.
    struct SoundUnder {
        const AudioPeaks *peaks = nullptr;
        QJsonObject linked;
    };
    std::optional<SoundUnder> soundUnder;
    const QString link = clip.value(QStringLiteral("link")).toString();
    if (!audio && m_artwork && !link.isEmpty() && m_linkedSound.contains(link)) {
        const QJsonObject linked = m_linkedSound.value(link);
        const auto found = m_artwork->peaks.constFind(linked.value(QStringLiteral("sourceId")).toString());
        if (found != m_artwork->peaks.cend() && !found->isEmpty()) soundUnder = SoundUnder{&found.value(), linked};
    }
    const ClipLayout layout = clipLayout(rect, peaks || strip || poster);
    const QRectF band = layout.band;
    const QRectF body = layout.body;

    const QRectF shape = clipShape(rect);
    const qreal radius = clipRadius(rect);
    QPainterPath outline;
    outline.addRoundedRect(shape, radius, radius);
    p.save();
    p.setClipPath(outline, Qt::IntersectClip);
    if (!layout.chip) {
        QLinearGradient bandFill(0, band.top(), 0, band.bottom());
        bandFill.setColorAt(0, style.header.lighter(108));
        bandFill.setColorAt(1, style.header);
        p.fillRect(band, bandFill);
    }
    if (body.height() > 0.0) {
        // The clip's colour, a shade lighter at the top. A sound's is a
        // pane of tinted glass, as a Logic region or a Final Cut clip reads:
        // a tint behind its waveform, not a solid box (see paintSoundGlass);
        // its name bar stays solid.
        if (audio) paintSoundGlass(p, body, fill);
        else p.fillRect(body, bodyGradient(body, fill.lighter(112), fill));
    }
    // The source seconds under the clip's left edge and its right: a leading
    // trim being dragged moves its in point with its start.
    const double headShift = dragged && m_dragLeading ? start - m_dragStart : 0.0;
    const auto sourceAt = [&clip, headShift](double offset) { return clipSourceTime(clip, offset + headShift); };
    if (body.height() > 0.0 && audio) {
        p.fillRect(QRectF(body.left(), std::floor(body.center().y()), body.width(), 1), theme.waveCenter);
        if (peaks) {
            // Clip gain is in decibels; the waveform shows what plays.
            const QJsonValue gain = clip.value(QStringLiteral("props")).toObject().value(QStringLiteral("gain"));
            drawWaveform(p, body.adjusted(0, 1, 0, -1), *peaks, sourceAt(0.0), sourceAt(duration),
                         theme.waveformPeak, theme.waveformBody, std::pow(10.0, gain.toDouble(0.0) / 20.0), visible);
        }
    } else if (body.height() > 0.0 && (strip || poster)) {
        // Its sound in a band under the frames, in the sound's colour and
        // white, with the linked sound clip's gain: picture and sound read
        // as one clip, the way Final Cut draws them.
        QRectF picture = body;
        if (soundUnder && !layout.chip && body.height() >= kSoundUnderPictureFrom) {
            const qreal soundHeight = std::max<qreal>(16.0, std::round(body.height() * 0.38));
            picture.setBottom(body.bottom() - soundHeight);
            const QRectF sound(body.left(), picture.bottom(), body.width(), soundHeight);
            // The same glass as the sound clip on its own track.
            paintSoundGlass(p, sound, theme.clipAudio.fill);
            p.fillRect(QRectF(sound.left(), std::floor(sound.center().y()), sound.width(), 1), theme.waveCenter);
            const double gain = soundUnder->linked.value(QStringLiteral("props")).toObject().value(QStringLiteral("gain")).toDouble(0.0);
            drawWaveform(p, sound.adjusted(0, 1, 0, -1), *soundUnder->peaks, sourceAt(0.0), sourceAt(duration), theme.waveformPeak,
                         theme.waveformBody, std::pow(10.0, gain / 20.0), visible);
        }
        // Tiles each the frame under its left edge (the first one the
        // clip's in point), scaled once per size. A tall picture's tile is
        // square, its middle cut out: at its own shape a 9:16 frame was a
        // sliver a few pixels wide, a stripe of the same picture repeated.
        const double aspect = strip ? strip->aspect() : double(poster->width()) / std::max(1, poster->height());
        const double tileHeight = picture.height();
        const double tileWidth = std::max(6.0, std::round(tileHeight * std::max(1.0, aspect)));
        const qreal ratio = p.device()->devicePixelRatioF();
        const double pps = pixelsPerSecond();
        const double firstTile = picture.left() + std::max(0.0, std::floor((visible.left() - picture.left()) / tileWidth)) * tileWidth;
        const double stop = std::min(picture.right(), double(visible.right()) + 1.0);
        for (double x = firstTile; x < stop; x += tileWidth) {
            const int frame = strip ? strip->frameAt(sourceAt((x - rect.left()) / pps)) : -1;
            // Frames meet without a seam, as a strip of film does on screen.
            p.drawImage(QPointF(x, picture.top()), filmstripTile(sourceId, frame, QSize(int(tileWidth), int(tileHeight)), ratio));
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
        if (layout.chip) {
            // Over the artwork, on a chip of the clip's own colour only as
            // wide as the name, so the rest of the picture or sound shows.
            // Its outer corner rounded, as a tab on the clip's edge.
            const qreal named = laidName(nameFont, label, std::max(1.0, textRight - textLeft)).size().width();
            QColor chip = style.header;
            chip.setAlphaF(0.9f);
            const QRectF tab(rect.left(), labelBand.top(), std::ceil(named) + 12.0, labelBand.height());
            QPainterPath corner;
            corner.moveTo(tab.topLeft());
            corner.lineTo(tab.topRight());
            corner.lineTo(tab.right(), tab.bottom() - 4.0);
            corner.quadTo(tab.bottomRight(), QPointF(tab.right() - 4.0, tab.bottom()));
            corner.lineTo(tab.bottomLeft());
            corner.closeSubpath();
            p.save();
            p.setRenderHint(QPainter::Antialiasing, true);
            p.fillPath(corner, chip);
            p.restore();
        }
        p.setPen(style.label);
        // Centred in the band even where a short row's band (inside a
        // selected clip's ring) is shorter than the text: the clip clips it,
        // and the ring, drawn after, covers no more than a pixel of it.
        drawName(p, QRectF(textLeft, labelBand.top(), std::max(1.0, textRight - textLeft), labelBand.height()), nameFont, label);
    }
    p.restore();

    p.fillRect(QRectF(shape.left() + radius, shape.top() + 1, std::max<qreal>(0.0, shape.width() - 2 * radius), 1), theme.clipHighlight);
    // The lane draws a selected clip's ring last, over any proposal (here
    // only for the clip being dragged, which the lane leaves out): drawn
    // twice, its smoothed corners would darken.
    // A sound clip's rim is its name bar's colour, solid round its
    // translucent body, so it reads as a pane of its colour.
    if (selected && dragged) paintSelectionRing(p, rect);
    else if (!selected) strokeRoundedInside(p, shape, 1, audio && peaks ? style.header : style.border, radius);

    // Trim bracket on the edge under the pointer, or the edge being trimmed.
    int edge = 0;
    if (dragged) edge = m_dragLeading ? -1 : m_dragTrailing ? 1 : 0;
    else if (hovered && !m_dragging) edge = m_hoverEdge;
    if (edge != 0) {
        // On the clip as drawn: inside its rounded corners, and clear of the
        // pixel of lane after it, so the bracket says whose edge it is.
        const QColor bracket = theme.clipTrim;
        p.save();
        p.setClipPath(outline, Qt::IntersectClip);
        const qreal x = edge < 0 ? shape.left() : shape.right() - 3;
        p.fillRect(QRectF(x, shape.top(), 3, shape.height()), bracket);
        const qreal serif = edge < 0 ? shape.left() : shape.right() - 8;
        p.fillRect(QRectF(serif, shape.top(), 8, 2), bracket);
        p.fillRect(QRectF(serif, shape.bottom() - 2, 8, 2), bracket);
        p.restore();
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
    {
        const qreal radius = clipRadius(rect);
        QPainterPath shadow;
        shadow.addRoundedRect(clipShape(rect).translated(0, 2), radius, radius);
        p.save();
        p.setRenderHint(QPainter::Antialiasing, true);
        p.fillPath(shadow, theme.dragShadow);
        p.restore();
    }
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
        // Against the clip as drawn, whose ring stops a pixel short of its edge.
        const QRectF overlap = clipShape(rect).intersected(clipRect(row, other.value(QStringLiteral("timelineStart")).toDouble(),
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

    p.setFont(monoFont(11));
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
    // The playhead's time in the glass's cream figures; the skimmer's in its
    // dimmer label ink, so a glance tells the two apart.
    paintCornerTime(p, origin, skimming ? m_skimmerTime : m_playhead, skimming ? theme.glassLabel : theme.glassText);
    if (m_dragging && m_dragMoved) paintDragReadout(p);
}

void TimelineWidget::paintDragReadout(QPainter &p) {
    const double delta = m_previewTime - dragOrigin();
    const QString text = QStringLiteral("%1  %2%3")
        .arg(timelineTimecode(m_previewTime, m_fps), delta < 0 ? QStringLiteral("-") : QStringLiteral("+"),
             timelineTimecode(std::abs(delta), m_fps));
    const QFont font = monoFont(11);
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
    // On a display glass sized to its figures, as a deck's counter window.
    const QString text = timelineTimecode(time, m_fps);
    const QRect glass = cornerGlassRect(origin, text);
    paintGlass(p, glass, 4);
    p.setFont(cornerFont());
    p.setPen(color);
    // Centred over the glass, clear of its lip of light.
    p.drawText(QRectF(glass).adjusted(0, 0, 0, -1), Qt::AlignCenter, text);
}

QFont TimelineWidget::cornerFont() {
    return monoFont(12, QFont::DemiBold);
}

QRect TimelineWidget::cornerGlassRect(const QPoint &origin, const QString &text) const {
    // 7px of glass either side of the figures, 18px tall in the corner's 25.
    // As wide as the widest figures could make the time, so the glass keeps
    // its size as the time runs, whatever face the figures come in.
    const QFontMetrics metrics(cornerFont());
    QChar widest = QLatin1Char('0');
    for (char digit = '1'; digit <= '9'; ++digit)
        if (metrics.horizontalAdvance(QLatin1Char(digit)) > metrics.horizontalAdvance(widest)) widest = QLatin1Char(digit);
    QString figures = text;
    for (QChar &figure : figures)
        if (figure.isDigit()) figure = widest;
    const int width = metrics.horizontalAdvance(figures) + 14;
    const int height = 18;
    return QRect(origin.x() + 8, origin.y() + (kRulerHeight - 1 - height) / 2, width, height);
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
    pixmap.fill(editorTheme().timelineBackground);
    QPainter painter(&pixmap);
    if (!painter.isActive()) return {};
    painter.translate(-area.topLeft());
    paintLanes(painter, area);
    painter.end();
    return pixmap;
}

void TimelineWidget::paintSoundGlass(QPainter &p, const QRectF &area, const QColor &fill) const {
    // The graphite behind the glass, fixed, so the pane is one colour in
    // either appearance and nothing under the clip shows through.
    p.fillRect(area, kSoundGlass);
    // The lanes' grid lines, carried on through the pane.
    const double pps = pixelsPerSecond();
    const double step = rulerScale(pps, m_fps).majorSeconds;
    double time = std::max(0.0, std::floor(((area.left() - kTimelineOrigin) / pps) / step) * step);
    for (qreal x = std::round(timeToX(time)); x <= area.right(); time += step, x = std::round(timeToX(time)))
        if (x >= area.left() && time > 0.0) p.fillRect(QRectF(x, area.top(), 1, area.height()), kSoundGrid);
    QColor top = fill.lighter(130);
    QColor foot = fill.lighter(116);
    top.setAlphaF(kSoundTint);
    foot.setAlphaF(kSoundTint);
    p.fillRect(area, bodyGradient(area, top, foot));
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
    // Filled, not stretched: a picture taller than its tile gives up its
    // top and bottom, keeping its middle.
    // Rounded up, so at a fractional scale (125%) a tile reaches the next
    // rather than leaving a column of the body showing between them.
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

TimelineWidget::ClipLayout TimelineWidget::clipLayout(const QRectF &clip, bool artwork) {
    if (artwork && clip.height() >= kClipArtworkFrom && clip.height() < kClipBandFrom)
        return {QRectF(clip.left(), clip.top(), clip.width(), kClipHeaderHeight), clip, true};
    // Short rows show only the name bar, as a mini track does.
    const qreal band = clipHeaderHeight(clip);
    return {QRectF(clip.left(), clip.top(), clip.width(), band), clip.adjusted(0, band, 0, 0), false};
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

// The fader's cap; it stands centred on the slider's handle, which the
// style lays out and a press grabs.
constexpr qreal kFaderCapWidth = 10;
constexpr qreal kFaderCapHeight = 16;

// The slider's handle, as the style lays it out from what the slider would
// give it.
QRect sliderHandle(const QSlider *slider) {
    QStyleOptionSlider option;
    option.initFrom(slider);
    option.subControls = QStyle::SC_SliderGroove | QStyle::SC_SliderHandle;
    option.orientation = slider->orientation();
    option.minimum = slider->minimum();
    option.maximum = slider->maximum();
    option.sliderPosition = slider->sliderPosition();
    option.sliderValue = slider->value();
    option.singleStep = slider->singleStep();
    option.pageStep = slider->pageStep();
    option.upsideDown = slider->invertedAppearance() != (option.direction == Qt::RightToLeft);
    return slider->style()->subControlRect(QStyle::CC_Slider, &option, QStyle::SC_SliderHandle, slider);
}

// The zoom's − and + as small raised keys, held in while pressed, their
// symbols engraved.
class ZoomKey final : public QToolButton {
public:
    ZoomKey(bool plus, QWidget *parent) : QToolButton(parent), m_plus(plus) {}

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        const bool held = isDown();
        const QRectF key = QRectF(rect()).adjusted(1, 1, -1, -1);
        paintRaisedKey(painter, key, held, underMouse() && isEnabled(), false);
        // A raised key's face stands above its 1px drop.
        const QPointF centre = (held ? key : key.adjusted(0, 0, 0, -1)).center();
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(QPen(editorTheme().engravedInk, 1.5, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(centre - QPointF(3.5, 0), centre + QPointF(3.5, 0));
        if (m_plus) painter.drawLine(centre - QPointF(0, 3.5), centre + QPointF(0, 3.5));
    }

private:
    bool m_plus;
};

// The zoom's fader: a recessed groove with a tick at each quarter of its
// travel, and a dark bakelite cap with a cream index line, standing where
// the style puts the slider's handle, so a press on the cap takes it.
class ZoomFader final : public QSlider {
public:
    explicit ZoomFader(QWidget *parent) : QSlider(Qt::Horizontal, parent) {}

protected:
    void paintEvent(QPaintEvent *) override {
        const EditorTheme &theme = editorTheme();
        QPainter painter(this);
        const QRectF cap = TimelineZoomControl::faderCap(this);
        const qreal mid = cap.center().y();
        // The cap's centre travels from half the handle's width in to half
        // its width short of the far end; the groove runs a little past both.
        const qreal from = sliderHandle(this).width() / 2.0;
        const qreal to = width() - from;
        paintWell(painter, QRectF(from - 3, mid - 3, to - from + 6, 6), 3);
        // Square ticks on whole pixels, above and below the groove.
        for (int tick = 0; tick <= 4; ++tick) {
            const qreal x = std::floor(from + (to - from) * tick / 4.0);
            painter.fillRect(QRectF(x, mid - 8, 1, 3), theme.knobTick);
            painter.fillRect(QRectF(x, mid + 5, 1, 3), theme.knobTick);
        }
        // The cap: its skirt, a face lit from above, and the index line
        // across its middle that reads against the ticks.
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        QPainterPath skirt;
        skirt.addRoundedRect(cap, 2.5, 2.5);
        painter.fillPath(skirt.translated(0, 1), theme.keyDrop);
        painter.fillPath(skirt, theme.knobRing);
        QPainterPath face;
        face.addRoundedRect(cap.adjusted(1, 1, -1, -1), 1.5, 1.5);
        QLinearGradient light(cap.topLeft(), cap.bottomLeft());
        light.setColorAt(0, theme.knobFaceLight);
        light.setColorAt(1, theme.knobFaceDark);
        painter.fillPath(face, light);
        painter.fillRect(QRectF(cap.center().x() - 1, cap.top() + 3, 2, cap.height() - 6), theme.knobPointer);
    }
};

} // namespace

TimelineZoomControl::TimelineZoomControl(TimelineWidget *timeline, QWidget *parent) : QWidget(parent), m_timeline(timeline) {
    setObjectName(QStringLiteral("timelineZoom"));
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    m_label = tr("Zoom");
    auto *layout = new QHBoxLayout(this);
    // The engraved label stands in the left margin, 6px clear of the − key.
    layout->setContentsMargins(labelRect().width() + 6, 0, 8, 0);
    layout->setSpacing(2);
    const auto key = [this](bool plus) {
        auto *button = new ZoomKey(plus, this);
        button->setObjectName(plus ? QStringLiteral("timelineZoomIn") : QStringLiteral("timelineZoomOut"));
        button->setFixedSize(20, 20);
        button->setToolTip(plus ? QStringLiteral("Zoom in") : QStringLiteral("Zoom out"));
        button->setAccessibleName(button->toolTip());
        button->setAutoRepeat(true);
        // Repainted as the pointer comes and goes.
        button->setAutoRaise(true);
        return button;
    };
    m_out = key(false);
    m_slider = new ZoomFader(this);
    m_slider->setObjectName(QStringLiteral("timelineZoomSlider"));
    m_slider->setRange(0, kSteps);
    m_slider->setFixedSize(72, 22);
    m_slider->setToolTip(QStringLiteral("Timeline zoom"));
    m_slider->setAccessibleName(QStringLiteral("Timeline zoom"));
    m_slider->setFocusPolicy(Qt::NoFocus);
    m_in = key(true);
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
    // Painted from the theme at paint time: a repaint takes the new one.
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
        for (QWidget *widget : {static_cast<QWidget *>(this), static_cast<QWidget *>(m_out), static_cast<QWidget *>(m_slider),
                                static_cast<QWidget *>(m_in)})
            widget->update();
    });
}

QRect TimelineZoomControl::labelRect() const {
    const QFont font = engravedFont(this->font(), m_label);
    const QString shown = font.capitalization() == QFont::AllUppercase ? m_label.toUpper() : m_label;
    return QRect(0, 0, int(std::ceil(QFontMetricsF(font).horizontalAdvance(shown))) + 1, height());
}

void TimelineZoomControl::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    drawEngraved(painter, labelRect(), Qt::AlignLeft | Qt::AlignVCenter, m_label);
}

QRectF TimelineZoomControl::faderCap(const QSlider *slider) {
    const QRect handle = sliderHandle(slider);
    const QPointF centre(handle.x() + handle.width() / 2.0, std::round(slider->height() / 2.0));
    // On whole pixels, so its edges and its 2px index line are crisp.
    return QRectF(std::round(centre.x() - kFaderCapWidth / 2), centre.y() - kFaderCapHeight / 2, kFaderCapWidth,
                  kFaderCapHeight);
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
