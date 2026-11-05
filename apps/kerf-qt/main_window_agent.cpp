// MainWindow's view of Kerf at work: following where it is on the timeline and
// picture, watching only what a proposal changes, and comparing versions of a
// request that several agents make at once.

#include "main_window.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "editor_bus.h"
#include "project_sidebar.h"
#include "review_model.h"
#include "stage_shell.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "ui/change_reel.h"
#include "ui/compare_view.h"
#include "viewer_widget.h"
#include "media_artwork.h"
#include "wiring/main_window_shared.h"
#include "wiring/review_wiring.h"
#include "wiring/stage_wiring.h"

#include <QJsonArray>
#include <QSet>
#include <QSettings>

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
        // The strip has its own Exit.
        m_stageWiring->previewChip()->setSuppressed(active);
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

    m_compare = new CompareView;
    m_shell->setCompare(m_compare);
    connect(m_compare, &CompareView::watchRequested, this, &MainWindow::watchChanges);
    connect(m_compare, &CompareView::useRequested, this, &MainWindow::useVersion);
    connect(m_compare, &CompareView::closeRequested, this, [this] { m_shell->setCompareShown(false); });
    if (ReviewModel *review = m_reviewWiring->model()) connect(review, &ReviewModel::changed, this, &MainWindow::refreshCompare);
    connect(m_artwork, &MediaArtwork::changed, this, &MainWindow::refreshCompare);

    m_followKerf = QSettings().value(QStringLiteral("layout/followKerf"), true).toBool();
}

void MainWindow::setFollowKerf(bool follow) {
    m_followKerf = follow;
    QSettings().setValue(QStringLiteral("layout/followKerf"), follow);
    syncPanelCommands();
    if (m_followButton) m_followButton->setOn(follow);
}

void MainWindow::showAgentAt(double seconds) {
    if (seconds < 0.0) return;
    if (m_versionsRunning) return;
    m_timeline->setAgentCursor(seconds);
    if (!m_followKerf || m_playing || m_changeReel->isActive() || m_duration <= 0.0) return;
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
    // Versions grow on their own cards, with no one place to follow.
    if (!m_agentBusy || !review || m_versionsRunning) return;
    const ReviewModel::Proposal *proposal = m_proposalId.isEmpty() ? nullptr : review->proposal(m_proposalId);
    if (!proposal) return;
    QSet<QString> keys;
    const ReviewModel::ClipChange *newest = nullptr;
    for (const ReviewModel::ClipChange &change : proposal->changes) {
        const QString key = QStringLiteral("%1@%2").arg(change.clip).arg(change.newEnd);
        keys.insert(key);
        if (!m_liveChanges.contains(key) && !change.removed && (!newest || change.newStart >= newest->newStart)) newest = &change;
    }
    m_liveChanges = keys;
    // Where a trim cuts is what it changes.
    if (newest) showAgentAt(newest->trimmed && !newest->moved ? newest->newEnd : newest->newStart);
}

void MainWindow::endAgentFollow() {
    m_timeline->setAgentCursor(std::numeric_limits<double>::quiet_NaN());
    m_liveChanges.clear();
    bool ran = false;
    for (VersionRun &run : m_versionRuns)
        if (run.state == CompareView::State::Running) {
            run.state = CompareView::State::Stopped;
            ran = true;
        }
    if (ran) refreshCompare();
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
    m_changeReel->start(proposalId, segments);
}

bool MainWindow::sendVersions(const QString &text) {
    QJsonObject command = agentSettingsPayload();
    command.insert(QStringLiteral("type"), QStringLiteral("agent_versions"));
    command.insert(QStringLiteral("text"), text);
    if (!m_bus->send(command)) return false;
    m_versionRuns.clear();
    m_versionsRunning = true;
    m_agentBusy = true;
    m_abortButton->setEnabled(true);
    updateChatControls();
    updateCommandStates();
    revealPanel(true, true);
    m_panesFollowContent = false;
    m_shell->setCompareShown(true);
    refreshCompare();
    return true;
}

void MainWindow::followVersion(const QJsonObject &event) {
    const QString key = event.value(QStringLiteral("key")).toString();
    if (key.isEmpty()) return;
    auto run = std::find_if(m_versionRuns.begin(), m_versionRuns.end(), [&key](const VersionRun &entry) { return entry.key == key; });
    if (run == m_versionRuns.end()) {
        m_versionRuns.append(VersionRun{key});
        std::sort(m_versionRuns.begin(), m_versionRuns.end(), [](const VersionRun &a, const VersionRun &b) { return a.key < b.key; });
        run = std::find_if(m_versionRuns.begin(), m_versionRuns.end(), [&key](const VersionRun &entry) { return entry.key == key; });
    }
    const QString state = event.value(QStringLiteral("state")).toString();
    const QString thread = event.value(QStringLiteral("thread")).toString();
    if (!thread.isEmpty()) run->thread = thread;
    const QString proposal = event.value(QStringLiteral("proposalId")).toString();
    if (!proposal.isEmpty()) run->proposal = proposal;
    if (state == QStringLiteral("running")) {
        run->state = CompareView::State::Running;
        const QString status = event.value(QStringLiteral("status")).toString();
        if (!status.isEmpty()) run->status = QStringLiteral("Working · %1").arg(status);
    } else {
        run->state = state == QStringLiteral("done")      ? CompareView::State::Ready
                     : state == QStringLiteral("stopped") ? CompareView::State::Stopped
                                                          : CompareView::State::Failed;
        run->status = event.value(QStringLiteral("message")).toString();
    }
    refreshCompare();
    refreshSidebar();
}

void MainWindow::refreshCompare() {
    if (!m_compare) return;
    ReviewModel *review = m_reviewWiring->model();
    QList<CompareView::Version> versions;
    for (const VersionRun &run : std::as_const(m_versionRuns)) {
        CompareView::Version version;
        version.key = run.key;
        version.proposalId = run.proposal;
        version.state = run.state;
        version.status = run.status;
        if (const ReviewModel::Proposal *proposal = review && !run.proposal.isEmpty() ? review->proposal(run.proposal) : nullptr) {
            // A revise gives the proposal a new id.
            version.proposalId = proposal->id;
            for (const ReviewModel::ClipChange &change : namedChanges(proposal->changes)) version.changes << reel::changeTitle(change);
            // The first clip it touches shows what it looks like.
            const QJsonObject timeline = proposalTimelineFor(m_snapshot, proposal->id);
            QString wanted = proposal->changes.isEmpty() ? QString() : proposal->changes.constFirst().clip;
            for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray())
                for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
                    const QJsonObject clip = value.toObject();
                    if (version.poster.isNull() && (wanted.isEmpty() || clip.value(QStringLiteral("id")).toString() == wanted))
                        version.poster = m_artwork->frames.value(clip.value(QStringLiteral("sourceId")).toString());
                }
        }
        // What Kerf said makes it different.
        if (const ReviewModel::Thread *thread = review && !run.thread.isEmpty() ? review->thread(run.thread) : nullptr;
            thread && !thread->replies.isEmpty() && version.status.isEmpty())
            version.status = thread->replies.constLast().body.section(QLatin1Char('\n'), 0, 0).trimmed();
        versions << version;
    }
    m_compare->setVersions(versions);
}

void MainWindow::useVersion(const QString &proposalId) {
    ReviewModel *review = m_reviewWiring->model();
    if (m_changeReel->isActive()) m_changeReel->stop();
    // The others are made from the same revision: accepting one leaves them stale.
    for (const VersionRun &run : std::as_const(m_versionRuns)) {
        if (run.proposal.isEmpty()) continue;
        const ReviewModel::Proposal *proposal = review ? review->proposal(run.proposal) : nullptr;
        const QString id = proposal ? proposal->id : run.proposal;
        if (id != proposalId && (!proposal || proposal->isOpen())) m_reviewWiring->reject(id);
    }
    m_reviewWiring->accept(proposalId);
    m_versionRuns.clear();
    m_shell->setCompareShown(false);
    refreshCompare();
    refreshSidebar();
}

} // namespace kerf
