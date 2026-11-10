// MainWindow's picture region: the viewer with its overlays (the agent HUD and
// the preview chip), safe zones, the frame steps and the masks. The region's
// own state and its bus traffic are StageWiring's.

#include "main_window.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "inspector_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "project_sidebar.h"
#include "review_model.h"
#include "source_monitor_panel.h"
#include "stage_shell.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#ifdef Q_OS_MACOS
#include "macos_window.h"
#endif

#include <QAbstractTextDocumentLayout>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QClipboard>
#include <QColorDialog>
#include <QCursor>
#include <QDateTime>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QFile>
#include <QFileDialog>
#include <QFileInfo>
#include <QFontDatabase>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QIcon>
#include <QImage>
#include <QJsonDocument>
#include <QKeySequence>
#include <QLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QPolygonF>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QProcess>
#include <QRegularExpression>
#include <QSaveFile>
#include <QScrollBar>
#include <QSettings>
#include <QScreen>
#include <QShortcut>
#include <QShowEvent>
#include <QSignalBlocker>
#include <QTextBlock>
#include <QTextDocument>
#include <QTextFrame>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>


#include <algorithm>
#include <numeric>
#include <limits>
#include <utility>
#include "wiring/main_window_shared.h"
#include "browser_panel.h"
#include "editor_bus.h"
#include "motion_path.h"
#include "inspector_column.h"
#include "wiring/review_wiring.h"
#include "stage_motion.h"
#include "wiring/stage_wiring.h"
#include "wiring/timeline_wiring.h"

namespace kerf {

using namespace window_parts;

void MainWindow::stepFrames(int frames) {
    const double rate = std::max(1.0, m_fps);
    queueSeek((std::round(m_playhead * rate) + frames) / rate);
}

void MainWindow::updateViewerMasks() {
    bool fit = false;
    const QSizeF picture = m_connected ? selectedPictureSize(&fit) : QSizeF();
    if (picture.isEmpty()) {
        for (QAction *action : m_maskTools) action->setEnabled(false);
        m_viewer->setMasks(QString(), QJsonArray{}, QTransform{});
        m_viewer->setMotionPath(QString(), QString(), QTransform{}, 0.0);
        m_viewer->setSampledPath({}, std::nullopt);
        return;
    }
    const QJsonObject clip = selectedClipObject();
    const double clipTime = std::clamp(m_playhead - clip.value(QStringLiteral("timelineStart")).toDouble(), 0.0,
                                       clip.value(QStringLiteral("duration")).toDouble());
    const QSizeF canvas(resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray()));
    const QTransform parents = parentsMotion(clip, canvas, clip.value(QStringLiteral("timelineStart")).toDouble() + clipTime);
    const QTransform placement = picturePlacement(clip, picture, fit, canvas, clipTime, parents, m_project);
    // A picture that cannot be seen cannot be drawn on.
    for (QAction *action : m_maskTools) action->setEnabled(placement.isInvertible());
    m_viewer->setMasks(m_selectedClip, clip.value(QStringLiteral("masks")).toArray(), placement);
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    m_viewer->setMotionPath(m_selectedClip, props.value(QStringLiteral("path")).toString(),
                            pathPlacement(clip, picture, fit, canvas, clipTime, parents, m_project), propertyAt(props, QStringLiteral("along"), clipTime, 0.0));
    // A path facing along itself from an anchor off the middle of a turned clip: no one map shows it.
    const QTransform onPath = pathPlacement(clip, picture, fit, canvas, clipTime, parents, m_project);
    const bool sampled = props.value(QStringLiteral("three_d")).toBool() && !onPath.isInvertible() && props.value(QStringLiteral("orient")).toBool() && !props.value(QStringLiteral("path")).toString().isEmpty();
    if (!sampled) {
        m_viewer->setSampledPath({}, std::nullopt);
        return;
    }
    constexpr int kSampledPathSteps = 96;
    const auto places = sampledPath(clip, picture, fit, canvas, clipTime, parents, m_project, kSampledPathSteps);
    const double along = propertyAt(props, QStringLiteral("along"), clipTime, 0.0);
    m_viewer->setSampledPath(places, places.value(int(std::lround(std::clamp(along, 0.0, 1.0) * kSampledPathSteps))));
    // Each point goes where a drag puts it, by solving from the clip's own placement there.
    const auto points = motionpath::parsePath(props.value(QStringLiteral("path")).toString());
    if (!points) return;
    const QJsonObject timeline = m_project;
    ViewerWidget::SampledPathEdit edit;
    edit.clip = m_selectedClip;
    edit.points = *points;
    edit.solve = [=](const QList<QPointF> &now, int index, const QPointF &seen) {
        return sampledPathPointFor(clip, picture, fit, canvas, clipTime, parents, timeline, now, index, seen);
    };
    edit.curve = [=](const QList<QPointF> &now) {
        return sampledPath(withPathPoints(clip, now), picture, fit, canvas, clipTime, parents, timeline, kSampledPathSteps);
    };
    edit.handles = [=](const QList<QPointF> &now) { return sampledPathPoints(clip, picture, fit, canvas, clipTime, parents, timeline, now); };
    m_viewer->setSampledPathEdit(edit);
}

QTransform MainWindow::parentsMotion(const QJsonObject &clip, const QSizeF &canvas, double at) const {
    QTransform moved;
    QStringList seen;
    QString parent = clip.value(QStringLiteral("parent")).toString();
    // As `Scene::geometry` goes: up the chain until one is not there to follow.
    while (!parent.isEmpty() && !seen.contains(parent)) {
        seen << parent;
        QJsonObject rig;
        QString type;
        for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray())
                if (value.toObject().value(QStringLiteral("id")).toString() == parent) {
                    rig = value.toObject();
                    type = track.toObject().value(QStringLiteral("type")).toString();
                }
        bool fit = false;
        const QSizeF picture = pictureSizeOf(rig, type, &fit);
        if (picture.isEmpty()) break;
        moved *= parentMotion(rig, picture, fit, canvas, at - rig.value(QStringLiteral("timelineStart")).toDouble());
        parent = rig.value(QStringLiteral("parent")).toString();
    }
    return moved;
}

void MainWindow::startMotionPath() {
    const QJsonObject clip = selectedClipObject();
    if (!canStartMotionPath(clip)) return;
    bool fit = false;
    const QSizeF picture = selectedPictureSize(&fit);
    if (picture.isEmpty()) return;
    const QSizeF canvas(resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray()));
    // From where the clip is, a quarter of the canvas towards the middle of it.
    const double clipTime = std::clamp(m_playhead - clip.value(QStringLiteral("timelineStart")).toDouble(), 0.0,
                                       clip.value(QStringLiteral("duration")).toDouble());
    const double from = pathPlacement(clip, picture, fit, canvas, clipTime, QTransform{}, m_project).map(QPointF()).x();
    QJsonObject props{{QStringLiteral("path"), motionpath::pathText({{0.0, 0.0}, {from > 0.5 ? -0.25 : 0.25, 0.0}})}};
    // Along it over the clip, unless something already says how far along it is.
    if (!clip.value(QStringLiteral("props")).toObject().contains(QStringLiteral("along"))) {
        const qint64 last = std::max<qint64>(1, qint64(clip.value(QStringLiteral("durationFrames")).toDouble()) - 1);
        const auto key = [](qint64 frame, double value) {
            return QJsonObject{{QStringLiteral("at"), double(frame)}, {QStringLiteral("value"), value}, {QStringLiteral("interp"), QStringLiteral("linear")}};
        };
        props.insert(QStringLiteral("along"), QJsonObject{{QStringLiteral("keys"), QJsonArray{key(0, 0.0), key(last, 1.0)}}});
    }
    sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")}, {QStringLiteral("clip"), m_selectedClip}, {QStringLiteral("props"), props}}});
}

bool MainWindow::canStartMotionPath(const QJsonObject &clip) const {
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    bool fit = false;
    return !selectedPictureSize(&fit).isEmpty() && props.value(QStringLiteral("path")).toString().isEmpty();
}

void MainWindow::setSafeZones(bool shown) {
    m_viewer->setSafeZones(shown);
    QSettings().setValue(QStringLiteral("viewer/safeZones"), shown);
}

// The picture: the viewer, its HUD and preview chip (StageWiring).
void MainWindow::buildStage() {
    m_viewer = new ViewerWidget;
    // A click on the picture runs Play, as the key does.
    connect(m_viewer, &ViewerWidget::playRequested, this, [this] { triggerCommand(QStringLiteral("playback.toggle")); });
    m_stageWiring->build();
    m_viewer->setSafeZones(QSettings().value(QStringLiteral("viewer/safeZones"), false).toBool());
}

// The picture's commands: the mask tools' keys (⇧R ⇧E ⇧P, wherever the window
// has the focus), and the switches' state. Then the region's wiring.
void MainWindow::setupPictureCommands() {
    const QList<QKeySequence> maskKeys{QKeySequence(Qt::SHIFT | Qt::Key_R), QKeySequence(Qt::SHIFT | Qt::Key_E),
                                       QKeySequence(Qt::SHIFT | Qt::Key_P)};
    for (int index = 0; index < m_maskTools.size() && index < maskKeys.size(); ++index) {
        QAction *tool = m_maskTools.at(index);
        if (tool->shortcut().isEmpty()) tool->setShortcut(maskKeys.at(index));
        if (!actions().contains(tool)) addAction(tool);
    }
    if (QAction *safe = m_commands->action(QStringLiteral("view.safeZones"))) {
        const QSignalBlocker blocker(safe);
        safe->setChecked(m_viewer->safeZonesShown());
    }
    m_stageWiring->attach();
}

// The sidebar, and what opens the panel beside the conversation.
void MainWindow::buildSidebar() {
    m_sidebar = new ProjectSidebar;
    m_shell->setSidebar(m_sidebar);
    // A project starts as a conversation; followContentWithPanes opens the
    // panel with the first thing to show.
    stage::Panes panes;
    panes.sidebar = QSettings().value(QStringLiteral("layout/sidebar"), true).toBool();
    panes.picture = false;
    panes.timeline = false;
    m_shell->setPanes(panes);
    connect(m_shell, &StageShell::panesChanged, this, &MainWindow::applyPanes);
    connect(m_sidebar, &ProjectSidebar::projectChosen, this, [this](const QString &path) { openProject(path); });
    connect(m_sidebar, &ProjectSidebar::newProjectRequested, this, &MainWindow::newProject);
    connect(m_sidebar, &ProjectSidebar::openFolderRequested, this, &MainWindow::openProjectFolder);
    connect(m_sidebar, &ProjectSidebar::conversationChosen, this, [this](const QString &id) { m_chat->openThread(id); });
    connect(m_sidebar, &ProjectSidebar::newConversationRequested, this, [this] {
        m_chat->setReplyTarget(QString());
        if (ReviewModel *review = m_reviewWiring->model()) review->setCurrentThread(QString());
        m_chat->focusComposer();
    });
    if (ReviewModel *review = m_reviewWiring->model()) {
        connect(review, &ReviewModel::changed, this, &MainWindow::refreshSidebar);
        connect(review, &ReviewModel::currentChanged, this, &MainWindow::refreshSidebar);
    }
    // Looking at something in the conversation shows it.
    connect(m_chat, &ChatPanel::seekRequested, this, [this] { revealPanel(true, false); });
    connect(m_chat, &ChatPanel::previewRequested, this, [this](const QString &, bool on) {
        if (on) revealPanel(true, false);
    });
    // Media and Effects drop on the timeline.
    connect(m_shell, &StageShell::sheetChanged, this, [this](StageShell::Sheet sheet) {
        if ((sheet == StageShell::Sheet::Media || sheet == StageShell::Sheet::Browser) && !m_shell->panes().panelOpen()) revealPanel(true, true);
    });
    refreshSidebar();
}

void MainWindow::setPaneOpen(const QString &command, bool open) {
    stage::Panes panes = m_shell->panes();
    if (command == QStringLiteral("view.sidebar")) {
        panes.sidebar = open;
        QSettings().setValue(QStringLiteral("layout/sidebar"), open);
    } else {
        (command == QStringLiteral("view.picture") ? panes.picture : panes.timeline) = open;
        m_panesFollowContent = false;
    }
    m_shell->setPanes(panes);
    applyPanes();
}

void MainWindow::revealPanel(bool picture, bool timeline, const QRect &origin) {
    stage::Panes panes = m_shell->panes();
    panes.picture = panes.picture || picture;
    panes.timeline = panes.timeline || timeline;
    m_shell->setPanes(panes, origin);
}

void MainWindow::applyPanes() {
    // The block under the picture holds the transport when there is one.
    if (m_timelineWiring) m_timelineWiring->setWide(m_shell->rects().transport.isEmpty());
    syncPanelCommands();
    const stage::Panes &panes = m_shell->panes();
    for (const auto &[button, open] : {std::pair<ui::IconButton *, bool>{m_sidebarButton, panes.sidebar},
                                       {m_pictureButton, panes.picture},
                                       {m_timelineButton, panes.timeline},
                                       {m_followButton, m_followKerf}})
        if (button) button->setOn(open);
    // A note stamps and draws on the picture.
    if (Composer *composer = m_chat ? m_chat->composerWidget() : nullptr) {
        composer->stamp()->setVisible(panes.picture);
        composer->drawButton()->setVisible(panes.picture);
    }
}

void MainWindow::followContentWithPanes() {
    if (!m_panesFollowContent) return;
    bool content = !m_proposalId.isEmpty();
    for (const QJsonValue &track : m_project.value(QStringLiteral("tracks")).toArray())
        content = content || !track.toObject().value(QStringLiteral("clips")).toArray().isEmpty();
    if (!content) return;
    m_panesFollowContent = false;
    // The picture grows out of the tile of the first thing it was going to show,
    // over the conversation, which gives way as it does. The timeline follows.
    QString firstClip;
    const QJsonObject cut = proposalTimelineFor(m_snapshot, m_proposalId);
    for (const QJsonObject &timeline : {cut, m_project})
        for (const QJsonValue &track : timeline.value(QStringLiteral("tracks")).toArray()) {
            if (track.toObject().value(QStringLiteral("type")).toString() != QStringLiteral("video")) continue;
            const QJsonArray clips = track.toObject().value(QStringLiteral("clips")).toArray();
            if (firstClip.isEmpty() && !clips.isEmpty()) firstClip = clips.first().toObject().value(QStringLiteral("id")).toString();
        }
    const QRect origin = flightStart(sourceOfClip(firstClip));
    const QImage first = posterForClip(firstClip);
    revealPanel(true, true, origin);
    if (!first.isNull()) {
        StageMotion::Flight grow;
        grow.image = first;
        grow.from = origin;
        grow.to = m_shell->rects().picture;
        grow.lively = false;
        // It holds the place until the first frame is there.
        grow.stays = true;
        grow.fit = true;
        grow.radius = tokens::layout::pictureRadius;
        grow.duration = tokens::motion::stage;
        m_motion->fly(grow);
    }
    if (!ui::reducedMotion() && isVisible()) {
        m_shell->veilTimeline(1.0, 0, 0);
        m_shell->veilTimeline(0.0, tokens::motion::stage - tokens::motion::timelineLead, tokens::motion::settle);
        m_holdFlights = true;
        QTimer::singleShot(tokens::motion::stage, this, [this] {
            m_holdFlights = false;
            flushFlights();
        });
    }
}

void MainWindow::refreshSidebar() {
    if (!m_sidebar) return;
    m_sidebar->setProjects(ProjectLauncher::recentProjects(), m_projectDirectory);
    QList<ProjectSidebar::Conversation> conversations;
    QString current;
    if (ReviewModel *review = m_reviewWiring->model()) {
        current = review->currentThread();
        for (const ReviewModel::Thread &thread : review->allThreads()) {
            if (thread.origin == ReviewModel::Origin::Welcome) continue;
            // A proposal joins its conversation when the turn that staged it ends.
            if (thread.origin == ReviewModel::Origin::Proposal && m_agentBusy) continue;
            ProjectSidebar::Conversation conversation;
            conversation.id = thread.id;
            conversation.title = thread.root.body.section(QLatin1Char('\n'), 0, 0).trimmed();
            if (conversation.title.isEmpty()) conversation.title = QStringLiteral("Proposal");
            const bool working = std::any_of(m_versionRuns.cbegin(), m_versionRuns.cend(), [&thread](const VersionRun &run) {
                return run.thread == thread.id && run.state == CompareView::State::Running;
            });
            conversation.state = working                        ? ProjectSidebar::State::Working
                                 : !thread.openProposal.isEmpty() ? ProjectSidebar::State::Review
                                 : thread.resolved()            ? ProjectSidebar::State::Settled
                                                                : ProjectSidebar::State::Idle;
            conversations << conversation;
        }
    }
    m_sidebar->setConversations(conversations, current);
}

void MainWindow::startWith(const QString &request, bool importFolder) {
    m_startRequest = request;
    m_startImport = importFolder;
    runStart();
}

void MainWindow::runStart() {
    // Kerf is at work on something else: this keeps until the next snapshot.
    if (!m_connected || m_snapshot.isEmpty() || m_agentBusy) return;
    if (m_startImport) {
        m_startImport = false;
        const QDir folder(m_projectDirectory);
        const QStringList files = folder.entryList({QStringLiteral("*.mov"), QStringLiteral("*.mp4"), QStringLiteral("*.m4v"), QStringLiteral("*.mkv"),
                                                    QStringLiteral("*.webm"), QStringLiteral("*.wav"), QStringLiteral("*.mp3")},
                                                   QDir::Files, QDir::Name);
        // Into the project's media, with the timeline left for Kerf.
        for (const QString &file : files)
            sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("import")},
                                   {QStringLiteral("path"), folder.absoluteFilePath(file)},
                                   {QStringLiteral("mediaOnly"), true}});
        if (!files.isEmpty()) addActivity(QStringLiteral("import"), QStringLiteral("Importing %1 files from the folder").arg(files.size()));
    }
    if (m_startRequest.isEmpty()) return;
    const QString request = m_startRequest;
    m_startRequest.clear();
    // Kerf cannot take it (no connection chosen yet): it waits in the composer.
    if (!sendAgent(request, QJsonObject{}, QString())) m_chat->composerWidget()->fill(request);
}

} // namespace kerf
