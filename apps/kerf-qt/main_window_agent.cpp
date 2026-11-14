// MainWindow's view of Kerf at work: following where it is on the timeline and
// picture, and watching only what a proposal changes.

#include "main_window.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "editor_bus.h"
#include "components.h"
#include "project_sidebar.h"
#include "review_model.h"
#include "stage_shell.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "ui/agent_hud.h"
#include "ui/change_reel.h"
#include "ui/stage_motion.h"
#include "viewer_widget.h"
#include "media_artwork.h"
#include "wiring/main_window_shared.h"
#include "wiring/review_wiring.h"
#include "wiring/stage_wiring.h"

#include <QJsonArray>
#include <QSet>
#include <QSettings>
#include <QTimer>

#include <algorithm>
#include <cmath>
#include <limits>

namespace kerf {

using namespace window_parts;

void MainWindow::buildAgentViews() {
    ChangeReel::Host host;
    host.preview = [this](const QString &proposal, double start, double duration) {
        return m_reviewWiring->previewRange(proposal, start, duration);
    };
    host.exitPreview = [this] { m_reviewWiring->preview(QString(), false); };
    host.seek = [this](double seconds, double limit) {
        m_seekLimit = limit;
        queueSeek(seconds);
    };
    host.play = [this](bool playing) {
        // Always said: a pause and a play in a row outrun m_playing.
        if (!m_connected) return;
        sendSimple(QJsonObject{{QStringLiteral("type"), playing ? QStringLiteral("play") : QStringLiteral("pause")}});
    };
    m_changeReel = new ChangeReel(std::move(host), this);
    m_reelBar = new ChangeReelBar(m_changeReel);
    m_shell->setReelBar(m_reelBar);
    connect(m_changeReel, &ChangeReel::changed, this, [this] {
        m_reelBar->update(m_playhead);
        const bool active = m_changeReel->isActive();
        if (!active) m_seekLimit = 0.0;
        // Back from watching a cut that is only a proposal, the picture is still of it.
        if (m_reelWasActive && !active && m_timeline->showsProposedCut()) {
            m_previewedCut.clear();
            m_cutPreview.start();
            if (const ReviewModel::Proposal *shown = m_reviewWiring->model()->proposal(m_proposalId); shown && !shown->changes.isEmpty())
                coverPicture(posterForClip(shown->changes.constFirst().clip));
        }
        m_reelWasActive = active;
        // The strip has its own Exit.
        quietenCutPreview();
        // The conversation falls back while the picture is looked at.
        moves::dim(m_chat, active ? tokens::motion::chatBehind : 1.0);
        if (!active) m_timeline->setFocusClips({});
        stage::Panes panes = m_shell->panes();
        panes.reel = active;
        if (active) panes.picture = true;
        m_shell->setPanes(panes);
    });
    connect(m_bus, &EditorBus::playheadChanged, this, [this](double seconds) {
        m_changeReel->followPlayhead(seconds);
        if (m_changeReel->isActive()) m_reelBar->update(seconds);
    });
    connect(m_chat, &ChatPanel::changedPartsRequested, this, &MainWindow::watchChanges);

    m_followKerf = QSettings().value(QStringLiteral("layout/followKerf"), true).toBool();

    m_motion = new StageMotion;
    m_shell->setMotion(m_motion);
    // The picture floats in the window's room.
    m_viewer->setFloating(true);
    m_timeline->setRoomyLanes(true);
    m_chat->setConversationLook(true);
    m_workChip = new WorkChip;
    m_viewer->setWorkChip(m_workChip);
    // The picture is not rescaled in each frame while the stage moves, and flights
    // wait for the timeline to have reached where they land.
    connect(m_shell, &StageShell::transitionStarted, this, [this] {
        m_viewer->setStageMoving(true);
        // The conversation is laid out once, at the width it is going to.
        m_chat->holdWidth(m_shell->rects().conversation.width());
    });
    connect(m_shell, &StageShell::transitionFinished, this, [this] {
        m_chat->holdWidth(0);
        m_viewer->setStageMoving(false);
        flushFlights();
    });
    connect(m_shell, &StageShell::pictureMoved, m_motion, &StageMotion::followPicture);
    connect(m_changeReel, &ChangeReel::sideChanging, this, [this] { m_viewer->wipeToNextFrame(); });

    m_cutPreview.setSingleShot(true);
    m_cutPreview.setInterval(tokens::motion::cutSettle);
    connect(&m_cutPreview, &QTimer::timeout, this, [this] {
        if (m_proposalId.isEmpty() || !m_timeline->showsProposedCut() || m_changeReel->isActive()) return;
        // One renders at a time; the latest follows when it is done.
        if (m_cutRendering) return;
        m_previewedCut = m_proposalId;
        m_cutRendering = m_reviewWiring->preview(m_proposalId, true);
        quietenCutPreview();
    });
    connect(m_changeReel, &ChangeReel::changed, this, &MainWindow::followReelCompare);
    connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
        if (type != QStringLiteral("effect_preview")) return;
        if (event.value(QStringLiteral("key")).toString() != QStringLiteral("reel-before:") + m_reelCompareAt || m_reelCompareAt.isEmpty()) return;
        const int width = event.value(QStringLiteral("width")).toInt();
        const int height = event.value(QStringLiteral("height")).toInt();
        const QByteArray rgba = QByteArray::fromBase64(event.value(QStringLiteral("rgba")).toString().toLatin1());
        if (width <= 0 || height <= 0 || rgba.size() < qsizetype(width) * height * 4) return;
        m_viewer->setCompareFrame(QImage(reinterpret_cast<const uchar *>(rgba.constData()), width, height, width * 4, QImage::Format_RGBA8888).copy());
    });
    // The footage gathers when its posters have come.
    connect(m_artwork, &MediaArtwork::changed, this, [this] {
        if (m_sheetWanted && m_agentBusy) showSheet();
    });
}

QRect MainWindow::toShell(const QWidget *widget, const QRect &rect) const {
    return QRect(widget->mapTo(m_shell, rect.topLeft()), rect.size());
}

// Where what comes from `source` starts: its tile in the conversation, else just
// over the composer. It has gone on from the tile from here on.
QRect MainWindow::flightStart(const QString &source) {
    FootageSheet *sheet = m_chat->footage();
    if (sheet && sheet->isVisible() && !source.isEmpty()) {
        const QRect tile = sheet->tileRect(source);
        if (!tile.isEmpty()) {
            sheet->take(source);
            return toShell(sheet, tile);
        }
    }
    const QRect conversation = m_shell->rects().conversation;
    const QRect composer = toShell(m_chat->composerWidget(), m_chat->composerWidget()->rect());
    return QRect(conversation.center().x() - 64, composer.top() - 84, 128, 72);
}

QString MainWindow::sourceOfClip(const QString &clip) const {
    const auto sourceOf = [&clip](const QJsonObject &timeline) {
        for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray())
                if (value.toObject().value(QStringLiteral("id")).toString() == clip) return value.toObject().value(QStringLiteral("sourceId")).toString();
        return QString();
    };
    const QString proposed = sourceOf(proposalTimelineFor(m_snapshot, m_proposalId));
    return proposed.isEmpty() ? sourceOf(m_project) : proposed;
}

QImage MainWindow::posterForClip(const QString &clip) const { return m_artwork->frames.value(sourceOfClip(clip)); }

// A clip goes from its tile to where the proposal puts it, and the picture becomes it.
void MainWindow::flyChange(const ReviewModel::ClipChange &change, int index) {
    const QRect lane = m_timeline->laneRect(change.track, change.newStart, change.newEnd);
    if (lane.isEmpty() || !m_timeline->isVisible()) return;
    // A picture goes to a picture's lane; a clip's sound comes along with it.
    if (m_timeline->isAudioLane(change.track)) return;
    const QString source = sourceOfClip(change.clip);
    StageMotion::Flight flight;
    flight.image = posterForClip(change.clip);
    flight.from = flightStart(source);
    flight.to = toShell(m_timeline, lane);
    flight.delay = index * tokens::motion::flightStagger;
    flight.duration = tokens::motion::flight;
    flight.tail = tokens::motion::flightLand;
    flight.ring = true;
    flight.radius = tokens::radius::small;
    const QJsonObject tracks = withClipNames(proposalTimelineFor(m_snapshot, m_proposalId), m_project.value(QStringLiteral("sources")).toArray());
    for (const QJsonValue &track : tracks.value(QStringLiteral("tracks")).toArray())
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray())
            if (value.toObject().value(QStringLiteral("id")).toString() == change.clip) flight.label = value.toObject().value(QStringLiteral("name")).toString();
    if (!m_motion->fly(flight)) return;
    const QImage poster = flight.image;
    QTimer::singleShot(flight.delay + tokens::motion::flight / 2, this, [this, poster] { coverPicture(poster); });
}

// While the cut is only a proposal the picture shows each clip as it lands, ahead
// of the render that will show it exactly.
void MainWindow::coverPicture(const QImage &image) {
    if (image.isNull() || !m_timeline->showsProposedCut() || m_changeReel->isActive()) return;
    m_motion->releaseStaying();
    StageMotion::Flight cover;
    cover.image = image;
    cover.from = m_shell->rects().picture;
    cover.to = cover.from;
    cover.fromOpacity = 0.0;
    cover.duration = tokens::motion::appear;
    cover.lively = false;
    cover.stays = true;
    cover.fit = true;
    cover.radius = tokens::layout::pictureRadius;
    m_motion->fly(cover);
}

void MainWindow::flushFlights() {
    if (m_shell->isMoving() || m_holdFlights) return;
    const QList<ReviewModel::ClipChange> pending = m_pendingFlights;
    m_pendingFlights.clear();
    for (int index = 0; index < pending.size(); ++index) flyChange(pending.at(index), index);
}

// As Kerf starts on a project with footage and no cut, the footage it will
// choose from gathers in the conversation.
void MainWindow::showSheet() {
    m_sheetWanted = false;
    if (m_shell->panes().panelOpen() || !m_panesFollowContent || ui::reducedMotion()) return;
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
        if (!track.toObject().value(QStringLiteral("clips")).toArray().isEmpty()) return;
    const QJsonArray sources = m_project.value(QStringLiteral("sources")).toArray();
    const QHash<QString, QString> names = sourceNames(sources);
    QList<FootageTile> tiles;
    for (const QJsonValue &value : sources) {
        const QString id = value.toObject().value(QStringLiteral("id")).toString();
        const QImage poster = m_artwork->frames.value(id);
        if (!poster.isNull()) tiles << FootageTile{id, names.value(id), poster};
        if (tiles.size() == 8) break;
    }
    // A folder just brought in has its media, and their posters, still to come.
    if (sources.isEmpty() || tiles.size() < std::min<qsizetype>(sources.size(), 8)) {
        m_sheetWanted = true;
        return;
    }
    m_chat->setFootage(tiles);
}

// Kerf's work is done: the picture itself goes home to the folder and the
// conversation has the room again.
void MainWindow::goHome() {
    SidebarRow *row = m_sidebar->projectRow(m_projectDirectory);
    if (!row || !m_viewer->isVisible() || ui::reducedMotion()) return;
    stage::Panes panes = m_shell->panes();
    panes.picture = false;
    panes.timeline = false;
    m_shell->veilTimeline(1.0);
    m_shell->setPanes(panes, toShell(row, QRect(8, 4, 44, row->height() - 8)));
}

void MainWindow::flyExportHome() {
    SidebarRow *row = m_sidebar->projectRow(m_projectDirectory);
    if (!row || !m_viewer->isVisible() || m_viewer->frame().isNull()) return;
    if (m_agentBusy) return;
    StageMotion::Flight home;
    home.image = m_viewer->frame();
    home.from = toShell(m_viewer, m_viewer->videoRect().toRect());
    home.to = toShell(row, QRect(8, 4, 44, row->height() - 8));
    home.fromOpacity = 0.9;
    home.toOpacity = 0.2;
    home.lively = false;
    home.duration = tokens::motion::flight;
    home.radius = tokens::layout::pictureRadius;
    m_motion->fly(home);
}

void MainWindow::takeOverFrom(const QPixmap &look, const QRect &box) {
    if (ui::reducedMotion()) return;
    // The window it follows, laid over this one and fading; the request box
    // goes on to where this one's is.
    moves::fadeIn(m_chat, tokens::motion::hover, tokens::motion::settle);
    QTimer::singleShot(0, this, [this, look, box] {
        const QPoint shift = m_shell->mapTo(this, QPoint());
        const QRect window = rect().translated(-shift);
        StageMotion::Flight room;
        room.image = look.toImage();
        room.from = window;
        room.to = window;
        room.toOpacity = 0.0;
        room.duration = tokens::motion::handOver;
        room.lively = false;
        room.radius = 0;
        m_motion->fly(room);
        const qreal ratio = look.devicePixelRatio();
        const QRect pixels(QPointF(box.topLeft()).toPoint() * ratio, box.size() * ratio);
        StageMotion::Flight request;
        request.image = look.copy(pixels).toImage();
        request.from = box.translated(-shift);
        request.to = toShell(m_chat->composerWidget()->box(), m_chat->composerWidget()->box()->rect());
        request.toOpacity = 0.0;
        request.duration = tokens::motion::stage;
        request.lively = false;
        request.radius = tokens::radius::panel;
        m_motion->fly(request);
    });
}

// With no cut yet there is nothing for a proposal to change on the timeline, so
// the timeline and the picture show the one Kerf is proposing, as it grows.
void MainWindow::showProposedCut() {
    bool programHasClips = false;
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
        programHasClips = programHasClips || !track.toObject().value(QStringLiteral("clips")).toArray().isEmpty();
    QJsonObject proposed = programHasClips || m_proposalId.isEmpty() ? QJsonObject() : proposalTimelineFor(m_snapshot, m_proposalId);
    double end = 0.0;
    for (const QJsonValue &track : proposed.value(QStringLiteral("tracks")).toArray())
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            end = std::max(end, clip.value(QStringLiteral("timelineStart")).toDouble() + clip.value(QStringLiteral("duration")).toDouble());
        }
    const bool showing = end > 0.0;
    if (!showing && !m_timeline->showsProposedCut()) return;
    m_proposedDuration = end;
    m_timeline->setShowsProposedCut(showing);
    // The picture has the shape of what it is showing.
    const QSize canvas = resolvedCanvasSize(showing ? proposed : m_project, m_project.value(QStringLiteral("sources")).toArray());
    m_viewer->setProjectInfo(canvas.width(), canvas.height());
    m_shell->setAspect(m_viewer->pictureAspect());
    m_timeline->setWideSequence(stage::isWide(m_viewer->pictureAspect()));
    syncCanvasPreset(canvas);
    if (!showing) {
        m_cutPreview.stop();
        m_previewedCut.clear();
        quietenCutPreview();
        m_timeline->setProject(m_project, m_duration, m_fps);
        return;
    }
    m_timeline->setProject(withClipNames(proposed, m_project.value(QStringLiteral("sources")).toArray()), end, m_fps);
    // The picture follows once the edits have settled for a moment.
    if (m_proposalId != m_previewedCut && m_followKerf && !m_changeReel->isActive()) m_cutPreview.start();
    quietenCutPreview();
}

// While Kerf builds its cut, the picture just keeps up: what renders, and that it
// is a preview, are for when one is asked for.
void MainWindow::quietenCutPreview() {
    const bool quiet = m_agentBusy && m_timeline->showsProposedCut();
    m_stageWiring->previewChip()->setSuppressed(quiet || m_changeReel->isActive());
    // The reel's strip already says it is getting the picture.
    m_stageWiring->hud()->setQuiet(quiet || m_cutRendering || m_changeReel->isActive());
    // The cut it is making is its card in the conversation, at work.
    m_chat->setBuildingProposal(quiet ? m_proposalId : QString());
    if (m_workChip) m_workChip->setVisible(m_agentBusy && !m_changeReel->isActive() && !m_stageWiring->previewChip()->isVisible());
}

void MainWindow::setFollowKerf(bool follow) {
    m_followKerf = follow;
    QSettings().setValue(QStringLiteral("layout/followKerf"), follow);
    syncPanelCommands();
    if (m_followButton) m_followButton->setOn(follow);
}

void MainWindow::showAgentAt(double seconds) {
    if (seconds < 0.0) return;
    m_timeline->setAgentCursor(seconds);
    if (!m_followKerf || m_playing || m_changeReel->isActive() || std::max(m_duration, m_proposedDuration) <= 0.0) return;
    queueSeek(seconds);
}

void MainWindow::followAgentFocus(const QJsonObject &event) {
    const double start = m_project.value(QStringLiteral("start")).toDouble();
    const QJsonArray times = event.value(QStringLiteral("times")).toArray();
    if (!times.isEmpty()) {
        showAgentAt(reel::parseTime(times.first().toString(), m_fps, start));
        return;
    }
    const QJsonArray clips = event.value(QStringLiteral("clips")).toArray();
    if (clips.isEmpty()) return;
    const QJsonObject clip = clipObject(clips.first().toString());
    if (!clip.isEmpty()) showAgentAt(clip.value(QStringLiteral("timelineStart")).toDouble());
}

void MainWindow::followLiveProposal() {
    ReviewModel *review = m_reviewWiring->model();
    if (!m_agentBusy || !review) return;
    const ReviewModel::Proposal *proposal = m_proposalId.isEmpty() ? nullptr : review->proposal(m_proposalId);
    if (!proposal) return;
    const QSet<QString> before = m_liveChanges;
    QSet<QString> keys;
    const ReviewModel::ClipChange *newest = nullptr;
    for (const ReviewModel::ClipChange &change : proposal->changes) {
        const QString key = QStringLiteral("%1@%2").arg(change.clip).arg(change.newEnd);
        keys.insert(key);
        if (!m_liveChanges.contains(key) && !change.removed && (!newest || change.newStart >= newest->newStart)) newest = &change;
    }
    m_liveChanges = keys;
    // What it is doing, in a line.
    if (m_timeline->showsProposedCut() && keys != before) {
        int placed = 0;
        for (const ReviewModel::ClipChange &change : proposal->changes) placed += !change.removed && !m_timeline->isAudioLane(change.track);
        m_chat->setTurnStatus(placed == 1 ? QStringLiteral("Cutting · 1 clip") : QStringLiteral("Cutting · %1 clips").arg(placed));
    }
    // The new ones fly to where they land, once the timeline is there.
    for (const ReviewModel::ClipChange &change : proposal->changes)
        if (!change.removed && !before.contains(QStringLiteral("%1@%2").arg(change.clip).arg(change.newEnd))) m_pendingFlights << change;
    flushFlights();
    // Where a trim cuts is what it changes.
    if (newest) showAgentAt(newest->trimmed && !newest->moved ? newest->newEnd : newest->newStart);
}

void MainWindow::endAgentFollow() {
    m_timeline->setAgentCursor(std::numeric_limits<double>::quiet_NaN());
    m_liveChanges.clear();
    // What stands in for the picture stays until its render has come.
    if (!m_cutRendering && !m_cutPreview.isActive()) m_motion->releaseStaying();
    m_chat->setFootage({});
    m_sheetWanted = false;
    refreshSidebar();
}

// The changes with the names the program knows its clips by.
QList<ReviewModel::ClipChange> MainWindow::namedChanges(QList<ReviewModel::ClipChange> changes) const {
    for (ReviewModel::ClipChange &change : changes) {
        const QJsonObject clip = clipObject(change.clip);
        if (!clip.isEmpty()) change.name = clipName(clip);
    }
    return changes;
}

// Watching, what the picture was at this segment wipes over what it now is.
void MainWindow::followReelCompare() {
    const bool watching = m_changeReel->isActive() && m_changeReel->side() == ChangeReel::Side::After;
    if (!watching) {
        m_reelCompareAt.clear();
        m_viewer->setCompareFrame({});
        return;
    }
    const reel::Segment &segment = m_changeReel->segments().at(m_changeReel->index());
    const QString key = QStringLiteral("%1/%2").arg(m_changeReel->proposalId()).arg(m_changeReel->index());
    if (key == m_reelCompareAt) return;
    m_reelCompareAt = key;
    m_viewer->setCompareFrame({});
    // The program's clip there; a part that is new has nothing to compare with.
    const double at = std::max(segment.before, 0.0) + 0.04;
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray()) {
        if (track.toObject().value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
        for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = value.toObject();
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            if (at < start || at >= start + clip.value(QStringLiteral("duration")).toDouble()) continue;
            m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("effect_previews")},
                                    {QStringLiteral("clip"), clip.value(QStringLiteral("id")).toString()},
                                    {QStringLiteral("at"), at},
                                    {QStringLiteral("effects"), QJsonArray{QJsonObject{{QStringLiteral("fx"), QStringLiteral("color.basic")},
                                                                                     {QStringLiteral("key"), QStringLiteral("reel-before:") + key}}}},
                                    {QStringLiteral("width"), 960},
                                    {QStringLiteral("key"), QStringLiteral("reel")}});
            return;
        }
    }
}

void MainWindow::watchChanges(const QString &proposalId) {
    ReviewModel *review = m_reviewWiring->model();
    const ReviewModel::Proposal *proposal = review ? review->proposal(proposalId) : nullptr;
    if (!proposal) return;
    double after = 0.0;
    for (const ReviewModel::ClipChange &change : proposal->changes) after = std::max(after, change.removed ? change.oldEnd : change.newEnd);
    const QList<reel::Segment> segments = reel::plan(namedChanges(proposal->changes), m_duration, after);
    if (segments.isEmpty()) {
        showStatus(QStringLiteral("This proposal changes nothing on the timeline"), 4000);
        return;
    }
    QSet<QString> touched;
    for (const ReviewModel::ClipChange &change : proposal->changes) touched.insert(change.clip);
    m_timeline->setFocusClips(touched);
    m_changeReel->start(proposalId, segments);
}

} // namespace kerf
