#include "timeline_wiring.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "main_window.h"
#include "source_monitor_panel.h"

#include <QAction>
#include <QEvent>
#include <QJsonArray>
#include <QLabel>
#include <QTimer>

#include <DockAreaTitleBar.h>

namespace kerf {

namespace {

// The registry's commands behind the strip, in its order: the six tools
// (TimelineToolStrip::toolAt), then Snap, Add marker, Add track and Ask.
constexpr const char *kStripCommands[] = {"tool.selection", "tool.razor", "tool.ripple", "tool.roll", "tool.slip",
                                          "tool.slide",     "sequence.snap", "marker.add", "timeline.addTrack", "ai.ask"};

} // namespace

TimelineWiring::TimelineWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("timelineWiring"));
    m_own = new ReviewModel(this);
    m_own->setObjectName(QStringLiteral("timelineReview"));
}

TimelineWiring::~TimelineWiring() {
    // The strip is the title strip's child once placed; a strip never placed is ours.
    if (m_tools && !m_tools->parent()) delete m_tools.data();
}

ReviewModel *TimelineWiring::review() const { return m_shared ? m_shared.data() : m_own; }

QString TimelineWiring::currentThread() const { return m_shared ? m_shared->currentThread() : m_own->currentThread(); }

void TimelineWiring::attach() {
    if (m_attached || !m_window || !m_window->m_timeline) return;
    m_attached = true;
    TimelineWidget *timeline = m_window->m_timeline;
    m_tools = new TimelineToolStrip(timeline);
    m_tools->hide();
    // The strip follows "N tracks · duration" into whatever title strip
    // shows the timeline (the workspace moves it between areas).
    if (QLabel *detail = m_window->m_timelineDetail) {
        detail->installEventFilter(this);
        QTimer::singleShot(0, this, &TimelineWiring::placeTools);
    }
    connect(m_tools, &TimelineToolStrip::toolRequested, this, [this](TimelineTool tool) {
        triggerCommand(QString::fromLatin1(kStripCommands[TimelineToolStrip::toolIndex(tool)]));
    });
    connect(m_tools, &TimelineToolStrip::snapRequested, this, [this](bool on) {
        QAction *snap = m_window->m_commands ? m_window->m_commands->action(QStringLiteral("sequence.snap")) : nullptr;
        if (snap) snap->setChecked(on);
        else m_window->m_timeline->setSnapping(on);
    });
    connect(m_tools, &TimelineToolStrip::addMarkerRequested, this, [this] { triggerCommand(QStringLiteral("marker.add")); });
    connect(m_tools, &TimelineToolStrip::addTrackRequested, this, [this] { triggerCommand(QStringLiteral("timeline.addTrack")); });
    connect(m_tools, &TimelineToolStrip::askRequested, this, [this] { triggerCommand(QStringLiteral("ai.ask")); });
    // Each control stands for its registry command (bindCommands).
    bindCommands();

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
    connect(m_bus, &EditorBus::snapshotChanged, this, [this] {
        m_own->setSnapshot(m_bus->snapshot());
        refreshPatch();
    });
    connect(m_bus, &EditorBus::identityChanged, this, [this](const QJsonObject &user) { m_own->setIdentity(user); });
    connect(m_bus, &EditorBus::currentThreadChanged, this, [this](const QString &id) {
        if (!m_shared) m_own->setCurrentThread(id);
        refreshReview();
    });
    connect(m_bus, &EditorBus::reviewChanged, this, &TimelineWiring::followSharedReview);
    connect(m_own, &ReviewModel::changed, this, &TimelineWiring::refreshReview);
    connect(m_own, &ReviewModel::currentChanged, this, &TimelineWiring::refreshReview);
    if (m_window->m_sourceMonitor) connect(m_window->m_sourceMonitor, &SourceMonitorPanel::changed, this, &TimelineWiring::refreshPatch);
    connect(timeline, &TimelineWidget::targetsChanged, this, &TimelineWiring::refreshPatch);
    followSharedReview();
}

void TimelineWiring::sendSolo(const QStringList &tracks) {
    m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("solo")}, {QStringLiteral("tracks"), QJsonArray::fromStringList(tracks)}});
}

void TimelineWiring::bindCommands() {
    // The window makes its registry after its regions (setupActions runs
    // after buildUi), so this binds on the first call that finds it: from
    // attach, the first connection change (setConnection, in the window's
    // constructor) or the first Ask refresh.
    if (m_commandsBound || !m_tools || !m_window || !m_window->m_commands) return;
    m_commandsBound = true;
    CommandRegistry *commands = m_window->m_commands;
    for (const char *id : kStripCommands)
        if (QAction *action = commands->action(QString::fromLatin1(id))) connect(action, &QAction::changed, this, &TimelineWiring::followCommands);
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
    for (int index = 0; index < 6; ++index) {
        const TimelineTool tool = TimelineToolStrip::toolAt(index);
        const auto s = state(QString::fromLatin1(kStripCommands[index]));
        m_tools->followTool(tool, s.enabled, s.reason, s.shortcut);
    }
    using Control = TimelineToolStrip::Control;
    const std::pair<Control, int> controls[] = {{Control::Snap, 6}, {Control::Marker, 7}, {Control::Track, 8}, {Control::Ask, 9}};
    for (const auto &[control, index] : controls) {
        const auto s = state(QString::fromLatin1(kStripCommands[index]));
        m_tools->followControl(control, s.enabled, s.reason, s.shortcut);
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
    if (m_window && watched == m_window->m_timelineDetail && m_tools) {
        switch (event->type()) {
        case QEvent::ParentChange:
            // Inserted into its new strip after this event: place the tools then.
            m_tools->hide();
            if (m_tools->parentWidget() != m_window->m_timelineDetail->parentWidget())
                m_tools->setParent(m_window->m_timelineDetail->parentWidget());
            QTimer::singleShot(0, this, &TimelineWiring::placeTools);
            break;
        case QEvent::Show: placeTools(); break;
        case QEvent::Hide: m_tools->hide(); break;
        default: break;
        }
    }
    return QObject::eventFilter(watched, event);
}

void TimelineWiring::placeTools() {
    if (!m_tools || !m_window || !m_window->m_timelineDetail) return;
    QLabel *detail = m_window->m_timelineDetail;
    auto *bar = qobject_cast<ads::CDockAreaTitleBar *>(detail->parentWidget());
    if (!bar) {
        m_tools->hide();
        return;
    }
    if (m_tools->parentWidget() != bar || bar->indexOf(m_tools) < 0) {
        const int at = bar->indexOf(detail);
        if (at < 0) return;
        bar->insertWidget(at + 1, m_tools);
    }
    m_tools->setVisible(!detail->isHidden());
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
    refreshAsk();
}

void TimelineWiring::refreshAsk() {
    if (!m_tools || !m_window) return;
    bindCommands();
    const ReviewModel *model = review();
    m_tools->setAskNotes(m_window->chatOpen() ? 0 : model->counts().open);
}

void TimelineWiring::refreshPatch() {
    if (!m_window || !m_window->m_timeline) return;
    QHash<QString, QString> patch;
    if (m_window->m_sourceMonitor && m_window->m_sourceMonitor->isOpen()) {
        // Where the Source viewer's picture and sound go: v1 and a1 on the
        // tracks a source edit writes.
        int video = 0;
        int audio = 0;
        const QJsonArray tracks = m_window->m_project.value(QStringLiteral("tracks")).toArray();
        for (const QString &id : m_window->sourceTargets()) {
            for (const QJsonValue &value : tracks) {
                const QJsonObject track = value.toObject();
                if (track.value(QStringLiteral("id")).toString() != id) continue;
                patch.insert(id, isAudioTrack(track) ? QStringLiteral("a%1").arg(++audio) : QStringLiteral("v%1").arg(++video));
            }
        }
    }
    m_window->m_timeline->setSourcePatch(patch);
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
