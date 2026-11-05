#include "timeline_wiring.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "main_window.h"
#include "source_monitor_panel.h"
#include "stage_shell.h"

#include <QAction>
#include <QEvent>
#include <QJsonArray>
#include <QLayout>

namespace kerf {

namespace {

// The registry's commands behind the strip: its two tools, Snap, and the
// transport in its order.
constexpr const char *kToolCommands[] = {"tool.selection", "tool.razor"};
constexpr const char *kSnapCommand = "sequence.snap";
constexpr const char *kTransportCommands[] = {"playback.goToStart", "playback.previousFrame", "playback.toggle", "playback.nextFrame",
                                              "playback.goToEnd"};

} // namespace

TimelineWiring::TimelineWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("timelineWiring"));
    m_own = new ReviewModel(this);
    m_own->setObjectName(QStringLiteral("timelineReview"));
}

TimelineWiring::~TimelineWiring() {
    // The strip is the panel's child, gone with it.
}

ReviewModel *TimelineWiring::review() const { return m_shared ? m_shared.data() : m_own; }

QString TimelineWiring::currentThread() const { return m_shared ? m_shared->currentThread() : m_own->currentThread(); }

void TimelineWiring::attach() {
    if (m_attached || !m_window || !m_window->m_timeline) return;
    m_attached = true;
    TimelineWidget *timeline = m_window->m_timeline;
    // The strip over the timeline, the first thing in its panel.
    m_tools = new TimelineToolStrip(timeline, m_window->m_timelinePanel);
    if (auto *layout = qobject_cast<QVBoxLayout *>(m_window->m_timelinePanel->layout())) layout->insertWidget(0, m_tools);
    connect(m_tools, &TimelineToolStrip::toolRequested, this, [this](TimelineTool tool) {
        triggerCommand(QString::fromLatin1(kToolCommands[TimelineToolStrip::toolIndex(tool)]));
    });
    connect(m_tools, &TimelineToolStrip::snapRequested, this, [this](bool on) {
        QAction *snap = m_window->m_commands ? m_window->m_commands->action(QString::fromLatin1(kSnapCommand)) : nullptr;
        if (snap) snap->setChecked(on);
        else m_window->m_timeline->setSnapping(on);
    });
    connect(m_tools, &TimelineToolStrip::transportRequested, this, [this](TimelineToolStrip::Transport which) {
        triggerCommand(QString::fromLatin1(kTransportCommands[int(which)]));
    });
    connect(m_bus, &EditorBus::playingChanged, this, [this](bool playing) {
        m_tools->setPlaying(playing);
        followCommands();
    });
    // Each control stands for its registry command (bindCommands).
    bindCommands();

    // The block under a vertical picture.
    m_pictureTransport = new PictureTransport(m_window->m_shell);
    m_window->m_shell->setTransport(m_pictureTransport);
    connect(m_bus, &EditorBus::playheadChanged, this, &TimelineWiring::showTime);
    connect(m_bus, &EditorBus::snapshotChanged, this, [this] { showTime(m_bus->playhead()); });
    showTime(m_bus->playhead());

    // Monitoring only: the playback mix hears the soloed tracks alone.
    connect(timeline, &TimelineWidget::soloChanged, this, &TimelineWiring::sendSolo);
    // The timeline is enabled when the engine is ready (setConnection): a
    // new engine (a reconnect) mixes every track until it hears the solo
    // again.
    timeline->installEventFilter(this);
    // The rubber band writes the clip's gain: a level, or its keys.
    // An effect card dropped on a clip adds it at the end of its stack, as
    // the Inspector's drop does.
    connect(timeline, &TimelineWidget::effectDropped, this, [this](const QString &clip, const QString &fx) {
        m_window->sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("fx-add")},
                                                  {QStringLiteral("clip"), clip},
                                                  {QStringLiteral("fx"), fx}}});
    });
    connect(timeline, &TimelineWidget::gainRequested, this, [this](const QString &clip, const QJsonValue &gain) {
        m_window->sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")},
                                                  {QStringLiteral("clip"), clip},
                                                  {QStringLiteral("props"), QJsonObject{{QStringLiteral("gain"), gain}}}}});
    });
    connect(timeline, &TimelineWidget::commentMarkActivated, this, [this](const QString &id) {
        if (m_shared) m_shared->setCurrentThread(id);
        m_own->setCurrentThread(id);
        emit m_bus->openThread(id);
    });

    connect(m_bus, &EditorBus::event, this, &TimelineWiring::handleEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, [this] { m_own->setSnapshot(m_bus->snapshot()); });
    connect(m_bus, &EditorBus::identityChanged, this, [this](const QJsonObject &user) { m_own->setIdentity(user); });
    connect(m_bus, &EditorBus::currentThreadChanged, this, [this](const QString &id) {
        if (!m_shared) m_own->setCurrentThread(id);
        refreshReview();
    });
    connect(m_bus, &EditorBus::reviewChanged, this, &TimelineWiring::followSharedReview);
    connect(m_own, &ReviewModel::changed, this, &TimelineWiring::refreshReview);
    connect(m_own, &ReviewModel::currentChanged, this, &TimelineWiring::refreshReview);
    followSharedReview();
}

void TimelineWiring::setWide(bool wide) {
    if (!m_tools || wide == m_wide) return;
    m_wide = wide;
    m_tools->setTransportHost(wide ? nullptr : m_pictureTransport->transportRow());
}

void TimelineWiring::showTime(double seconds) {
    m_pictureTransport->setTime(timelineTimecode(seconds, m_bus->fps()));
}

void TimelineWiring::sendSolo(const QStringList &tracks) {
    m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("solo")}, {QStringLiteral("tracks"), QJsonArray::fromStringList(tracks)}});
}

void TimelineWiring::bindCommands() {
    // The window makes its registry after its regions (setupActions runs
    // after buildUi), so this binds on the first call that finds it: from
    // attach, the first connection change (setConnection, in the window's
    // constructor) or the first review refresh.
    if (m_commandsBound || !m_tools || !m_window || !m_window->m_commands) return;
    m_commandsBound = true;
    CommandRegistry *commands = m_window->m_commands;
    QStringList ids{QString::fromLatin1(kSnapCommand)};
    for (const char *id : kToolCommands) ids << QString::fromLatin1(id);
    for (const char *id : kTransportCommands) ids << QString::fromLatin1(id);
    for (const QString &id : std::as_const(ids))
        if (QAction *action = commands->action(id)) connect(action, &QAction::changed, this, &TimelineWiring::followCommands);
    connect(commands, &CommandRegistry::shortcutsChanged, this, &TimelineWiring::followCommands);
    followCommands();
}

void TimelineWiring::followCommands() {
    if (!m_tools || !m_window || !m_window->m_commands) return;
    CommandRegistry *commands = m_window->m_commands;
    const auto state = [commands](const QString &id) {
        struct State {
            bool enabled = false;
            QString reason;
            QString shortcut;
        };
        const QAction *action = commands->action(id);
        // A command the registry does not have is unavailable.
        if (!action) return State{false, QStringLiteral("Unavailable"), {}};
        return State{action->isEnabled(), commands->reason(CommandRegistry::resolve(id)), action->shortcut().toString(QKeySequence::NativeText)};
    };
    for (int index = 0; index < 2; ++index) {
        const auto s = state(QString::fromLatin1(kToolCommands[index]));
        m_tools->followTool(TimelineToolStrip::toolAt(index), s.enabled, s.reason, s.shortcut);
    }
    {
        const auto s = state(QString::fromLatin1(kSnapCommand));
        m_tools->followSnap(s.enabled, s.reason, s.shortcut);
    }
    for (int index = 0; index < TimelineToolStrip::kTransports; ++index) {
        const auto s = state(QString::fromLatin1(kTransportCommands[index]));
        m_tools->followTransport(TimelineToolStrip::Transport(index), s.enabled, s.reason, s.shortcut);
    }
}

void TimelineWiring::triggerCommand(const QString &id) {
    if (!m_window->m_commands) return;
    m_window->triggerCommand(id);
}

bool TimelineWiring::eventFilter(QObject *watched, QEvent *event) {
    if (m_window && watched == m_window->m_timeline && event->type() == QEvent::EnabledChange) {
        bindCommands();
        TimelineWidget *timeline = m_window->m_timeline;
        if (timeline->isEnabled() && !timeline->soloTracks().isEmpty()) sendSolo(timeline->soloTracks());
        return QObject::eventFilter(watched, event);
    }
    return QObject::eventFilter(watched, event);
}

void TimelineWiring::followSharedReview() {
    ReviewModel *shared = m_bus->review();
    if (shared == m_shared) return;
    for (const QMetaObject::Connection &connection : std::as_const(m_sharedConnections)) disconnect(connection);
    m_sharedConnections.clear();
    m_shared = shared;
    if (shared) {
        // The panel's model: its current thread, and its counts for Ask.
        m_sharedConnections << connect(shared, &ReviewModel::currentChanged, this, [this](const QString &id) {
            m_own->setCurrentThread(id);
            refreshReview();
        });
        m_sharedConnections << connect(shared, &ReviewModel::changed, this, &TimelineWiring::refreshReview);
        m_own->setCurrentThread(shared->currentThread());
    }
    refreshReview();
}

void TimelineWiring::handleEvent(const QString &type, const QJsonObject &json) {
    TimelineWidget *timeline = m_window->m_timeline;
    if (type == QStringLiteral("comments") || type == QStringLiteral("collab")) {
        m_own->applyEvent(type, json);
    } else if (type == QStringLiteral("check")) {
        const QString status = json.value(QStringLiteral("status")).toString();
        if (status == QStringLiteral("done")) timeline->setQcClips(qcClips(json));
    } else if (type == QStringLiteral("preview")) {
        const QString id = previewedProposal(json, timeline->previewProposal());
        if (id.isEmpty()) {
            timeline->setPreview({}, {});
            return;
        }
        const QJsonObject snapshot = m_bus->snapshot();
        QJsonObject proposed = snapshot.value(QStringLiteral("proposalTimelines")).toObject().value(id).toObject();
        if (proposed.isEmpty() && snapshot.value(QStringLiteral("pendingProposal")).toObject().value(QStringLiteral("proposalId")).toString() == id)
            proposed = snapshot.value(QStringLiteral("proposalTimeline")).toObject();
        proposed = MainWindow::withClipNames(proposed, snapshot.value(QStringLiteral("timeline")).toObject().value(QStringLiteral("sources")).toArray());
        timeline->setPreview(proposed.isEmpty() ? QString() : id, proposed);
    }
}

QString TimelineWiring::previewedProposal(const QJsonObject &preview, const QString &playing) {
    const QString status = preview.value(QStringLiteral("status")).toString();
    const QString id = preview.value(QStringLiteral("proposalId")).toString();
    // Rendering keeps what plays; ready and live play the proposal named;
    // anything else (stopped, exited, failed, required) ends it.
    if (status == QStringLiteral("running")) return playing;
    if (status == QStringLiteral("ready") || status == QStringLiteral("live")) return id;
    return {};
}

void TimelineWiring::refreshReview() {
    if (!m_window || !m_window->m_timeline) return;
    TimelineWidget *timeline = m_window->m_timeline;
    timeline->setCommentMarks(commentMarks(*m_own, currentThread()));
    timeline->setProposalBands(proposalBands(*m_own));
    bindCommands();
}

QList<TimelineWidget::CommentMark> TimelineWiring::commentMarks(const ReviewModel &review, const QString &current) {
    QList<TimelineWidget::CommentMark> marks;
    for (const ReviewModel::LaneMarker &marker : review.laneMarkers()) {
        TimelineWidget::CommentMark mark;
        mark.id = marker.id;
        mark.at = marker.at;
        mark.end = marker.end;
        mark.ranged = marker.ranged;
        mark.circle = marker.circle;
        mark.current = marker.current || (!current.isEmpty() && marker.id == current);
        mark.resolved = marker.resolved;
        mark.name = marker.name;
        marks << mark;
    }
    return marks;
}

QList<TimelineWidget::ProposalBand> TimelineWiring::proposalBands(const ReviewModel &review) {
    QList<TimelineWidget::ProposalBand> bands;
    for (const ReviewModel::Proposal &proposal : review.proposals()) {
        if (!proposal.isOpen() || !proposal.hasRange) continue;
        TimelineWidget::ProposalBand band;
        band.id = proposal.id;
        band.author = proposal.author.name;
        band.start = proposal.start;
        band.end = proposal.end;
        for (const ReviewModel::ClipChange &change : proposal.changes) {
            if (!change.track.isEmpty() && !band.tracks.contains(change.track)) band.tracks << change.track;
            if (change.transition) continue;
            band.clips << change.clip;
            if (!change.added && (change.moved || change.trimmed || change.removed || change.retracked))
                band.ghosts.append({change.track, change.oldStart, change.oldEnd});
        }
        bands << band;
    }
    return bands;
}

QSet<QString> TimelineWiring::qcClips(const QJsonObject &check) {
    QSet<QString> clips;
    for (const QJsonValue &value : check.value(QStringLiteral("findings")).toArray()) {
        const QJsonObject finding = value.toObject();
        if (finding.value(QStringLiteral("severity")).toString() == QStringLiteral("info")) continue;
        const QString entity = finding.value(QStringLiteral("entity")).toString();
        if (!entity.isEmpty()) clips.insert(entity);
    }
    return clips;
}

} // namespace kerf
