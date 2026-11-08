#include "main_window.h"

#include "comment_row.h"
#include "review_link.h"
#include "secret_store.h"
#include "stage_motion.h"
#include "ask_prompt.h"
#include "bridge.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "inspector_panel.h"
#include "effects_panel.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "change_reel.h"
#include "project_sidebar.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "stage_shell.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "browser_panel.h"
#include "deliver_tray.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "graph_editor.h"
#include "deliver_pane.h"
#include "inspector_column.h"
#include "wiring/main_window_shared.h"
#include "wiring/bin_wiring.h"
#include "wiring/inspector_wiring.h"
#include "wiring/review_wiring.h"
#include "wiring/source_wiring.h"
#include "wiring/stage_wiring.h"
#include "wiring/timeline_wiring.h"
#include "wiring/toolbar_wiring.h"
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
#include <QTextEdit>
#include <QLineEdit>
#include <QAbstractSpinBox>
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

namespace kerf {

using namespace window_parts;

MainWindow::MainWindow(const QString &projectDirectory, const QString &kerfdBinary, QWidget *parent)
    : QMainWindow(parent), m_projectDirectory(projectDirectory), m_kerfdBinary(kerfdBinary) {
    resize(1500, 940);
    setMinimumSize(1080, 700);
#if KERF_EDITOR_OWN_TITLE_BAR
    setWindowFlag(Qt::ExpandedClientAreaHint);
    setWindowFlag(Qt::NoTitleBarBackgroundHint);
    // The toolbar fills the window to its top edge; fitTitleBar keeps its
    // commands clear of the window buttons.
    setAttribute(Qt::WA_ContentsMarginsRespectsSafeArea, false);
#endif
    // The regions' bus: requests go out as the window's own do.
    m_bus = new EditorBus(this);
    m_bus->setSender([this](const QJsonObject &command) { return sendSimple(command); });
    m_bus->setRunner([this](const QString &id, const QJsonObject &params) { return runCommand(id, params); });
    m_reviewWiring = std::make_unique<ReviewWiring>(this, m_bus);
    m_stageWiring = std::make_unique<StageWiring>(this, m_bus);
    m_toolbarWiring = std::make_unique<ToolbarWiring>(this, m_bus);
    m_inspectorWiring = std::make_unique<InspectorWiring>(this, m_bus);
    m_timelineWiring = std::make_unique<TimelineWiring>(this, m_bus);
    m_binWiring = std::make_unique<BinWiring>(this, m_bus);
    m_sourceWiring = std::make_unique<SourceWiring>(this, m_bus);
    buildUi();
    setupActions();
    updateCloudStatus();
    // The folder's name until the first snapshot names the project.
    setProjectTitle(m_projectDirectory.isEmpty() ? QStringLiteral("Industrial Kerf") : QFileInfo(m_projectDirectory).fileName());
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, &MainWindow::applyAppearance);
    m_exactSeekTimer.setSingleShot(true);
    connect(&m_exactSeekTimer, &QTimer::timeout, this, &MainWindow::settlePointerSeek);
    setConnection(QStringLiteral("connecting"));
#ifndef KERF_QT_TESTING
    m_bridge = kerf_bridge_new(m_projectDirectory.toUtf8().constData(), m_kerfdBinary.toUtf8().constData());
    if (!m_bridge) {
        setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
    }
    m_pollTimer.setInterval(8);
    connect(&m_pollTimer, &QTimer::timeout, this, &MainWindow::pollBridge);
    m_pollTimer.start();
#endif
}

MainWindow::~MainWindow() {
    // The composer follows the focus no more: its widgets go before the window.
    disconnect(qApp, &QApplication::focusChanged, this, nullptr);
    if (m_bridge) kerf_bridge_free(m_bridge);
}

void MainWindow::pollBridge() {
    flushSeek();
    if (!m_bridge) return;
    for (int count = 0; count < 32; ++count) {
        char *raw = kerf_bridge_poll(m_bridge);
        if (!raw) break;
        const QByteArray bytes(raw);
        kerf_bridge_string_free(raw);
        const QJsonDocument doc = QJsonDocument::fromJson(bytes);
        if (doc.isObject()) handleEvent(doc.object());
    }
    KerfFrame frame{};
    if (kerf_bridge_frame(m_bridge, &frame)) showProgramFrame(frame);
    KerfSignal signal{};
    if (kerf_bridge_signal(m_bridge, &signal) && signal.pixels && signal.width && signal.height &&
        signal.length >= size_t(signal.width) * size_t(signal.height) * 8) {
        const QImage image(signal.pixels, int(signal.width), int(signal.height), int(signal.width) * 8, QImage::Format_RGBA64);
        m_bus->setProgramSignal(image.copy(), int(signal.curve));
    }
}

bool MainWindow::showProgramFrame(const KerfFrame &frame) {
    if (!m_connected || !frame.pixels || frame.length == 0 || frame.width == 0 || frame.height == 0) return false;
    const qsizetype expected = static_cast<qsizetype>(frame.width) * static_cast<qsizetype>(frame.height) * 4;
    if (expected <= 0 || frame.length < static_cast<size_t>(expected)) return false;
    const double target = m_hoverSkimming ? m_pendingHoverSeek : m_playhead;
    if ((m_hoverSkimming || m_pointerOwnsPlayhead) && !nearerScrubFrame(m_shownFrameTime, m_hasShownFrame, frame.time, target, m_fps))
        return false;
    QImage image(frame.pixels, static_cast<int>(frame.width), static_cast<int>(frame.height), static_cast<int>(frame.width * 4),
                 QImage::Format_RGBA8888);
    m_shownFrameTime = frame.time;
    m_hasShownFrame = true;
    const QImage shown = image.copy();
    m_viewer->setFrame(shown, frame.time);
    // The sequence's thumbnail in the Inspector, with nothing selected.
    if (m_selectedClip.isEmpty()) m_inspector->setProgramFrame(shown);
    // The frame the viewer got, for the scopes and the regions.
    m_bus->setProgramFrame(shown);
    return true;
}

void MainWindow::importMedia() {
    const QString path = QFileDialog::getOpenFileName(this, QStringLiteral("Import media"), QString(),
                                                       QStringLiteral("Media files (*.mov *.mp4 *.m4v *.mkv *.webm *.wav *.mp3);;All files (*)"));
    if (path.isEmpty()) return;
    if (!execute(QStringLiteral("file.import"), QJsonObject{{QStringLiteral("path"), path}})) return;
    addActivity(QStringLiteral("import"), QStringLiteral("Importing %1").arg(QFileInfo(path).fileName()));
}

void MainWindow::exportProject() {
    // The quality chosen in Export's menu: Master unless Draft was picked.
    const QString quality = m_exportButton->quality();
    if (!execute(QStringLiteral("file.export"), QJsonObject{{QStringLiteral("quality"), quality}})) return;
    addActivity(QStringLiteral("export"), QStringLiteral("Export started (%1)").arg(m_exportButton->qualityName()));
}

void MainWindow::renderPreview() {
    if (!execute(QStringLiteral("sequence.renderPreview"))) return;
    addActivity(QStringLiteral("preview"), QStringLiteral("Rendering preview"));
    m_previewStatus = QStringLiteral("running");
    updateCommandStates();
}

void MainWindow::togglePlayback() {
    if (!m_connected || m_previewRequired || m_duration <= 0) return;
    if (!m_playing && m_hoverSkimming) {
        // Skimming is a viewer-only preview. Restore the edit playhead
        // before starting playback so hover never changes its start point.
        m_hasPendingHoverSeek = false;
        m_hoverSkimming = false;
        m_timeline->clearSkimmer();
        sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")},
                               {QStringLiteral("position"), m_playhead}});
    }
    sendSimple(QJsonObject{{QStringLiteral("type"), m_playing ? QStringLiteral("pause") : QStringLiteral("play")} });
}

QJsonObject MainWindow::withClipNames(QJsonObject timeline, const QJsonArray &sources) {
    QHash<QString, QJsonObject> sourceById;
    for (const auto &value : sources) { const auto source = value.toObject(); sourceById.insert(source.value("id").toString(), source); }
    const QHash<QString, QString> names = sourceNames(sources);
    QJsonArray displayTracks;
    for (const auto &value : timeline.value("tracks").toArray()) {
        auto track = value.toObject(); QJsonArray displayClips;
        for (const auto &clipValue : track.value("clips").toArray()) {
            auto clip = clipValue.toObject(); const auto source = sourceById.value(clip.value("sourceId").toString());
            // Named after its media's file; a clip of media with no path
            // keeps no stand-in name (the Inspector and the timeline fall
            // back on their own).
            if (!clip.contains("name") && !source.value("path").toString().isEmpty()) clip.insert("name", names.value(source.value("id").toString()));
            displayClips.append(clip);
        }
        track.insert("clips", displayClips); displayTracks.append(track);
    }
    timeline.insert("tracks", displayTracks);
    return timeline;
}

void MainWindow::queueSeek(double time) {
    if (!m_connected) return;
    m_hasPendingHoverSeek = false;
    m_hoverSkimming = false;
    m_timeline->clearSkimmer();
    m_pendingSeek = std::clamp(time, 0.0, std::max({m_duration, m_seekLimit, m_proposedDuration}));
    m_pendingSeekScrub = scrubFromShownFrame(m_pendingSeek);
    m_hasPendingSeek = true;
    m_pointerOwnsPlayhead = true;
    m_playhead = m_pendingSeek;
    m_timeline->setPlayhead(m_playhead);
    m_bus->setPlayhead(m_playhead);
    updateCommandStates();
    m_viewer->setLiveScrub(true);
    if (m_pendingSeekScrub) m_exactSeekTimer.start(50);
    else m_exactSeekTimer.stop();
    flushSeek();
}

void MainWindow::queueHoverSeek(double time) {
    if (!m_connected || m_previewRequired || m_playing || m_duration <= 0.0) return;
    m_pendingHoverSeek = std::clamp(time, 0.0, m_duration);
    m_pendingHoverScrub = scrubFromShownFrame(m_pendingHoverSeek);
    m_hasPendingHoverSeek = true;
    m_hoverSkimming = true;
    m_pointerOwnsPlayhead = true;
    m_viewer->setLiveScrub(true);
    if (m_pendingHoverScrub) m_exactSeekTimer.start(50);
    else m_exactSeekTimer.stop();
    flushSeek();
}

void MainWindow::endHoverSeek() {
    const bool wasSkimming = m_hoverSkimming || m_hasPendingHoverSeek;
    m_hasPendingHoverSeek = false;
    m_hoverSkimming = false;
    m_exactSeekTimer.stop();
    if (!wasSkimming || !m_connected || m_playing) {
        m_pointerOwnsPlayhead = false;
        m_viewer->setLiveScrub(false);
        return;
    }
    // Keep the pointer in charge until the edit playhead's frame arrives,
    // so a late skim picture cannot jump back onto the viewer.
    m_pointerOwnsPlayhead = true;
    m_viewer->setLiveScrub(true);
    m_pendingSeek = m_playhead;
    m_pendingSeekScrub = false;
    m_hasPendingSeek = true;
    m_exactSeekTimer.start(50);
    flushSeek();
}

void MainWindow::settlePointerSeek() {
    if (m_hoverSkimming) {
        m_pendingHoverSeek = m_timeline->skimmerTime();
        m_pendingHoverScrub = false;
        m_hasPendingHoverSeek = true;
        flushSeek();
        return;
    }
    if (!m_pointerOwnsPlayhead) return;
    m_pendingSeek = m_playhead;
    m_pendingSeekScrub = false;
    m_hasPendingSeek = true;
    m_pointerOwnsPlayhead = false;
    m_viewer->setLiveScrub(false);
    flushSeek();
}

bool MainWindow::scrubFromShownFrame(double time) const {
    if (!m_hasShownFrame) return true;
    const double delta = time - m_shownFrameTime;
    return delta < -(1.0 / std::max(1.0, m_fps)) || delta > 0.40;
}

void MainWindow::selectClip(const QString &id) {
    m_selectedClip = id;
    m_selectedClips = id.isEmpty() ? QStringList{} : QStringList{id};
    m_timeline->setSelectedClip(id);
    updateInspector();
    updateCommandStates();
    updateChatControls();
    m_bus->setSelection(m_selectedClips);
}

void MainWindow::extendClipSelection(const QString &id, bool toggle) {
    // ⌘-click toggles a clip in the selection, ⇧-click adds it; the clip
    // clicked is the primary one (the one single-clip commands act on).
    QStringList clips = m_selectedClips;
    if (clips.isEmpty() && !m_selectedClip.isEmpty()) clips << m_selectedClip;
    if (!clips.contains(id)) clips << id;
    else if (toggle) clips.removeAll(id);
    else clips.move(clips.indexOf(id), clips.size() - 1);
    if (clips.isEmpty()) {
        selectClip(QString());
        return;
    }
    m_selectedClip = clips.constLast();
    m_selectedClips = clips;
    m_timeline->setSelectedClips(clips);
    updateInspector();
    updateCommandStates();
    updateChatControls();
    m_bus->setSelection(m_selectedClips);
}

bool MainWindow::sendEdit(const QJsonArray &ops) {
    if (ops.isEmpty()) return false;
    return sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("edit")},
                                  {QStringLiteral("revision"), revisionValue()},
                                  {QStringLiteral("ops"), ops}});
}

QJsonObject MainWindow::clipObject(const QString &id) const {
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (clip.value(QStringLiteral("id")).toString() == id) return clip;
        }
    }
    return {};
}

qint64 MainWindow::clipFrames(const QJsonObject &clip, const QString &key) {
    return static_cast<qint64>(clip.value(key).toDouble());
}

bool MainWindow::usesCloud() const {
    return m_cloudRadio && m_cloudRadio->isChecked();
}

bool MainWindow::usesClaudeCode() const {
    return m_claudeCodeRadio && m_claudeCodeRadio->isChecked();
}

bool MainWindow::sendCloud(const QJsonObject &command) {
    if (!m_bridge) {
        m_lastCommand = command;
        return false;
    }
    const QByteArray payload = QJsonDocument(command).toJson(QJsonDocument::Compact);
    return kerf_bridge_send(m_bridge, payload.constData());
}

QString MainWindow::aiAccess() const {
    if (usesCloud()) return m_cloudAccess.isEmpty() ? QStringLiteral("unavailable") : m_cloudAccess;
    // Claude Code signs in on its own; a turn says so if it is not.
    if (usesClaudeCode()) return QStringLiteral("ready");
    const bool openai = activeProviderId() == QStringLiteral("openai");
    const QString key = (openai ? m_openaiKey : m_anthropicKey)->text().trimmed();
    const bool environment = !qEnvironmentVariable(openai ? "OPENAI_API_KEY" : "ANTHROPIC_API_KEY").isEmpty();
    return key.isEmpty() && !environment ? QStringLiteral("noKey") : QStringLiteral("ready");
}

void MainWindow::updateCloudStatus() {
    if (!m_cloudStatus) return;
    if (usesClaudeCode()) {
        // Nothing of Kerf Cloud's applies: Claude Code keeps its own sign-in.
        m_cloudStatus->setText(QStringLiteral("Chat runs your installed Claude Code with only this project's Kerf tools, on the Claude plan it is signed in to. Sign in once by running claude in a terminal."));
        m_cloudToken->hide();
        m_cloudSignIn->hide();
        m_cloudSignOut->hide();
        updateCommandStates();
        return;
    }
    const QString email = m_cloudAccount.value(QStringLiteral("email")).toString();
    const bool signedIn = !email.isEmpty() && m_cloudAccess != QStringLiteral("signedOut");
    QString text;
    if (m_cloudAccess == QStringLiteral("unavailable") || m_cloudAccess.isEmpty()) {
        text = QStringLiteral("Kerf Cloud isn't available yet. Until it is, choose My own API key and add it under Advanced.");
    } else if (!signedIn) {
        text = m_cloudAccess == QStringLiteral("unreachable") ? QStringLiteral("Kerf Cloud can't be reached right now.")
                                                               : QStringLiteral("Signed out. Paste your Kerf Cloud token to sign in.");
    } else {
        const QJsonValue credits = m_cloudAccount.value(QStringLiteral("credits"));
        text = QStringLiteral("Signed in as %1  ·  %2 plan%3").arg(email, m_cloudAccount.value(QStringLiteral("plan")).toString(),
            credits.isDouble() ? QStringLiteral("  ·  %1 requests left").arg(credits.toInteger()) : QString());
        if (m_cloudAccess == QStringLiteral("notIncluded")) text += QStringLiteral("\nThis plan doesn't include Kerf Cloud requests.");
        if (m_cloudAccess == QStringLiteral("outOfCredits")) text += QStringLiteral("\nYou've used this period's requests.");
    }
    if (!m_cloudMessage.isEmpty()) text += QStringLiteral("\n") + m_cloudMessage;
    m_cloudStatus->setText(text);
    const bool available = !m_cloudAccess.isEmpty() && m_cloudAccess != QStringLiteral("unavailable");
    m_cloudToken->setVisible(available && !signedIn);
    m_cloudSignIn->setVisible(available && !signedIn);
    m_cloudSignOut->setVisible(available && signedIn);
    updateCommandStates();
}

QString MainWindow::activeProviderId() const {
    return m_openaiActiveRadio && m_openaiActiveRadio->isChecked() ? QStringLiteral("openai")
                                                                   : QStringLiteral("anthropic");
}

QJsonObject MainWindow::agentSettingsPayload() const {
    if (usesCloud()) {
        // The service picks the model; MCP tools still run locally.
        return QJsonObject{{QStringLiteral("cloud"), true},
                           {QStringLiteral("mcpEnabled"), m_mcpEnabled && m_mcpEnabled->isChecked()},
                           {QStringLiteral("mcpConfigVersion"), static_cast<qint64>(m_mcpConfigVersion)}};
    }
    if (usesClaudeCode()) {
        // No key: Claude Code runs on the account it is signed in to.
        return QJsonObject{{QStringLiteral("provider"), QStringLiteral("claude-code")},
                           {QStringLiteral("model"), m_agentModel.trimmed()},
                           {QStringLiteral("mcpEnabled"), m_mcpEnabled && m_mcpEnabled->isChecked()},
                           {QStringLiteral("mcpConfigVersion"), static_cast<qint64>(m_mcpConfigVersion)}};
    }
    const bool openai = activeProviderId() == QStringLiteral("openai");
    const QLineEdit *key = openai ? m_openaiKey : m_anthropicKey;
    const QLineEdit *baseUrl = openai ? m_openaiBaseUrl : m_anthropicBaseUrl;
    return QJsonObject{{QStringLiteral("provider"), activeProviderId()},
                       {QStringLiteral("apiMode"), m_openaiMode->currentData().toString()},
                       {QStringLiteral("apiKey"), key ? key->text().trimmed() : QString()},
                       {QStringLiteral("baseUrl"), baseUrl ? baseUrl->text().trimmed() : QString()},
                       {QStringLiteral("model"), m_agentModel.trimmed()},
                       {QStringLiteral("mcpEnabled"), m_mcpEnabled && m_mcpEnabled->isChecked()},
                       {QStringLiteral("mcpConfigVersion"), static_cast<qint64>(m_mcpConfigVersion)}};
}

void MainWindow::loadInboundMcpConfig(const QString &path, bool imported) {
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not read %1: %2").arg(path, file.errorString()));
        return;
    }
    const QByteArray raw = file.readAll();
    if (file.error() != QFileDevice::NoError) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not read %1: %2").arg(path, file.errorString()));
        return;
    }
    m_inboundMcpJson->setPlainText(QString::fromUtf8(raw));
    const QString error = validateInboundMcpConfig(raw);
    m_inboundMcpStatus->setText(error.isEmpty()
        ? (imported ? QStringLiteral("Loaded %1 into the editor only. Review, then Save and apply.").arg(path)
                    : QStringLiteral("Loaded project config. Changes apply on the next chat request."))
        : QStringLiteral("Loaded %1. %2").arg(path, error));
}

void MainWindow::saveInboundMcpConfig() {
    const QByteArray raw = m_inboundMcpJson->toPlainText().toUtf8();
    const QString error = validateInboundMcpConfig(raw);
    if (!error.isEmpty()) {
        m_inboundMcpStatus->setText(error);
        return;
    }
    const QDir project(m_projectDirectory);
    if (!project.mkpath(QStringLiteral(".kerf"))) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not create the project's .kerf directory."));
        return;
    }
    QSaveFile file(project.filePath(QStringLiteral(".kerf/mcp.json")));
    if (!file.open(QIODevice::WriteOnly) || file.write(raw) != raw.size() || !file.commit()) {
        m_inboundMcpStatus->setText(QStringLiteral("Could not save MCP config: %1").arg(file.errorString()));
        return;
    }
    ++m_mcpConfigVersion;
    m_inboundMcpStatus->setText(QStringLiteral("Saved project .kerf/mcp.json. Applies on the next chat request when MCP is enabled."));
}

void MainWindow::updateAgentModelChoices() {
    static const QString cloudModel = QStringLiteral("Kerf Cloud");
    if (usesCloud()) {
        // The service picks the model: nothing to choose.
        m_agentModels = {cloudModel};
        m_agentModel = cloudModel;
        if (m_chat) m_chat->setModels(m_agentModels, m_agentModel, false);
        return;
    }
    if (usesClaudeCode()) {
        // Claude Code's own aliases; "default" leaves the choice to it.
        m_agentModels = {QStringLiteral("default"), QStringLiteral("opus"), QStringLiteral("sonnet"), QStringLiteral("haiku")};
        if (!m_agentModels.contains(m_agentModel)) m_agentModel = m_agentModels.constFirst();
        if (m_chat) m_chat->setModels(m_agentModels, m_agentModel, true);
        return;
    }
    // The active connection's model, else the one chosen, else its first.
    const bool openai = activeProviderId() == QStringLiteral("openai");
    const QString current = m_agentModel == cloudModel ? QString() : m_agentModel;
    const QString configured = openai ? m_openaiModel->text().trimmed() : m_anthropicModel->text().trimmed();
    m_agentModels = openai ? QStringList{QStringLiteral("gpt-5.6-luna"), QStringLiteral("gpt-4o"), QStringLiteral("gpt-4o-mini")}
                           : QStringList{QStringLiteral("claude-opus-5-5"), QStringLiteral("claude-sonnet-5-5"), QStringLiteral("claude-haiku-5-5")};
    const QString model = configured.isEmpty() ? current : configured;
    m_agentModel = model.isEmpty() ? m_agentModels.constFirst() : model;
    if (m_chat) m_chat->setModels(m_agentModels, m_agentModel, true);
}

void MainWindow::loadAgentSettings() {
    QSettings settings;
    const QString envAnthropicKey = qEnvironmentVariable("ANTHROPIC_API_KEY");
    const QString envOpenaiKey = qEnvironmentVariable("OPENAI_API_KEY");
    // Keys come from the system keychain, moved there from plain settings.
    m_anthropicKey->setText(secrets::loadKey(settings, QStringLiteral("agent/anthropic/apiKey"), envAnthropicKey));
    m_anthropicBaseUrl->setText(settings.value(QStringLiteral("agent/anthropic/baseUrl"), QStringLiteral("https://api.anthropic.com/v1")).toString());
    m_anthropicModel->setText(settings.value(QStringLiteral("agent/anthropic/model"), QStringLiteral("claude-opus-5-5")).toString());
    m_openaiKey->setText(secrets::loadKey(settings, QStringLiteral("agent/openai/apiKey"), envOpenaiKey));
    m_openaiBaseUrl->setText(settings.value(QStringLiteral("agent/openai/baseUrl"), QStringLiteral("https://api.openai.com/v1")).toString());
    m_openaiModel->setText(settings.value(QStringLiteral("agent/openai/model"), QStringLiteral("gpt-5.6-luna")).toString());
    const QString provider = settings.value(QStringLiteral("agent/activeProvider"),
                                             qEnvironmentVariable("KERF_PROVIDER", envOpenaiKey.isEmpty() ? QStringLiteral("anthropic") : QStringLiteral("openai")))
                                 .toString()
                                 .toLower();
    if (provider == QStringLiteral("openai"))
        m_openaiActiveRadio->setChecked(true);
    else
        m_anthropicActiveRadio->setChecked(true);
    // Kerf Cloud unless a key of one's own is already set up, so existing
    // setups keep working; an explicit choice wins.
    const bool hasOwnKey = !m_anthropicKey->text().trimmed().isEmpty() || !m_openaiKey->text().trimmed().isEmpty();
    const QString connection = settings.value(QStringLiteral("agent/connection"), hasOwnKey ? QStringLiteral("own") : QStringLiteral("cloud")).toString();
    (connection == QStringLiteral("claude-code") ? m_claudeCodeRadio
     : connection == QStringLiteral("own")       ? m_ownKeyRadio
                                                 : m_cloudRadio)->setChecked(true);
    updateCloudStatus();
    const QString mode = settings.value(QStringLiteral("agent/openai/apiMode"), qEnvironmentVariable("OPENAI_API_MODE", QStringLiteral("chat"))).toString().toLower();
    const int modeIndex = m_openaiMode->findData(mode == QStringLiteral("responses") ? QStringLiteral("responses") : QStringLiteral("chat"));
    m_openaiMode->setCurrentIndex(std::max(0, modeIndex));
    updateAgentModelChoices();
}

void MainWindow::saveAgentSettings() {
    if (!m_anthropicKey || !m_openaiKey) return;
    QSettings settings;
    secrets::saveKey(settings, QStringLiteral("agent/anthropic/apiKey"), m_anthropicKey->text());
    settings.setValue(QStringLiteral("agent/anthropic/baseUrl"), m_anthropicBaseUrl->text().trimmed());
    settings.setValue(QStringLiteral("agent/anthropic/model"), m_anthropicModel->text().trimmed());
    secrets::saveKey(settings, QStringLiteral("agent/openai/apiKey"), m_openaiKey->text());
    settings.setValue(QStringLiteral("agent/openai/baseUrl"), m_openaiBaseUrl->text().trimmed());
    settings.setValue(QStringLiteral("agent/openai/model"), m_openaiModel->text().trimmed());
    settings.setValue(QStringLiteral("agent/openai/apiMode"), m_openaiMode->currentData().toString());
    settings.setValue(QStringLiteral("agent/activeProvider"), activeProviderId());
    settings.sync();
    updateAgentModelChoices();
    showStatus(QStringLiteral("Agent settings saved"), 3000);
}

bool MainWindow::eventFilter(QObject *object, QEvent *event) {
    if (object == m_preferences && event->type() == QEvent::Show) fitPreferences();
    if (object == m_errorBanner && (event->type() == QEvent::ShowToParent || event->type() == QEvent::HideToParent)) layoutProjectTitle();
    // The title stays centred as the toolbar, or a group beside it, changes.
    if ((object == m_toolbar || object == m_formatBank) &&
        (event->type() == QEvent::Resize || event->type() == QEvent::Move))
        layoutProjectTitle();
#if KERF_EDITOR_OWN_TITLE_BAR
    // Dragging the toolbar's empty stretches (or its title, which takes no
    // clicks) moves the window, and a double click zooms it, as a title
    // bar's do. The press is kept so the moves after it come here.
    if (object == m_toolbar && windowHandle()) {
        if (event->type() == QEvent::MouseButtonPress) return static_cast<QMouseEvent *>(event)->button() == Qt::LeftButton;
        if (event->type() == QEvent::MouseMove && (static_cast<QMouseEvent *>(event)->buttons() & Qt::LeftButton)) {
            windowHandle()->startSystemMove();
            return true;
        }
        if (event->type() == QEvent::MouseButtonDblClick) {
            titleBarDoubleClicked(this);
            return true;
        }
    }
#endif
    if (object == m_chatInput && event->type() == QEvent::KeyPress) {
        auto *key = static_cast<QKeyEvent *>(event);
        if ((key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) &&
            !(key->modifiers() & Qt::ShiftModifier)) {
            if (m_sendButton->isEnabled()) sendChat();
            return true;
        }
    }
    if (m_timelineScroll && object == m_timelineScroll->viewport() && event->type() == QEvent::Resize)
        m_timeline->followViewportWidth(m_timelineScroll->viewport()->width());
    return QMainWindow::eventFilter(object, event);
}

void MainWindow::buildUi() {
    // The three regions run edge to edge under the toolbar; the sheets, the
    // trays and the bars lie over them (StageShell).
    m_shell = new StageShell(this);
    m_artwork = new MediaArtwork(this);

    m_connectionBanner = new QFrame;
    m_connectionBanner->setObjectName(QStringLiteral("connectionBanner"));
    auto *connectionLayout = new QHBoxLayout(m_connectionBanner);
    connectionLayout->setContentsMargins(12, 5, kTrailingInset, 5);
    m_connectionMessage = new QLabel;
    m_connectionMessage->setWordWrap(true);
    connectionLayout->addWidget(m_connectionMessage, 1);
    m_reconnectButton = new QPushButton(QStringLiteral("Reconnect"));
    connect(m_reconnectButton, &QPushButton::clicked, this, [this] {
        setConnection(QStringLiteral("connecting"));
        if (!m_bridge) {
            m_bridge = kerf_bridge_new(m_projectDirectory.toUtf8().constData(), m_kerfdBinary.toUtf8().constData());
            if (!m_bridge) setConnection(QStringLiteral("disconnected"), QStringLiteral("Could not start the runtime connection."));
        } else if (!sendSimple(QJsonObject{{QStringLiteral("type"), QStringLiteral("reconnect")}})) {
            setConnection(QStringLiteral("disconnected"), QStringLiteral("Reconnect could not be queued. Try again."));
        }
    });
    connectionLayout->addWidget(m_reconnectButton);

    m_errorBanner = new QFrame;
    m_errorBanner->setObjectName(QStringLiteral("errorBanner"));
    auto *errorLayout = new QHBoxLayout(m_errorBanner);
    errorLayout->setContentsMargins(12, 5, kTrailingInset, 5);
    m_errorMessage = new QLabel;
    m_errorMessage->setWordWrap(true);
    errorLayout->addWidget(m_errorMessage, 1);
    auto *dismissError = new QPushButton(QStringLiteral("Dismiss"));
    connect(dismissError, &QPushButton::clicked, m_errorBanner, &QWidget::hide);
    errorLayout->addWidget(dismissError);
    m_errorBanner->hide();

    // A git pull in a terminal or a text editor changed project.kerf; kerfd
    // keeps the editor's version until one of them is chosen.
    m_divergedBanner = new QFrame;
    m_divergedBanner->setObjectName(QStringLiteral("divergedBanner"));
    auto *divergedLayout = new QHBoxLayout(m_divergedBanner);
    divergedLayout->setContentsMargins(12, 5, kTrailingInset, 5);
    divergedLayout->setSpacing(8);
    // The colour is in the triangle only; the bar and its text stay neutral.
    m_divergedGlyph = new QLabel;
    m_divergedGlyph->setObjectName(QStringLiteral("divergedGlyph"));
    m_divergedGlyph->setPixmap(warningTriangle(editorTheme().warningGlyph));
    m_divergedGlyph->setFixedSize(14, 14);
    divergedLayout->addWidget(m_divergedGlyph);
    auto *divergedTitle = new QLabel(QStringLiteral("project.kerf changed on disk."));
    divergedTitle->setObjectName(QStringLiteral("divergedTitle"));
    divergedLayout->addWidget(divergedTitle);
    auto *divergedText = new QLabel(QStringLiteral("Load it to bring in edits made outside Kerf (a git pull, a text editor), "
                                                   "or keep the editor's version and write it over the file."));
    divergedText->setObjectName(QStringLiteral("divergedMessage"));
    divergedText->setWordWrap(true);
    divergedLayout->addWidget(divergedText, 1);
    // The default action trails, as macOS orders a row of buttons.
    m_keepEditorVersion = new QPushButton(QStringLiteral("Keep Editor Version"));
    m_keepEditorVersion->setObjectName(QStringLiteral("keepEditorVersion"));
    connect(m_keepEditorVersion, &QPushButton::clicked, this, [this] { execute(QStringLiteral("projection.repair")); });
    divergedLayout->addWidget(m_keepEditorVersion);
    m_loadFromDisk = new QPushButton(QStringLiteral("Load from Disk"));
    m_loadFromDisk->setObjectName(QStringLiteral("loadFromDisk"));
    connect(m_loadFromDisk, &QPushButton::clicked, this, [this] { execute(QStringLiteral("projection.import")); });
    divergedLayout->addWidget(m_loadFromDisk);
    m_divergedBanner->hide();

    // The regions, each in its own file; the order is the order they need
    // each other in.
    buildReview();
    buildStage();
    buildTimeline();
    buildSource();
    buildInspector();
    buildBin();
    buildPreferences();
    buildShell();
    setCentralWidget(m_shell);
    wireTimeline();
    updateProposal(QJsonValue());
    updateInspector();
}

void MainWindow::buildPreferences() {
    // Agent settings live in the Settings dialog, out of the editing surface.
    m_preferences = new QDialog(this);
    m_preferences->setObjectName(QStringLiteral("preferencesDialog"));
    m_preferences->setWindowTitle(QStringLiteral("Settings"));
    // A macOS Settings window: sized to what it shows, closed with its
    // close button or Command-W, with no Close button of its own.
    auto *closeShortcut = new QShortcut(QKeySequence::Close, m_preferences);
    connect(closeShortcut, &QShortcut::activated, m_preferences, &QDialog::hide);
    auto *settingsTabLayout = new QVBoxLayout(m_preferences);
    settingsTabLayout->setContentsMargins(20, 12, 20, 20);
    auto *settingsScroll = new QScrollArea;
    settingsScroll->setWidgetResizable(true);
    settingsScroll->setFrameShape(QFrame::NoFrame);
    auto *settingsContent = new QWidget;
    auto *settingsLayout = new QVBoxLayout(settingsContent);
    settingsLayout->setContentsMargins(0, 0, 2, 0);
    settingsLayout->setSpacing(10);
    // Kerf Cloud, the hosted model, comes first. Keys of one's own and MCP
    // live under Advanced.
    auto *connectionGroup = new QGroupBox(QStringLiteral("AI"));
    connectionGroup->setObjectName(QStringLiteral("aiConnection"));
    auto *cloudLayout = new QVBoxLayout(connectionGroup);
    cloudLayout->setContentsMargins(10, 10, 10, 10);
    cloudLayout->setSpacing(6);
    m_cloudRadio = new QRadioButton(QStringLiteral("Kerf Cloud"));
    m_cloudRadio->setObjectName(QStringLiteral("agentConnectionCloud"));
    m_ownKeyRadio = new QRadioButton(QStringLiteral("My own API key (set up under Advanced)"));
    m_ownKeyRadio->setObjectName(QStringLiteral("agentConnectionOwnKey"));
    m_claudeCodeRadio = new QRadioButton(QStringLiteral("Claude Code (signed in to your Claude account)"));
    m_claudeCodeRadio->setObjectName(QStringLiteral("agentConnectionClaudeCode"));
    m_claudeCodeRadio->setToolTip(QStringLiteral("Runs your installed Claude Code with only this project's Kerf tools. It uses your Claude plan; Kerf never sees its sign-in. Sign in once by running claude in a terminal."));
    auto *connectionChoice = new QButtonGroup(connectionGroup);
    connectionChoice->addButton(m_cloudRadio);
    connectionChoice->addButton(m_ownKeyRadio);
    connectionChoice->addButton(m_claudeCodeRadio);
    cloudLayout->addWidget(m_cloudRadio);
    cloudLayout->addWidget(m_ownKeyRadio);
    cloudLayout->addWidget(m_claudeCodeRadio);
    m_cloudStatus = new QLabel;
    m_cloudStatus->setObjectName(QStringLiteral("cloudStatus"));
    m_cloudStatus->setWordWrap(true);
    cloudLayout->addWidget(m_cloudStatus);
    m_cloudToken = new QLineEdit;
    m_cloudToken->setObjectName(QStringLiteral("cloudToken"));
    m_cloudToken->setEchoMode(QLineEdit::Password);
    m_cloudToken->setPlaceholderText(QStringLiteral("Kerf Cloud sign-in token"));
    cloudLayout->addWidget(m_cloudToken);
    auto *cloudButtons = new QHBoxLayout;
    m_cloudSignIn = new QPushButton(QStringLiteral("Sign in"));
    m_cloudSignIn->setObjectName(QStringLiteral("primaryButton"));
    connect(m_cloudSignIn, &QPushButton::clicked, this, [this] {
        const QString token = m_cloudToken->text().trimmed();
        if (token.isEmpty()) return;
        if (sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_in")}, {QStringLiteral("token"), token}}))
            m_cloudToken->clear();
    });
    m_cloudSignOut = new QPushButton(QStringLiteral("Sign out"));
    connect(m_cloudSignOut, &QPushButton::clicked, this, [this] {
        sendCloud(QJsonObject{{QStringLiteral("type"), QStringLiteral("cloud_sign_out")}});
    });
    auto *plans = new QPushButton(QStringLiteral("Plans"));
    plans->setObjectName(QStringLiteral("cloudPlans"));
    plans->setEnabled(false);
    plans->setToolTip(QStringLiteral("Plans are coming soon"));
    cloudButtons->addWidget(m_cloudSignIn);
    cloudButtons->addWidget(m_cloudSignOut);
    cloudButtons->addStretch(1);
    cloudButtons->addWidget(plans);
    cloudLayout->addLayout(cloudButtons);
    settingsLayout->addWidget(connectionGroup);
    const auto onConnectionChanged = [this](bool checked) {
        if (!checked) return;
        QSettings().setValue(QStringLiteral("agent/connection"), m_cloudRadio->isChecked()        ? QStringLiteral("cloud")
                                                                 : m_claudeCodeRadio->isChecked() ? QStringLiteral("claude-code")
                                                                                                  : QStringLiteral("own"));
        updateAgentModelChoices();
        updateCloudStatus();
    };
    connect(m_cloudRadio, &QRadioButton::toggled, this, onConnectionChanged);
    connect(m_ownKeyRadio, &QRadioButton::toggled, this, onConnectionChanged);
    connect(m_claudeCodeRadio, &QRadioButton::toggled, this, onConnectionChanged);

    // A borderless disclosure, as the launcher's Details row.
    auto *advancedToggle = new QToolButton;
    advancedToggle->setObjectName(QStringLiteral("disclosure"));
    advancedToggle->setAccessibleName(QStringLiteral("Advanced"));
    advancedToggle->setText(QStringLiteral("Advanced"));
    advancedToggle->setAutoRaise(true);
    advancedToggle->setCheckable(true);
    advancedToggle->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
    advancedToggle->setIconSize(QSize(11, 11));
    advancedToggle->setIcon(disclosureIcon(false, editorTheme().textSecondary));
    m_disclosures.append(advancedToggle);
    settingsLayout->addWidget(advancedToggle);
    m_advancedSettings = new QWidget;
    m_advancedSettings->setObjectName(QStringLiteral("advancedSettings"));
    auto *advancedLayout = new QVBoxLayout(m_advancedSettings);
    advancedLayout->setContentsMargins(0, 0, 0, 0);
    advancedLayout->setSpacing(10);
    settingsLayout->addWidget(m_advancedSettings);
    m_advancedSettings->hide();
    connect(advancedToggle, &QToolButton::toggled, this, [this, advancedToggle](bool show) {
        m_advancedSettings->setVisible(show);
        advancedToggle->setIcon(disclosureIcon(show, editorTheme().textSecondary));
        fitPreferences();
    });
    auto *settingsIntro = new QLabel(QStringLiteral("Two protocol slots. One active connection for chat. Gemini, Groq, and OpenRouter use the OpenAI-compatible slot with that vendor's compatible base URL."));
    settingsIntro->setObjectName(QStringLiteral("chatHint"));
    settingsIntro->setWordWrap(true);
    advancedLayout->addWidget(settingsIntro);
    auto *activeConnection = new QGroupBox(QStringLiteral("Active connection"));
    auto *activeLayout = new QVBoxLayout(activeConnection);
    activeLayout->setContentsMargins(10, 10, 10, 10);
    activeLayout->setSpacing(6);
    m_anthropicActiveRadio = new QRadioButton(QStringLiteral("Anthropic Messages"));
    m_anthropicActiveRadio->setObjectName(QStringLiteral("agentActiveAnthropic"));
    m_openaiActiveRadio = new QRadioButton(QStringLiteral("OpenAI-compatible"));
    m_openaiActiveRadio->setObjectName(QStringLiteral("agentActiveOpenai"));
    activeLayout->addWidget(m_anthropicActiveRadio);
    activeLayout->addWidget(m_openaiActiveRadio);
    advancedLayout->addWidget(activeConnection);

    auto *anthropicGroup = new QGroupBox(QStringLiteral("Anthropic Messages"));
    anthropicGroup->setObjectName(QStringLiteral("providerBox"));
    auto *anthropicForm = new QFormLayout(anthropicGroup);
    anthropicForm->setContentsMargins(10, 10, 10, 10);
    anthropicForm->setSpacing(7);
    m_anthropicKey = new QLineEdit;
    m_anthropicKey->setEchoMode(QLineEdit::Password);
    m_anthropicKey->setPlaceholderText(QStringLiteral("sk-ant-…"));
    m_anthropicKey->setClearButtonEnabled(true);
    m_anthropicBaseUrl = new QLineEdit(QStringLiteral("https://api.anthropic.com/v1"));
    m_anthropicBaseUrl->setPlaceholderText(QStringLiteral("https://…/v1"));
    m_anthropicModel = new QLineEdit(QStringLiteral("claude-opus-5-5"));
    anthropicForm->addRow(QStringLiteral("API key"), m_anthropicKey);
    anthropicForm->addRow(QStringLiteral("Base URL"), m_anthropicBaseUrl);
    anthropicForm->addRow(QStringLiteral("Model"), m_anthropicModel);
    advancedLayout->addWidget(anthropicGroup);

    auto *openaiGroup = new QGroupBox(QStringLiteral("OpenAI-compatible"));
    openaiGroup->setObjectName(QStringLiteral("providerBox"));
    auto *openaiForm = new QFormLayout(openaiGroup);
    openaiForm->setContentsMargins(10, 10, 10, 10);
    openaiForm->setSpacing(7);
    m_openaiKey = new QLineEdit;
    m_openaiKey->setEchoMode(QLineEdit::Password);
    m_openaiKey->setPlaceholderText(QStringLiteral("sk-…"));
    m_openaiKey->setClearButtonEnabled(true);
    m_openaiBaseUrl = new QLineEdit(QStringLiteral("https://api.openai.com/v1"));
    m_openaiBaseUrl->setPlaceholderText(QStringLiteral("https://…/v1"));
    auto *openaiBaseUrlHint = new QLabel(QStringLiteral("Must serve /chat/completions or /responses. Native Gemini generateContent URLs return 404. Use a root that ends in /openai."));
    openaiBaseUrlHint->setObjectName(QStringLiteral("muted"));
    openaiBaseUrlHint->setWordWrap(true);
    m_openaiModel = new QLineEdit(QStringLiteral("gpt-5.6-luna"));
    m_openaiMode = new QComboBox;
    m_openaiMode->addItem(QStringLiteral("Chat Completions"), QStringLiteral("chat"));
    m_openaiMode->addItem(QStringLiteral("Responses"), QStringLiteral("responses"));
    openaiForm->addRow(QStringLiteral("API key"), m_openaiKey);
    openaiForm->addRow(QStringLiteral("Base URL"), m_openaiBaseUrl);
    openaiForm->addRow(QString(), openaiBaseUrlHint);
    openaiForm->addRow(QStringLiteral("Model"), m_openaiModel);
    openaiForm->addRow(QStringLiteral("API shape"), m_openaiMode);
    advancedLayout->addWidget(openaiGroup);
    auto *settingsNote = new QLabel(QStringLiteral("Credentials stay outside project files and are never included in chat activity."));
    settingsNote->setObjectName(QStringLiteral("muted"));
    settingsNote->setWordWrap(true);
    advancedLayout->addWidget(settingsNote);
    auto *saveSettings = new QPushButton(QStringLiteral("Save agent connection"));
    saveSettings->setObjectName(QStringLiteral("primaryButton"));
    connect(saveSettings, &QPushButton::clicked, this, &MainWindow::saveAgentSettings);
    advancedLayout->addWidget(saveSettings);

    auto *inboundSection = new QGroupBox(QStringLiteral("Use MCP tools in Kerf chat"));
    inboundSection->setObjectName(QStringLiteral("inboundMcpSection"));
    auto *inboundLayout = new QVBoxLayout(inboundSection);
    m_mcpEnabled = new QCheckBox(QStringLiteral("Enable external MCP tools"));
    m_mcpEnabled->setObjectName(QStringLiteral("inboundMcpEnabled"));
    const bool environmentMcpEnabled = qEnvironmentVariable("KERF_ENABLE_MCP") == QStringLiteral("1");
    m_mcpEnabled->setChecked(QSettings().value(QStringLiteral("agent/mcpEnabled"), environmentMcpEnabled).toBool());
    inboundLayout->addWidget(m_mcpEnabled);
    auto *inboundIntro = new QLabel(QStringLiteral("Kerf chat calls external MCP servers directly; Cursor is not needed. Your AI provider above still powers the chat. Kerf starts local commands using stdio; URL-only remote servers are not supported. KERF_ENABLE_MCP=1 also enables tools even if this box is unchecked."));
    inboundIntro->setWordWrap(true);
    inboundIntro->setObjectName(QStringLiteral("chatHint"));
    inboundLayout->addWidget(inboundIntro);
    const QString inboundPath = QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".kerf/mcp.json"));
    auto *inboundPathLabel = new QLabel(QStringLiteral("Project config: %1\nAlso reads ~/.kerf/mcp.json; project entries override matching server names. Use command, optional args, env, and tools (an allowlist). Commands must be installed locally; env values are literal strings. This file may contain server credentials.").arg(inboundPath));
    inboundPathLabel->setWordWrap(true);
    inboundPathLabel->setObjectName(QStringLiteral("muted"));
    inboundLayout->addWidget(inboundPathLabel);
    m_inboundMcpJson = new QTextEdit;
    m_inboundMcpJson->setObjectName(QStringLiteral("inboundMcpConfigJson"));
    m_inboundMcpJson->setAcceptRichText(false);
    m_inboundMcpJson->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    m_inboundMcpJson->setMinimumHeight(160);
    m_inboundMcpJson->setPlainText(QStringLiteral("{\n  \"mcpServers\": {}\n}\n"));
    m_inboundMcpJson->setPlaceholderText(QStringLiteral("{\"mcpServers\": {\"example\": {\"command\": \"/path/to/server\", \"args\": [], \"env\": {}}}}"));
    inboundLayout->addWidget(m_inboundMcpJson);
    for (const auto &source : {qMakePair(QStringLiteral("Load project Cursor config"), QDir(m_projectDirectory).absoluteFilePath(QStringLiteral(".cursor/mcp.json"))),
                               qMakePair(QStringLiteral("Load home Cursor config"), QDir::home().filePath(QStringLiteral(".cursor/mcp.json")))}) {
        auto *load = new QPushButton(source.first);
        load->setObjectName(source.first.contains(QStringLiteral("project")) ? QStringLiteral("loadProjectCursorMcp") : QStringLiteral("loadHomeCursorMcp"));
        load->setToolTip(source.second);
        connect(load, &QPushButton::clicked, this, [this, path = source.second]() { loadInboundMcpConfig(path, true); });
        inboundLayout->addWidget(load);
    }
    auto *saveInbound = new QPushButton(QStringLiteral("Save and apply MCP config"));
    saveInbound->setObjectName(QStringLiteral("saveInboundMcpConfig"));
    connect(saveInbound, &QPushButton::clicked, this, &MainWindow::saveInboundMcpConfig);
    inboundLayout->addWidget(saveInbound);
    m_inboundMcpStatus = new QLabel(QStringLiteral("No project config yet. Add servers here or load a Cursor config, then Save and apply. Existing home Kerf config is used automatically."));
    m_inboundMcpStatus->setObjectName(QStringLiteral("inboundMcpStatus"));
    m_inboundMcpStatus->setWordWrap(true);
    inboundLayout->addWidget(m_inboundMcpStatus);
    if (QFileInfo::exists(inboundPath)) loadInboundMcpConfig(inboundPath, false);
    connect(m_mcpEnabled, &QCheckBox::toggled, this, [this](bool enabled) {
        QSettings settings;
        settings.setValue(QStringLiteral("agent/mcpEnabled"), enabled);
        settings.sync();
        m_inboundMcpStatus->setText(QStringLiteral("MCP preference saved. Applies on the next chat request. KERF_ENABLE_MCP=1 also enables tools."));
    });
    advancedLayout->addWidget(inboundSection);

    auto *mcpSection = new QGroupBox(QStringLiteral("Use Kerf from Cursor / Claude"));
    mcpSection->setObjectName(QStringLiteral("mcpSection"));
    auto *mcpLayout = new QVBoxLayout(mcpSection);
    auto *mcpIntro = new QLabel(QStringLiteral("This is the opposite direction: Cursor, Claude, or Codex calls Kerf through kerf-mcp to work on this project. It does not enable external tools in Kerf chat. Kerf provider API keys are not required. Copy this config into your external client's MCP settings."));
    mcpIntro->setWordWrap(true);
    mcpIntro->setObjectName(QStringLiteral("chatHint"));
    mcpLayout->addWidget(mcpIntro);
    const QString projectPath = QDir(m_projectDirectory).absolutePath();
    auto *mcpProject = new QLineEdit(projectPath);
    mcpProject->setReadOnly(true);
    mcpProject->setObjectName(QStringLiteral("mcpProjectPath"));
    auto *mcpProjectForm = new QFormLayout;
    mcpProjectForm->addRow(QStringLiteral("Project"), mcpProject);
    mcpLayout->addLayout(mcpProjectForm);
    const QJsonObject config{{QStringLiteral("mcpServers"), QJsonObject{
        {QStringLiteral("kerf"), QJsonObject{
            {QStringLiteral("command"), mcpExecutable(projectPath)},
            {QStringLiteral("args"), QJsonArray{QStringLiteral("--dir"), projectPath}}
        }}}}};
    auto *mcpJson = new QTextEdit;
    mcpJson->setObjectName(QStringLiteral("mcpConfigJson"));
    mcpJson->setReadOnly(true);
    mcpJson->setFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    mcpJson->setPlainText(QString::fromUtf8(QJsonDocument(config).toJson(QJsonDocument::Indented)));
    mcpLayout->addWidget(mcpJson);
    auto *copyMcp = new QPushButton(QStringLiteral("Copy MCP config"));
    copyMcp->setObjectName(QStringLiteral("copyMcpConfig"));
    auto *mcpCopyStatus = new QLabel;
    mcpCopyStatus->setObjectName(QStringLiteral("muted"));
    connect(copyMcp, &QPushButton::clicked, this, [mcpJson, mcpCopyStatus]() {
        QApplication::clipboard()->setText(mcpJson->toPlainText());
        mcpCopyStatus->setText(QStringLiteral("MCP config copied"));
    });
    mcpLayout->addWidget(copyMcp);
    mcpLayout->addWidget(mcpCopyStatus);
    advancedLayout->addWidget(mcpSection);
    settingsLayout->addStretch(1);
    settingsScroll->setWidget(settingsContent);
    m_preferencesContent = settingsContent;
    settingsTabLayout->addWidget(settingsScroll);
    m_preferences->installEventFilter(this);

    loadAgentSettings();
    const auto onActiveConnectionChanged = [this](bool checked) {
        if (!checked) return;
        updateAgentModelChoices();
        saveAgentSettings();
    };
    connect(m_anthropicActiveRadio, &QRadioButton::toggled, this, onActiveConnectionChanged);
    connect(m_openaiActiveRadio, &QRadioButton::toggled, this, onActiveConnectionChanged);
    connect(m_anthropicKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_anthropicBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_anthropicModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_openaiKey, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_openaiBaseUrl, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_openaiModel, &QLineEdit::editingFinished, this, &MainWindow::saveAgentSettings);
    connect(m_openaiMode, QOverload<int>::of(&QComboBox::currentIndexChanged), this, &MainWindow::saveAgentSettings);
}

void MainWindow::buildShell() {
    m_finishTray = new FinishTray(nullptr, m_bus);
    m_deliverTray = new DeliverTray(nullptr, m_bus);
    m_graphEditor = new GraphEditor(nullptr, m_bus);
    if (char *catalog = kerf_fx_catalog_all()) {
        m_graphEditor->setEffectCatalog(QJsonDocument::fromJson(QByteArray(catalog)).array());
        kerf_bridge_string_free(catalog);
    }
    connect(m_graphEditor, &GraphEditor::editRequested, this, &MainWindow::sendEdit);
    connect(m_graphEditor, &GraphEditor::seekRequested, this, &MainWindow::queueSeek);
    m_shell->setRegions(m_viewer, m_chat, m_timelinePanel);
    buildSidebar();
    buildAgentViews();
    m_shell->setSheets(m_projectPanel, m_browserPanel, m_inspectorColumn);
    m_shell->setTrays(m_finishTray, m_deliverTray, m_graphEditor);
    m_shell->setBanners({m_connectionBanner, m_errorBanner, m_divergedBanner});
    // Each sheet closes from the end of its strip.
    const auto closeButton = [this](StageShell::Sheet sheet) {
        auto *button = new ui::IconButton(QStringLiteral("g-close"));
        button->setObjectName(QStringLiteral("sheetClose.%1").arg(StageShell::sheetId(sheet)));
        button->setAccessibleName(QStringLiteral("Close"));
        button->setTip(QStringLiteral("Close"), QStringLiteral("Esc"));
        button->setFocusPolicy(Qt::NoFocus);
        connect(button, &QAbstractButton::clicked, m_shell, [this] { m_shell->setSheet(StageShell::Sheet::None); });
        return button;
    };
    m_projectPanel->addToStrip(closeButton(StageShell::Sheet::Media));
    m_browserPanel->addToStrip(closeButton(StageShell::Sheet::Browser));
    m_inspectorColumn->addToStrip(closeButton(StageShell::Sheet::Inspector));
    connect(m_shell, &StageShell::sheetChanged, this, [this](StageShell::Sheet sheet) {
        // Closing Effects without adding: a later add made there stays there.
        if (sheet != StageShell::Sheet::Browser) m_effectsFromInspector = false;
        syncPanelCommands();
        if (sheet == StageShell::Sheet::None) settleFocus();
    });
    connect(m_shell, &StageShell::trayChanged, this, &MainWindow::applyTray);
    // A page chosen in a sheet's own strip keeps its commands in step.
    connect(m_browserPanel, &BrowserPanel::pageChanged, this, &MainWindow::syncPanelCommands);
    // The error bar shown or dismissed, the toolbar's subtitle follows it.
    m_errorBanner->installEventFilter(this);
    // Esc in an empty composer, with nothing of its own to stop or cancel.
    connect(m_chat, &ChatPanel::escapeUnused, m_shell, [this] { m_shell->dismiss(); });
}

void MainWindow::setupActions() {
    // Menus, shortcuts, and the toolbar come from the command registry.
    // Frontend commands and engine commands that need a dialog first have
    // handlers here; every other command resolves to a bridge request.
    m_commands = new CommandRegistry(QSettings().value(QStringLiteral("commands/keymap")).toString(),
                                     {{QStringLiteral("timeline"), m_timeline}, {QStringLiteral("source"), m_sourceMonitor}}, this);
    m_commandHandlers = {
        {QStringLiteral("file.newProject"), [this] { newProject(); }},
        {QStringLiteral("file.openProject"), [this] { openProjectFolder(); }},
        {QStringLiteral("preset.save"), [this] { savePreset(); }},
        {QStringLiteral("collab.sync"), [this] { if (execute(QStringLiteral("collab.sync"))) m_syncRequested = true; }},
        {QStringLiteral("collab.commit"), [this] { commitChanges(); }},
        {QStringLiteral("collab.newBranch"), [this] { newBranch(); }},
        {QStringLiteral("collab.switchBranch"), [this] { m_branchMenu->popup(QCursor::pos()); }},
        {QStringLiteral("collab.pullRequest"), [this] { openPullRequest(); }},
        {QStringLiteral("collab.publish"), [this] { publishProject(); }},
        {QStringLiteral("file.import"), [this] { importMedia(); }},
        {QStringLiteral("file.export"), [this] { exportProject(); }},
        {QStringLiteral("file.exportPreset"), [this] { exportEditingPreset(); }},
        {QStringLiteral("file.applyPreset"), [this] { importEditingPreset(); }},
        {QStringLiteral("app.quit"), [this] { if (close()) QApplication::quit(); }},
        {QStringLiteral("clip.focusRegion"), [this] { focusRegion(); }},
        {QStringLiteral("clip.startPath"), [this] { startMotionPath(); }},
        {QStringLiteral("sequence.renderPreview"), [this] { renderPreview(); }},
        {QStringLiteral("playback.toggle"), [this] { togglePlayback(); }},
        {QStringLiteral("playback.goToStart"), [this] { seekToStart(); }},
        {QStringLiteral("playback.previousFrame"), [this] { stepBackward(); }},
        {QStringLiteral("playback.nextFrame"), [this] { stepForward(); }},
        {QStringLiteral("playback.goToEnd"), [this] { seekToEnd(); }},
        {QStringLiteral("view.fitTracks"), [this] { m_timeline->fitTracksToView(); }},
        {QStringLiteral("view.resetTrackHeights"), [this] { m_timeline->resetTrackHeights(); }},
        {QStringLiteral("app.preferences"), [this] {
            m_preferences->show();
            m_preferences->raise();
            m_preferences->activateWindow();
        }},
        // The conversation is always open: its key puts the keys in its composer.
        {QStringLiteral("window.assistant"), [this] {
            uncoverComposer();
            m_chat->focusComposer();
        }},
        {QStringLiteral("timeline.addTrack"), [this] { addTrack(); }},
        {QStringLiteral("ai.ask"), [this] { askAi(); }},
        {QStringLiteral("source.markIn"), [this] { m_sourceMonitor->markIn(); }},
        {QStringLiteral("source.markOut"), [this] { m_sourceMonitor->markOut(); }},
        {QStringLiteral("source.clearMarks"), [this] { m_sourceMonitor->clearMarks(); }},
        {QStringLiteral("source.previousFrame"), [this] { m_sourceMonitor->step(-1); }},
        {QStringLiteral("source.nextFrame"), [this] { m_sourceMonitor->step(1); }},
    };
    connect(m_commands, &CommandRegistry::triggered, this, &MainWindow::triggerCommand);
    // Tools are one exclusive choice; snapping is a switch.
    const QHash<QString, TimelineTool> tools{
        {QStringLiteral("tool.selection"), TimelineTool::Selection}, {QStringLiteral("tool.razor"), TimelineTool::Razor},
        {QStringLiteral("tool.ripple"), TimelineTool::Ripple}, {QStringLiteral("tool.roll"), TimelineTool::Roll},
        {QStringLiteral("tool.slip"), TimelineTool::Slip}, {QStringLiteral("tool.slide"), TimelineTool::Slide},
    };
    connect(m_commands, &CommandRegistry::toggled, this, [this, tools](const QString &id, bool checked) {
        if (id.startsWith(QStringLiteral("workspace."))) {
            // The base state, or a tray over it; choosing an open tray again
            // goes back, and choosing the base state again leaves it chosen.
            if (id == QStringLiteral("workspace.finish")) m_shell->setTray(checked ? StageShell::Tray::Finish : StageShell::Tray::None);
            else if (id == QStringLiteral("workspace.deliver")) m_shell->setTray(checked ? StageShell::Tray::Deliver : StageShell::Tray::None);
            else m_shell->setTray(StageShell::Tray::None);
            syncPanelCommands();
        } else if (id == QStringLiteral("view.graph")) {
            m_shell->setTray(checked ? StageShell::Tray::Graph : StageShell::Tray::None);
            syncPanelCommands();
        } else if (id == QStringLiteral("view.safeZones")) setSafeZones(checked);
        else if (id == QStringLiteral("inspector.video")) {
            m_shell->setSheet(StageShell::Sheet::Inspector);
            m_inspectorColumn->setPage(InspectorColumn::Page::Inspector);
            m_inspectorColumn->setKind(InspectorColumn::Kind::Video);
            syncPanelCommands();
        } else if (id == QStringLiteral("window.source")) {
            // The Inspector sheet's Source page.
            if (checked) {
                m_shell->setSheet(StageShell::Sheet::Inspector);
                m_inspectorColumn->setPage(InspectorColumn::Page::Source);
            } else {
                m_shell->setSheet(StageShell::Sheet::None);
            }
            syncPanelCommands();
        } else if (id == QStringLiteral("window.effects") || id == QStringLiteral("window.presets")) {
            // The browser, on that page; unchecked, the sheet closes.
            if (checked) {
                m_shell->setSheet(StageShell::Sheet::Browser);
                m_browserPanel->setPage(id == QStringLiteral("window.effects") ? BrowserPanel::Page::Effects : BrowserPanel::Page::Presets);
            } else {
                m_shell->setSheet(StageShell::Sheet::None);
            }
            syncPanelCommands();
        } else if (id == QStringLiteral("window.inspector")) {
            // Open on Source, the sheet turns to the Inspector; only on its
            // own page does the command close it.
            if (m_shell->sheet() == StageShell::Sheet::Inspector && m_inspectorColumn->page() == InspectorColumn::Page::Source) {
                m_inspectorColumn->setPage(InspectorColumn::Page::Inspector);
            } else {
                m_shell->toggleSheet(StageShell::Sheet::Inspector);
                if (m_shell->sheet() == StageShell::Sheet::Inspector) m_inspectorColumn->setPage(InspectorColumn::Page::Inspector);
            }
            syncPanelCommands();
        } else if (id == QStringLiteral("window.project")) {
            m_shell->toggleSheet(StageShell::Sheet::Media);
            syncPanelCommands();
        } else if (id == QStringLiteral("view.sidebar") || id == QStringLiteral("view.picture") || id == QStringLiteral("view.timeline")) {
            setPaneOpen(id, checked);
        } else if (id == QStringLiteral("view.followKerf")) {
            setFollowKerf(checked);
        }
        if (tools.contains(id) && checked) m_timeline->setTool(tools.value(id));
        if (id == QStringLiteral("sequence.snap")) m_timeline->setSnapping(checked);
        if (id == QStringLiteral("collab.live")) {
            // Remembered per project, and asked for again on each connection.
            rememberLiveSync(checked);
            execute(id, QJsonObject{{QStringLiteral("enabled"), checked}});
        }
    });
    {
        QAction *live = m_commands->action(QStringLiteral("collab.live"));
        const QSignalBlocker blocker(live);
        live->setChecked(liveSyncRemembered());
    }
    m_branchMenu = new QMenu(QStringLiteral("Switch Branch"), this);
    m_branchMenu->setObjectName(QStringLiteral("switchBranchMenu"));
    connect(m_branchMenu, &QMenu::aboutToShow, this, [this] { buildBranchMenu(m_branchMenu); });
    m_commands->action(QStringLiteral("collab.switchBranch"))->setMenu(m_branchMenu);
    m_presets->setActions(m_commands->action(QStringLiteral("preset.save")), m_commands->action(QStringLiteral("preset.library")));
    auto *toolGroup = new QActionGroup(this);
    toolGroup->setExclusive(true);
    for (const QString &id : {QStringLiteral("tool.selection"), QStringLiteral("tool.razor"), QStringLiteral("tool.ripple"),
                              QStringLiteral("tool.roll"), QStringLiteral("tool.slip"), QStringLiteral("tool.slide")}) {
        QAction *action = m_commands->action(id);
        toolGroup->addAction(action);
        // Tool symbols, the way edit suites label their palettes.
        action->setIcon(toolIcon(id));
        m_toolGlyphs.append(action);
    }
    m_commands->action(QStringLiteral("tool.selection"))->setChecked(true);
    m_commands->action(QStringLiteral("sequence.snap"))->setChecked(m_timeline->snapping());
    // The Source monitor's marks and edits are drawn symbols, as the Program
    // monitor's transport; their tooltips name them with their shortcuts.
    for (const auto &[id, glyph] : {std::pair{QStringLiteral("source.markIn"), TransportGlyph::MarkIn},
                                    std::pair{QStringLiteral("source.markOut"), TransportGlyph::MarkOut},
                                    std::pair{QStringLiteral("source.insert"), TransportGlyph::Insert},
                                    std::pair{QStringLiteral("source.overwrite"), TransportGlyph::Overwrite}}) {
        QAction *action = m_commands->action(id);
        action->setIcon(transportIcon(glyph));
        m_sourceGlyphs.append({action, glyph});
    }
    m_commands->action(QStringLiteral("proposal.accept"))->setIconText(QStringLiteral("Accept"));
    m_commands->action(QStringLiteral("proposal.reject"))->setIconText(QStringLiteral("Reject"));
    m_commands->action(QStringLiteral("ai.stop"))->setIconText(QStringLiteral("Stop"));
    m_sourceMonitor->setActions({m_commands->action(QStringLiteral("source.markIn")), m_commands->action(QStringLiteral("source.markOut")),
                                 m_commands->action(QStringLiteral("source.insert")), m_commands->action(QStringLiteral("source.overwrite"))});
    menuBar()->setNativeMenuBar(true);
    const QHash<QString, QMenu *> menus = m_commands->buildMenus(menuBar());
    if (QMenu *file = menus.value(QStringLiteral("File"))) {
        m_recentMenu = new QMenu(QStringLiteral("Open Recent"), file);
        m_recentMenu->setObjectName(QStringLiteral("openRecentMenu"));
        const QList<QAction *> actions = file->actions();
        const qsizetype open = actions.indexOf(m_commands->action(QStringLiteral("file.openProject")));
        file->insertMenu(actions.value(open + 1), m_recentMenu);
        connect(m_recentMenu, &QMenu::aboutToShow, this, [this] { buildRecentMenu(m_recentMenu); });
        buildRecentMenu(m_recentMenu);
    }
    if (QMenu *edit = menus.value(QStringLiteral("Edit"))) {
        edit->addSeparator();
        auto *keymapMenu = edit->addMenu(QStringLiteral("Keyboard Shortcuts"));
        auto *keymapGroup = new QActionGroup(keymapMenu);
        for (const QJsonValue &value : m_commands->keymaps()) {
            const QJsonObject keymap = value.toObject();
            const QString id = keymap.value(QStringLiteral("id")).toString();
            auto *choice = keymapMenu->addAction(keymap.value(QStringLiteral("label")).toString());
            choice->setObjectName(QStringLiteral("keymap.%1").arg(id));
            choice->setCheckable(true);
            choice->setChecked(id == m_commands->keymap());
            keymapGroup->addAction(choice);
            connect(choice, &QAction::triggered, this, [this, id] {
                m_commands->setKeymap(id);
                QSettings().setValue(QStringLiteral("commands/keymap"), id);
            });
        }
    }
    setupToolbar(menus);
    setupPictureCommands();
    m_finishTray->setActions(m_commands->action(QStringLiteral("ai.ask")), m_commands->action(QStringLiteral("qc.run")));
    m_deliverTray->setRenderAction(m_commands->action(QStringLiteral("export.master")));
    // Deliver keys its outputs and paperwork by the project (not through the
    // window's file path).
    m_deliverTray->setProjectDirectory(m_projectDirectory);
    if (DeliverPane *pane = m_inspectorColumn ? m_inspectorColumn->deliverPane() : nullptr) pane->setProjectDirectory(m_projectDirectory);
    // The base state, for every region.
    applyTray();
    // The keys start on the timeline, so activating the window gives them to no button.
    m_timeline->setFocus(Qt::OtherFocusReason);
}

void MainWindow::closeEvent(QCloseEvent *event) { QMainWindow::closeEvent(event); }

void MainWindow::keyPressEvent(QKeyEvent *event) {
    // An Esc no widget under the focus took: the sheet closes, else the tray.
    if (event->key() == Qt::Key_Escape && event->modifiers() == Qt::NoModifier && m_shell && m_shell->dismiss()) {
        event->accept();
        return;
    }
    QMainWindow::keyPressEvent(event);
}

void MainWindow::openTray(const QString &name) {
    m_shell->setTray(name.compare(QLatin1String("Finish"), Qt::CaseInsensitive) == 0 ? StageShell::Tray::Finish
                     : name.compare(QLatin1String("Deliver"), Qt::CaseInsensitive) == 0 ? StageShell::Tray::Deliver
                                                                                        : StageShell::Tray::None);
}

void MainWindow::showEvent(QShowEvent *event) {
    QMainWindow::showEvent(event);
    // Shown the first time, the keys start where they belong (see
    // settleFocus); later shows keep the reader's place.
    if (!m_focusSettled) QTimer::singleShot(0, this, &MainWindow::settleFocus);
#if KERF_EDITOR_OWN_TITLE_BAR
    hideNativeWindowTitle(this);
    if (QWindow *handle = windowHandle()) {
        disconnect(handle, &QWindow::safeAreaMarginsChanged, this, nullptr);
        connect(handle, &QWindow::safeAreaMarginsChanged, this, [this] { fitTitleBar(); });
    }
    fitTitleBar();
#endif
}

void MainWindow::changeEvent(QEvent *event) {
    QMainWindow::changeEvent(event);
    if (event->type() == QEvent::WindowStateChange) fitTitleBar();
    // Only the first activation: Qt then hands the keys to the first button
    // in the chain. Coming back to the window later keeps the button a
    // keyboard user tabbed to.
    if (event->type() == QEvent::ActivationChange && isActiveWindow() && !m_focusSettled) {
        m_focusSettled = true;
        settleFocus();
    }
}

void MainWindow::newProject() {
    ProjectLauncher launcher(m_kerfdBinary, this);
    if (launcher.exec() == QDialog::Accepted) openProject(launcher.chosenPath());
}

void MainWindow::openProjectFolder() {
    const QString start = m_projectDirectory.isEmpty() ? QDir::homePath() : QFileInfo(m_projectDirectory).absolutePath();
    const QString path = QFileDialog::getExistingDirectory(this, QStringLiteral("Open Project"), start);
    if (path.isEmpty()) return;
    if (!QFileInfo::exists(QDir(path).filePath(QStringLiteral("project.kerf"))) &&
        QMessageBox::question(this, QStringLiteral("Open Project"),
                              QStringLiteral("“%1” has no project.kerf. Start a new project in it?").arg(QFileInfo(path).fileName())) !=
            QMessageBox::Yes)
        return;
    openProject(path);
}

void MainWindow::openProject(const QString &path) {
    if (path.isEmpty()) return;
    const QString clean = QDir::cleanPath(QFileInfo(path).absoluteFilePath());
    if (!m_projectDirectory.isEmpty() && clean == QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath())) {
        raise();
        activateWindow();
        return;
    }
    emit projectOpenRequested(clean);
}

void MainWindow::buildRecentMenu(QMenu *menu) {
    menu->clear();
    const QString current = m_projectDirectory.isEmpty() ? QString() : QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath());
    const QStringList recent = ProjectLauncher::recentProjects();
    for (const QString &path : recent) {
        QAction *open = menu->addAction(QFileInfo(path).fileName());
        open->setToolTip(path);
        open->setCheckable(path == current);
        open->setChecked(path == current);
        open->setEnabled(QFileInfo(path).isDir());
        connect(open, &QAction::triggered, this, [this, path] { openProject(path); });
    }
    if (recent.isEmpty()) menu->addAction(QStringLiteral("No Recent Projects"))->setEnabled(false);
    menu->addSeparator();
    QAction *clear = menu->addAction(QStringLiteral("Clear Menu"));
    clear->setEnabled(!recent.isEmpty());
    connect(clear, &QAction::triggered, this, [] { ProjectLauncher::clearRecentProjects(); });
}

void MainWindow::startSession() {
    execute(QStringLiteral("collab.status"));
    execute(QStringLiteral("preset.library"));
    if (liveSyncRemembered()) execute(QStringLiteral("collab.live"), QJsonObject{{QStringLiteral("enabled"), true}});
}

bool MainWindow::liveSyncRemembered() const {
    return !m_projectDirectory.isEmpty() && QSettings()
                                                .value(QStringLiteral("collab/liveProjects"))
                                                .toStringList()
                                                .contains(QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath()));
}

void MainWindow::rememberLiveSync(bool enabled) {
    if (m_projectDirectory.isEmpty()) return;
    QSettings settings;
    QStringList projects = settings.value(QStringLiteral("collab/liveProjects")).toStringList();
    const QString path = QDir::cleanPath(QFileInfo(m_projectDirectory).absoluteFilePath());
    projects.removeAll(path);
    if (enabled) projects.append(path);
    settings.setValue(QStringLiteral("collab/liveProjects"), projects);
}

QJsonObject MainWindow::collabContext() const {
    // Typed field by field: the registry reads a context it cannot parse
    // as an offline one.
    const auto text = [this](const char *key) {
        const QJsonValue value = m_collabStatus.value(QLatin1String(key));
        return value.isString() && !value.toString().isEmpty() ? value : QJsonValue();
    };
    QJsonArray branches;
    for (const QJsonValue &branch : m_collabStatus.value(QStringLiteral("branches")).toArray())
        if (branch.isString()) branches.append(branch);
    const QJsonValue gh = m_collabStatus.value(QStringLiteral("gh"));
    return QJsonObject{
        {QStringLiteral("repository"), m_collabStatus.value(QStringLiteral("repository")).toBool()},
        {QStringLiteral("branch"), text("branch")},
        {QStringLiteral("upstream"), text("upstream")},
        {QStringLiteral("remote"), text("remote")},
        {QStringLiteral("ahead"), std::max(0, m_collabStatus.value(QStringLiteral("ahead")).toInt())},
        {QStringLiteral("behind"), std::max(0, m_collabStatus.value(QStringLiteral("behind")).toInt())},
        {QStringLiteral("changed"), m_collabStatus.value(QStringLiteral("changed")).toBool()},
        {QStringLiteral("conflicted"), m_collabStatus.value(QStringLiteral("conflicted")).toBool()},
        {QStringLiteral("branches"), branches},
        // A signed-in account may be reported by name.
        {QStringLiteral("gh"), gh.isBool() ? gh.toBool() : gh.isString() ? !gh.toString().isEmpty() : gh.isObject()},
        {QStringLiteral("live"), m_collabStatus.value(QStringLiteral("live")).toBool()},
        {QStringLiteral("busy"), m_collabStatus.value(QStringLiteral("busy")).toBool() || !m_syncStage.isEmpty()},
        {QStringLiteral("externalMedia"), m_mediaStatus.value(QStringLiteral("external")).toArray().size()},
    };
}

void MainWindow::handleCollabEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type == QStringLiteral("collab")) {
        m_collabStatus = event.value(QStringLiteral("status")).toObject();
        QAction *live = m_commands->action(QStringLiteral("collab.live"));
        const QSignalBlocker blocker(live);
        live->setChecked(m_collabStatus.value(QStringLiteral("live")).toBool());
    } else if (type == QStringLiteral("collab_sync")) {
        const QString stage = event.value(QStringLiteral("stage")).toString();
        if (stage == QStringLiteral("done")) {
            m_syncStage.clear();
            m_syncFailure.clear();
            const QJsonObject report = event.value(QStringLiteral("report")).toObject();
            const QJsonArray incoming = report.value(QStringLiteral("incoming")).toArray();
            const QJsonArray conflicts = report.value(QStringLiteral("conflicts")).toArray();
            QStringList lines;
            for (const QJsonValue &value : incoming) {
                const QJsonObject commit = value.toObject();
                lines << QStringLiteral("%1 · %2").arg(commit.value(QStringLiteral("author")).toString(),
                                                        commit.value(QStringLiteral("subject")).toString());
            }
            int keptTheirs = 0;
            for (const QJsonValue &value : conflicts) {
                const QJsonObject conflict = value.toObject();
                const bool theirs = conflict.value(QStringLiteral("kept")).toString() == QStringLiteral("theirs");
                if (theirs) ++keptTheirs;
                const QString path = conflict.value(QStringLiteral("path")).toString();
                const QString what = conflict.value(QStringLiteral("what")).toString();
                lines << QStringLiteral("%1 · %2").arg(theirs ? QStringLiteral("Kept theirs") : QStringLiteral("Kept yours"),
                                                        path.isEmpty() ? what : QStringLiteral("%1: %2").arg(path, what));
            }
            const int keptYours = conflicts.size() - keptTheirs;
            for (const QJsonValue &value : report.value(QStringLiteral("renamed")).toArray()) {
                const QJsonArray pair = value.toArray();
                lines << QStringLiteral("Renumbered · %1 → %2").arg(pair.at(0).toString(), pair.at(1).toString());
            }
            for (const QString &line : std::as_const(lines)) addActivity(QStringLiteral("sync"), line);
            // A quiet live-sync cycle keeps the last sync's news in the tooltip.
            if (!lines.isEmpty()) m_syncLines = lines;
            QStringList summary;
            if (!incoming.isEmpty())
                summary << (incoming.size() == 1 ? QStringLiteral("1 commit in") : QStringLiteral("%1 commits in").arg(incoming.size()));
            if (report.value(QStringLiteral("pushed")).toBool()) summary << QStringLiteral("pushed");
            if (keptYours > 0)
                summary << (keptYours == 1 ? QStringLiteral("1 change kept yours") : QStringLiteral("%1 changes kept yours").arg(keptYours));
            if (keptTheirs > 0)
                summary << (keptTheirs == 1 ? QStringLiteral("1 change kept theirs") : QStringLiteral("%1 changes kept theirs").arg(keptTheirs));
            if (!summary.isEmpty() || m_syncRequested)
                showStatus(summary.isEmpty() ? QStringLiteral("Synced: up to date")
                                             : QStringLiteral("Synced: ") + summary.join(QStringLiteral(", ")),
                           8000);
            // A pull or branch switch changes presets/, and a commit who
            // last changed each preset.
            execute(QStringLiteral("preset.library"));
            m_syncRequested = false;
        } else if (stage == QStringLiteral("failed")) {
            m_syncRequested = false;
            m_syncStage.clear();
            m_syncFailure = event.value(QStringLiteral("message")).toString(QStringLiteral("The sync did not finish."));
            showError(QStringLiteral("Sync failed: %1").arg(m_syncFailure));
            addActivity(QStringLiteral("sync"), m_syncFailure);
        } else {
            m_syncStage = stage.isEmpty() ? QStringLiteral("syncing") : stage;
            m_syncFailure.clear();
        }
    } else if (type == QStringLiteral("collab_pull_request")) {
        const QString url = event.value(QStringLiteral("url")).toString();
        const bool created = event.value(QStringLiteral("created")).toBool();
        // Without gh, the bridge gives GitHub's page for starting one.
        const bool compare = QUrl(url).path().section(QLatin1Char('/'), 3, 3) == QStringLiteral("compare");
        showStatus(created ? QStringLiteral("Opened a pull request: %1").arg(url)
                   : compare ? QStringLiteral("Opened GitHub's page for a new pull request: %1").arg(url)
                             : QStringLiteral("This branch already has a pull request: %1").arg(url),
                   8000);
        addActivity(QStringLiteral("sync"), url);
#ifndef KERF_QT_TESTING
        if (!url.isEmpty()) QDesktopServices::openUrl(QUrl(url));
#endif
    }
    layoutProjectTitle();
    updateCommandStates();
}

void MainWindow::buildBranchMenu(QMenu *menu) {
    menu->clear();
    const QString current = m_collabStatus.value(QStringLiteral("branch")).toString();
    for (const QJsonValue &value : m_collabStatus.value(QStringLiteral("branches")).toArray()) {
        const QString branch = value.toString();
        if (branch.isEmpty() || branch == current) continue;
        QAction *choice = menu->addAction(branch);
        choice->setObjectName(QStringLiteral("branch.%1").arg(branch));
        connect(choice, &QAction::triggered, this, [this, branch] {
            execute(QStringLiteral("collab.switchBranch"), QJsonObject{{QStringLiteral("name"), branch}});
        });
    }
    if (menu->isEmpty()) menu->addAction(QStringLiteral("No Other Branches"))->setEnabled(false);
}

void MainWindow::commitChanges() {
    bool ok = false;
    const QString message = QInputDialog::getText(this, QStringLiteral("Commit"), QStringLiteral("Describe the change"),
                                                  QLineEdit::Normal, QString(), &ok);
    if (ok && !message.trimmed().isEmpty())
        execute(QStringLiteral("collab.commit"), QJsonObject{{QStringLiteral("message"), message}});
}

void MainWindow::newBranch() {
    bool ok = false;
    const QString name = QInputDialog::getText(this, QStringLiteral("New Branch"),
                                               QStringLiteral("Commits your edits, then starts a branch from them.\nName"),
                                               QLineEdit::Normal, QString(), &ok);
    if (ok && !name.trimmed().isEmpty())
        execute(QStringLiteral("collab.newBranch"), QJsonObject{{QStringLiteral("name"), name}});
}

void MainWindow::openPullRequest() {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("pullRequestDialog"));
    dialog.setWindowTitle(QStringLiteral("Open Pull Request"));
    dialog.resize(480, 320);
    auto *layout = new QVBoxLayout(&dialog);
    const QString branch = m_collabStatus.value(QStringLiteral("branch")).toString();
    auto *intro = new QLabel(QStringLiteral("Pushes %1 and opens a pull request on GitHub. Reviewers read the change as Kerf code.")
                                 .arg(branch.isEmpty() ? QStringLiteral("this branch") : branch));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    auto *form = new QFormLayout;
    QString suggested = branch;
    suggested.replace(QLatin1Char('-'), QLatin1Char(' ')).replace(QLatin1Char('_'), QLatin1Char(' '));
    if (!suggested.isEmpty()) suggested[0] = suggested[0].toUpper();
    auto *title = new QLineEdit(suggested);
    title->setObjectName(QStringLiteral("pullRequestTitle"));
    form->addRow(QStringLiteral("Title"), title);
    auto *body = new QPlainTextEdit;
    body->setObjectName(QStringLiteral("pullRequestBody"));
    body->setPlaceholderText(QStringLiteral("Leave empty to list this branch's commits"));
    form->addRow(QStringLiteral("Description"), body);
    layout->addLayout(form, 1);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Open Pull Request"));
    buttons->button(QDialogButtonBox::Ok)->setDefault(true);
    connect(title, &QLineEdit::textChanged, buttons, [buttons](const QString &text) {
        buttons->button(QDialogButtonBox::Ok)->setEnabled(!text.trimmed().isEmpty());
    });
    buttons->button(QDialogButtonBox::Ok)->setEnabled(!suggested.trimmed().isEmpty());
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;
    QJsonObject params{{QStringLiteral("title"), title->text()}};
    if (!body->toPlainText().trimmed().isEmpty()) params.insert(QStringLiteral("body"), body->toPlainText());
    execute(QStringLiteral("collab.pullRequest"), params);
}

void MainWindow::publishProject() {
    QDialog dialog(this);
    dialog.setObjectName(QStringLiteral("publishDialog"));
    dialog.setWindowTitle(QStringLiteral("Publish to GitHub"));
    dialog.resize(500, 300);
    auto *layout = new QVBoxLayout(&dialog);
    auto *intro = new QLabel(QStringLiteral("Makes the project a Git repository if it is not one yet, then pushes it so "
                                            "collaborators can clone it, sync, and open pull requests."));
    intro->setWordWrap(true);
    layout->addWidget(intro);
    const QJsonObject collab = collabContext();
    auto *github = new QRadioButton(QStringLiteral("Create a GitHub repository"));
    github->setObjectName(QStringLiteral("publishGitHub"));
    github->setEnabled(collab.value(QStringLiteral("gh")).toBool());
    if (!github->isEnabled()) github->setToolTip(QStringLiteral("Install the GitHub CLI and sign in with `gh auth login`"));
    auto *name = new QLineEdit(QFileInfo(m_projectDirectory).fileName().replace(QRegularExpression(QStringLiteral("[^A-Za-z0-9._-]+")), QStringLiteral("-")));
    name->setObjectName(QStringLiteral("publishName"));
    auto *priv = new QCheckBox(QStringLiteral("Private"));
    priv->setObjectName(QStringLiteral("publishPrivate"));
    priv->setChecked(true);
    auto *githubRow = new QHBoxLayout;
    githubRow->setContentsMargins(22, 0, 0, 0);
    githubRow->addWidget(name, 1);
    githubRow->addWidget(priv);
    auto *remoteChoice = new QRadioButton(QStringLiteral("Push to a remote I already have"));
    remoteChoice->setObjectName(QStringLiteral("publishRemoteChoice"));
    auto *remote = new QLineEdit;
    remote->setObjectName(QStringLiteral("publishRemote"));
    remote->setPlaceholderText(QStringLiteral("git@github.com:you/film.git"));
    auto *remoteRow = new QHBoxLayout;
    remoteRow->setContentsMargins(22, 0, 0, 0);
    remoteRow->addWidget(remote);
    auto *local = new QRadioButton(QStringLiteral("Only make it a repository on this computer"));
    local->setObjectName(QStringLiteral("publishLocal"));
    local->setEnabled(!collab.value(QStringLiteral("repository")).toBool());
    (github->isEnabled() ? github : remoteChoice)->setChecked(true);
    layout->addWidget(github);
    layout->addLayout(githubRow);
    layout->addWidget(remoteChoice);
    layout->addLayout(remoteRow);
    layout->addWidget(local);
    if (!m_collabStatus.value(QStringLiteral("enclosing")).toString().isEmpty()) {
        auto *nested = new QLabel(QStringLiteral("This folder is inside another repository (%1); the project gets a repository of its own.")
                                      .arg(m_collabStatus.value(QStringLiteral("enclosing")).toString()));
        nested->setObjectName(QStringLiteral("muted"));
        nested->setWordWrap(true);
        layout->addWidget(nested);
    }
    layout->addStretch(1);
    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Publish"));
    buttons->button(QDialogButtonBox::Ok)->setDefault(true);
    const auto update = [=] {
        name->setEnabled(github->isChecked());
        priv->setEnabled(github->isChecked());
        remote->setEnabled(remoteChoice->isChecked());
        buttons->button(QDialogButtonBox::Ok)->setEnabled(!remoteChoice->isChecked() || !remote->text().trimmed().isEmpty());
    };
    for (QRadioButton *choice : {github, remoteChoice, local}) connect(choice, &QRadioButton::toggled, &dialog, update);
    connect(remote, &QLineEdit::textChanged, &dialog, update);
    update();
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addWidget(buttons);
    if (dialog.exec() != QDialog::Accepted) return;
    QJsonObject params;
    if (github->isChecked()) {
        QJsonObject repository{{QStringLiteral("private"), priv->isChecked()}};
        if (!name->text().trimmed().isEmpty()) repository.insert(QStringLiteral("name"), name->text().trimmed());
        params.insert(QStringLiteral("github"), repository);
    } else if (remoteChoice->isChecked()) {
        params.insert(QStringLiteral("remote"), remote->text().trimmed());
    }
    if (params.isEmpty()) execute(QStringLiteral("collab.publish"));
    else execute(QStringLiteral("collab.publish"), params);
}

void MainWindow::triggerCommand(const QString &requested) {
    const QString id = CommandRegistry::resolve(requested);
    if (!m_commands->isEnabled(id)) return;
    // A checkable command toggles its action, which carries it out.
    if (QAction *action = m_commands->action(id); action && action->isCheckable()) {
        action->toggle();
        return;
    }
    const auto handler = m_commandHandlers.constFind(id);
    if (handler != m_commandHandlers.cend()) (*handler)();
    else execute(id);
}

bool MainWindow::execute(const QString &id, const QJsonObject &params) {
    const QJsonObject result = CommandRegistry::run(id, params, commandContext());
    if (result.contains(QStringLiteral("error"))) {
        showStatus(result.value(QStringLiteral("error")).toString(), 4000);
        return false;
    }
    return sendSimple(result.value(QStringLiteral("request")).toObject());
}

QJsonObject MainWindow::commandContext() const {
    int videoTracks = 0;
    int audioTracks = 0;
    for (const QJsonValue &value : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QString type = value.toObject().value(QStringLiteral("type")).toString();
        if (type == QStringLiteral("video")) ++videoTracks;
        else if (type == QStringLiteral("audio")) ++audioTracks;
    }
    QJsonObject context{{QStringLiteral("connected"), m_connected},
                        {QStringLiteral("revision"), revisionValue()},
                        {QStringLiteral("canUndo"), m_snapshot.value(QStringLiteral("canUndo")).toBool()},
                        {QStringLiteral("canRedo"), m_snapshot.value(QStringLiteral("canRedo")).toBool()},
                        {QStringLiteral("playheadFrame"), frameAt(m_playhead)},
                        {QStringLiteral("previewRequired"), m_previewRequired},
                        {QStringLiteral("previewRenderable"), m_previewStatus == QStringLiteral("required") ||
                                                                  m_previewStatus == QStringLiteral("live")},
                        {QStringLiteral("videoTracks"), videoTracks},
                        {QStringLiteral("audioTracks"), audioTracks},
                        {QStringLiteral("sourceTargets"), QJsonArray::fromStringList(sourceTargets())},
                        {QStringLiteral("agentBusy"), m_agentBusy},
                        {QStringLiteral("aiAccess"), m_cloudRadio ? aiAccess() : QStringLiteral("ready")}};
    const QJsonValue projection = m_snapshot.value(QStringLiteral("projectionStatus"));
    if (projection.isString()) context.insert(QStringLiteral("projectionStatus"), projection);
    if (!m_collabStatus.isEmpty()) context.insert(QStringLiteral("collab"), collabContext());
    if (!m_proposalId.isEmpty()) {
        QJsonObject proposal{{QStringLiteral("id"), m_proposalId}};
        if (m_proposalBlocked) proposal.insert(QStringLiteral("blocked"), QStringLiteral("stale"));
        context.insert(QStringLiteral("proposal"), proposal);
    }
    if (m_sourceMonitor->isOpen()) {
        // The marked range: in frames of the media, length in frames of the sequence.
        const double in = m_sourceMonitor->inPoint();
        context.insert(QStringLiteral("source"),
                       QJsonObject{{QStringLiteral("media"), m_sourceMonitor->mediaId()},
                                   {QStringLiteral("inFrame"), std::llround(in * m_sourceMonitor->fps())},
                                   {QStringLiteral("durationFrames"), frameAt(m_sourceMonitor->outPoint() - in)}});
    }
    const QJsonObject clip = selectedClipObject();
    if (!clip.isEmpty()) {
        context.insert(QStringLiteral("selection"),
                       QJsonObject{{QStringLiteral("clip"), m_selectedClip},
                                   {QStringLiteral("track"), selectedTrackType()},
                                   {QStringLiteral("startFrame"), clipFrames(clip, QStringLiteral("startFrame"))},
                                   {QStringLiteral("durationFrames"), clipFrames(clip, QStringLiteral("durationFrames"))}});
    }
    return context;
}

void MainWindow::updateCommandStates() {
    if (!m_commands) return;
    m_commands->update(commandContext());
    // A click on the picture is Play.
    m_viewer->setPlayEnabled(m_commands->isEnabled(QStringLiteral("playback.toggle")));
    updateComposerHint();
}

bool MainWindow::sendSimple(const QJsonObject &value) {
    if (!m_connected && value.value(QStringLiteral("type")).toString() != QStringLiteral("reconnect") &&
        value.value(QStringLiteral("type")).toString() != QStringLiteral("abort")) {
        return false;
    }
    m_lastCommand = value;
    if (!m_bridge) {
        showError(QStringLiteral("Runtime is not connected"));
        return false;
    }
    const QByteArray payload = QJsonDocument(value).toJson(QJsonDocument::Compact);
    if (!kerf_bridge_send(m_bridge, payload.constData())) {
        showError(QStringLiteral("Runtime command queue is full; the edit was not sent"));
        return false;
    }
    return true;
}

bool MainWindow::sendSeek(double position, bool scrub) {
    if (!m_connected || !m_bridge) return false;
    const QJsonObject payload{{QStringLiteral("type"), QStringLiteral("seek")},
                              {QStringLiteral("position"), position},
                              {QStringLiteral("scrub"), scrub}};
    const QByteArray bytes = QJsonDocument(payload).toJson(QJsonDocument::Compact);
    return kerf_bridge_send(m_bridge, bytes.constData());
}

void MainWindow::flushSeek() {
    if (m_hasPendingSeek) {
        if (!sendSeek(m_pendingSeek, m_pendingSeekScrub)) return;
        m_hasPendingSeek = false;
        return;
    }
    if (m_hasPendingHoverSeek) {
        if (!sendSeek(m_pendingHoverSeek, m_pendingHoverScrub)) return;
        m_hasPendingHoverSeek = false;
    }
}

void MainWindow::handleEvent(QJsonObject event) {
    routeEvent(event);
    m_bus->dispatch(event);
}

void MainWindow::routeEvent(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (handleAgentEvent(event)) {
        // Kerf's turn, in the comments panel (main_window_review.cpp).
    } else if (type == QStringLiteral("connection")) {
        setConnection(event.value(QStringLiteral("status")).toString(), event.value(QStringLiteral("message")).toString());
    } else if (type == QStringLiteral("state")) {
        const QJsonObject snapshot = event.value(QStringLiteral("snapshot")).toObject().isEmpty()
                                          ? event.value(QStringLiteral("state")).toObject()
                                          : event.value(QStringLiteral("snapshot")).toObject();
        if (!snapshot.isEmpty()) applySnapshot(snapshot);
    } else if (type == QStringLiteral("viewer_reset")) {
        // What was showing stands in until the new playback's first frame, unless it would be
        // stretched, or there is nothing to play and no first frame will come.
        const QSize canvas = resolvedCanvasSize(m_project, m_project.value(QStringLiteral("sources")).toArray());
        const QSize shown = m_viewer->frameSize();
        const bool sameShape = !shown.isEmpty() && !canvas.isEmpty() && qAbs(double(shown.width()) / shown.height() - double(canvas.width()) / canvas.height()) < 0.01;
        if (!sameShape || m_snapshot.value(QStringLiteral("duration")).toDouble() <= 0.0) m_viewer->clearFrame();
        m_viewer->setCaption(QString());
        m_hasShownFrame = false;
    } else if (type == QStringLiteral("error")) {
        const QString message = event.value(QStringLiteral("message")).toString(QStringLiteral("Runtime error"));
        // How preset_save refuses a file that exists, after its path.
        static const QString presetExists = QStringLiteral(" already exists; choose another name or replace it");
        if (!m_savingPreset.isEmpty() && message.endsWith(presetExists)) {
            replaceSavedPreset(message.chopped(presetExists.size()));
        } else if (event.value(QStringLiteral("code")).toString() == QStringLiteral("BLOCKING_NOTES")) {
            // kerfd held a delivery back: its words, and the notes to resolve.
            m_toolbarWiring->holdForNotes(message);
        } else {
            showError(message);
            // A refused mask edit puts the outline back.
            updateViewerMasks();
        }
    } else if (type == QStringLiteral("preset_export")) {
        const QString path = event.value(QStringLiteral("path")).toString();
        const QByteArray document = event.value(QStringLiteral("document")).toString().toUtf8();
        QSaveFile file(path);
        if (!file.open(QIODevice::WriteOnly) || file.write(document) != document.size() || !file.commit()) {
            showError(QStringLiteral("Could not export preset: %1").arg(file.errorString()));
            return;
        }
        showStatus(QStringLiteral("Preset exported: %1").arg(QFileInfo(path).fileName()), 6000);
        addActivity(QStringLiteral("preset"), path);
    } else if (type == QStringLiteral("preset_inspect")) {
        showPresetBindings(event.value(QStringLiteral("path")).toString(), event.value(QStringLiteral("preset")).toObject());
    } else if (type == QStringLiteral("preset_applied")) {
        showStatus(QStringLiteral("Preset proposal ready for review"), 6000);
    } else if (type == QStringLiteral("preset_library")) {
        m_presets->setLibrary(event, m_projectDirectory);
    } else if (type == QStringLiteral("preset_saved")) {
        m_savingPreset.clear();
        const QString path = event.value(QStringLiteral("path")).toString();
        showStatus(QStringLiteral("Saved preset %1").arg(path), 6000);
        addActivity(QStringLiteral("preset"), path);
        execute(QStringLiteral("preset.library"));
    } else if (type == QStringLiteral("collab") || type == QStringLiteral("collab_sync") ||
               type == QStringLiteral("collab_pull_request")) {
        handleCollabEvent(event);
    } else if (type == QStringLiteral("media_status")) {
        m_mediaStatus = event;
        m_projectPanel->setMediaStatus(event);
        updateCommandStates();
    } else if (type == QStringLiteral("clock")) {
        if (event.value(QStringLiteral("duration")).isDouble()) m_duration = std::max(0.0, event.value(QStringLiteral("duration")).toDouble());
        if (!m_hoverSkimming && !m_pointerOwnsPlayhead) {
            m_playhead = std::clamp(event.value(QStringLiteral("position")).toDouble(), 0.0, m_duration);
            m_timeline->setPlayhead(m_playhead);
        }
        m_bus->setDuration(m_duration);
        m_bus->setPlayhead(m_playhead);
        updateCommandStates();
        // An animated clip's picture moves with the playhead, and its masks
        // with it; so do the Inspector's animated values.
        if (!m_selectedClip.isEmpty()) updateViewerMasks();
        m_inspector->setPlayhead(m_playhead);
    } else if (type == QStringLiteral("playback")) {
        m_playing = m_connected && event.value(QStringLiteral("playing")).toBool();
        m_bus->setPlaying(m_playing);
        if (m_playing) {
            m_hasPendingHoverSeek = false;
            m_hoverSkimming = false;
            m_timeline->clearSkimmer();
        }
        updateCommandStates();
    } else if (type == QStringLiteral("export")) {
        // A job running shows its progress in the toolbar (ToolbarWiring); how
        // it ended, for a moment.
        const QString status = event.value(QStringLiteral("status")).toString();
        const bool finished = status == QStringLiteral("ready") || status == QStringLiteral("complete") ||
                              status == QStringLiteral("completed") || status == QStringLiteral("failed") ||
                              status == QStringLiteral("cancelled") || status == QStringLiteral("canceled");
        if (finished) showStatus(QStringLiteral("Export %1").arg(status), 6000);
        // What was made goes home to its folder.
        if (status == QStringLiteral("complete") || status == QStringLiteral("completed") || status == QStringLiteral("ready")) flyExportHome();
        if (!event.value(QStringLiteral("path")).toString().isEmpty()) addActivity(QStringLiteral("export"), event.value(QStringLiteral("path")).toString());
    } else if (type == QStringLiteral("analysis")) {
        // Analyses effects draw from, made in the background.
        const QString message = event.value(QStringLiteral("message")).toString();
        showStatus(message, 6000);
        addActivity(QStringLiteral("analysis"), message);
    } else if (type == QStringLiteral("preview")) {
        const QString status = event.value(QStringLiteral("status")).toString();
        const QString message = event.value(QStringLiteral("message")).toString();
        if (!status.isEmpty()) m_previewStatus = status;
        if (status != QStringLiteral("running") && m_cutRendering) {
            m_cutRendering = false;
            // The render is the picture now, if it is of the latest.
            if (m_proposalId == m_previewedCut) m_motion->releaseStaying();
            quietenCutPreview();
            if (m_timeline->showsProposedCut() && m_proposalId != m_previewedCut) m_cutPreview.start();
        }
        if (status == QStringLiteral("required") || status == QStringLiteral("unavailable")) {
            m_previewRequired = true;
            m_viewer->clearFrame();
            const QString detail = message.isEmpty() ? QStringLiteral("Rendered preview required") : message;
            showStatus(detail, 6000);
            addActivity(QStringLiteral("preview"), message.isEmpty() ? QStringLiteral("Native preview unavailable") : message);
        } else if (status == QStringLiteral("running")) {
            m_previewRequired = true;
        } else if (status == QStringLiteral("ready")) {
            m_previewRequired = false;
            showStatus(QStringLiteral("Preview ready"), 4000);
        } else if (status == QStringLiteral("live")) {
            // The compositor draws the picture as it plays; a render still
            // gives the exact preview.
            m_previewRequired = false;
            showStatus(message.isEmpty() ? QStringLiteral("Live preview") : QStringLiteral("Live preview: ") + message, 4000);
        }
        // Playback reopened the program from its start: back to where it was.
        if (status == QStringLiteral("exited")) {
            m_returnAfterPreview = true;
        } else if ((status == QStringLiteral("ready") || status == QStringLiteral("live")) &&
                   event.value(QStringLiteral("proposalId")).toString().isEmpty() && m_returnAfterPreview) {
            m_returnAfterPreview = false;
            if (!m_changeReel->isActive()) queueSeek(m_playhead);
        }
        m_changeReel->followPreview(event);
    } else if (type == QStringLiteral("cloud")) {
        m_cloudAccess = event.value(QStringLiteral("access")).toString();
        m_cloudAccount = event.value(QStringLiteral("account")).toObject();
        m_cloudMessage = event.value(QStringLiteral("message")).toString();
        updateCloudStatus();
    } else if (type == QStringLiteral("caption")) {
        // (effect_preview answers reach their regions keyed, through the bus:
        // BinWiring for the browser, ReviewWiring for the colour chip.)
        m_viewer->setCaption(event.value(QStringLiteral("text")).toString());
    }
    if (type == QStringLiteral("preview")) updateCommandStates();
}

void MainWindow::applySnapshot(const QJsonObject &snapshot) {
    const auto diverged = [](const QJsonObject &state) {
        return state.value(QStringLiteral("projectionStatus")).toString() == QStringLiteral("diverged");
    };
    // project.kerf changing outside Kerf (a pull in a terminal), or being
    // loaded, can come with other presets.
    const bool presetsMayDiffer = m_connected && diverged(m_snapshot) != diverged(snapshot);
    m_snapshot = snapshot;
    m_revision = static_cast<quint64>(snapshot.value(QStringLiteral("revision")).toDouble());
    m_duration = std::max(0.0, snapshot.value(QStringLiteral("duration")).toDouble());
    // The timeline is a view of the model's first sequence; edits go back
    // as typed operations.
    m_project = snapshot.value(QStringLiteral("timeline")).toObject();
    // Derive presentation fields once per snapshot, not on every repaint.
    m_project = withClipNames(m_project, m_project.value(QStringLiteral("sources")).toArray());
    m_fps = m_project.value(QStringLiteral("fps")).toDouble(24.0);
    setProjectTitle(m_project.value(QStringLiteral("name")).toString(QStringLiteral("Untitled project")));
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    m_trackCount = tracks.size();
    if (!m_selectedClip.isEmpty() && selectedClipObject().isEmpty()) m_selectedClip.clear();
    m_selectedClips.erase(std::remove_if(m_selectedClips.begin(), m_selectedClips.end(),
                                         [this](const QString &id) { return clipObject(id).isEmpty(); }),
                          m_selectedClips.end());
    if (m_selectedClip.isEmpty()) m_selectedClips.clear();
    else if (!m_selectedClips.contains(m_selectedClip)) m_selectedClips = {m_selectedClip};
    m_timeline->setProject(m_project, m_duration, m_fps);
    m_timeline->setSelectedClips(m_selectedClips);
    const QJsonArray sources = m_project.value(QStringLiteral("sources")).toArray();
    const QSize canvasSize = resolvedCanvasSize(m_project, sources);
    // A format change lays out again at once. A first cut still being proposed has
    // its own format, which stays until the next look at it (showProposedCut).
    if (!m_timeline->showsProposedCut()) {
        m_viewer->setProjectInfo(canvasSize.width(), canvasSize.height());
        m_shell->setAspect(m_viewer->pictureAspect());
        m_timeline->setWideSequence(stage::isWide(m_viewer->pictureAspect()));
    }
    applyPanes();
    syncCanvasPreset(canvasSize);
    m_artwork->request(sources, m_projectDirectory);
    m_projectPanel->setSources(sources);
    if (m_sourceMonitor->isOpen()) {
        const bool present = std::any_of(sources.cbegin(), sources.cend(), [this](const QJsonValue &value) {
            return value.toObject().value(QStringLiteral("id")).toString() == m_sourceMonitor->mediaId();
        });
        if (!present) m_sourceMonitor->close();
    }
    updateProposal(snapshot.value(QStringLiteral("pendingProposal")));
    // The pinned proposal's (the version its card shows), not just the first open one.
    m_timeline->setProposal(window_parts::proposalTimelineFor(snapshot, m_proposalId));
    showProposedCut();
    refreshCompare();
    m_divergedBanner->setVisible(diverged(snapshot));
    updateInspector();
    updateCommandStates();
    m_bus->setDuration(m_duration);
    m_bus->setSelection(m_selectedClips);
    m_bus->setSnapshot(snapshot, m_project, m_fps, canvasSize);
    if (presetsMayDiffer) execute(QStringLiteral("preset.library"));
    applyPendingLink();
    followContentWithPanes();
    followLiveProposal();
    refreshCompare();
    runStart();
}

QStringList MainWindow::projectSlugs() const {
    QStringList slugs;
    for (const QString &name : {QFileInfo(m_projectDirectory).fileName(), m_projectName,
                                m_project.value(QStringLiteral("name")).toString()})
        if (const QString slug = links::slug(name); !slug.isEmpty() && !slugs.contains(slug)) slugs << slug;
    return slugs;
}

void MainWindow::followLink(qint64 revision, const QString &timecode) {
    m_linkPending = true;
    m_linkRevision = revision;
    m_linkTimecode = timecode;
    applyPendingLink();
}

void MainWindow::applyPendingLink() {
    // Only once the project is connected and its programme known (the seek
    // is clamped to the duration).
    if (!m_linkPending || !m_connected || m_snapshot.isEmpty()) return;
    m_linkPending = false;
    if (!m_linkTimecode.isEmpty()) {
        const double seconds = review::parseTimecode(m_linkTimecode, m_fps);
        if (std::isfinite(seconds)) queueSeek(seconds);
    }
    if (m_linkRevision >= 0 && quint64(m_linkRevision) != m_revision)
        addActivity(QStringLiteral("link"), QStringLiteral("The review link is for rev %1; the project is at rev %2").arg(m_linkRevision).arg(m_revision));
}

QString MainWindow::selectedTrackType() const {
    for (const QJsonValue &trackValue : m_project.value(QStringLiteral("tracks")).toArray()) {
        const QJsonObject track = trackValue.toObject();
        for (const QJsonValue &clipValue : track.value(QStringLiteral("clips")).toArray())
            if (clipValue.toObject().value(QStringLiteral("id")).toString() == m_selectedClip)
                return track.value(QStringLiteral("type")).toString();
    }
    return {};
}

QJsonObject MainWindow::selectedClipObject() const {
    const QJsonArray tracks = m_project.value(QStringLiteral("tracks")).toArray();
    for (const QJsonValue &trackValue : tracks) {
        for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (clip.value(QStringLiteral("id")).toString() == m_selectedClip) return clip;
        }
    }
    return {};
}

QSizeF MainWindow::selectedPictureSize(bool *fit) const { return pictureSizeOf(selectedClipObject(), selectedTrackType(), fit); }

QSizeF MainWindow::pictureSizeOf(const QJsonObject &clip, const QString &trackType, bool *fit) const {
    // A picture clip draws media, a nested sequence, or a generator.
    const QString kind = clip.value(QStringLiteral("kind")).toString();
    if (clip.isEmpty() || trackType != QStringLiteral("video") || kind == QStringLiteral("caption")) return {};
    *fit = kind == QStringLiteral("media") || kind == QStringLiteral("sequence");
    const QJsonArray sources = m_project.value(QStringLiteral("sources")).toArray();
    if (kind == QStringLiteral("media")) {
        for (const QJsonValue &value : sources) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() != clip.value(QStringLiteral("sourceId")).toString()) continue;
            const QSizeF size(source.value(QStringLiteral("width")).toDouble(), source.value(QStringLiteral("height")).toDouble());
            return size.isEmpty() ? QSizeF() : size;
        }
        return {};
    }
    // Generators fill the canvas; a nested sequence is fitted to it.
    return QSizeF(resolvedCanvasSize(m_project, sources));
}

void MainWindow::fitPreferences() {
    if (!m_preferences || !m_preferencesContent) return;
    m_preferences->ensurePolished();
    constexpr int width = 520;
    const QMargins margins = m_preferences->layout()->contentsMargins();
    QLayout *content = m_preferencesContent->layout();
    content->activate();
    const int inner = width - margins.left() - margins.right();
    const int height = content->hasHeightForWidth() ? content->totalHeightForWidth(inner) : content->totalSizeHint().height();
    const QScreen *screen = m_preferences->screen();
    const int limit = std::min(720, screen ? screen->availableGeometry().height() * 9 / 10 : 720);
    m_preferences->resize(width, std::min(limit, height + margins.top() + margins.bottom()));
}

void MainWindow::applyAppearance() {
    for (const auto &[action, glyph] : std::as_const(m_toolbarGlyphs)) action->setIcon(toolbarIcon(glyph));
    for (QAction *action : std::as_const(m_toolGlyphs)) action->setIcon(toolIcon(action->objectName()));
    if (m_divergedGlyph) m_divergedGlyph->setPixmap(warningTriangle(editorTheme().warningGlyph));
    for (QToolButton *disclosure : std::as_const(m_disclosures))
        disclosure->setIcon(disclosureIcon(disclosure->isChecked(), editorTheme().textSecondary));
    for (QAction *action : std::as_const(m_maskTools)) action->setIcon(maskToolIcon(static_cast<MaskTool>(action->data().toInt())));
    for (const auto &[action, glyph] : std::as_const(m_sourceGlyphs)) action->setIcon(transportIcon(glyph));
}

void MainWindow::addActivity(const QString &kind, const QString &text) {
    if (text.isEmpty()) return;
    m_activityLog << QStringLiteral("%1  %2").arg(kind.toUpper(), text);
    while (m_activityLog.size() > 80) m_activityLog.removeFirst();
    if (m_activityList) {
        m_activityList->addItem(m_activityLog.constLast());
        while (m_activityList->count() > 8) delete m_activityList->takeItem(0);
        m_activityList->scrollToBottom();
    }
}

void MainWindow::showError(const QString &message) {
    m_errorMessage->setText(message);
    m_errorBanner->show();
    // The toolbar's subtitle says so too, until it is dismissed.
    if (!m_errorBanner->isHidden()) layoutProjectTitle();
}

bool MainWindow::runCommand(const QString &id, const QJsonObject &params) {
    const QString command = CommandRegistry::resolve(id);
    QAction *action = m_commands ? m_commands->action(command) : nullptr;
    // A frontend command (a dialog, a switch, Ask) runs as its menu item does.
    if (params.isEmpty() && action && (m_commandHandlers.contains(command) || action->isCheckable())) {
        if (!m_commands->isEnabled(command)) return false;
        triggerCommand(command);
        return true;
    }
    return execute(command, params);
}

void MainWindow::applyTray() {
    // The regions hear it and apply theirs: the Inspector's wheels in Finish,
    // the Inspector column's page.
    const StageShell::Tray tray = m_shell->tray();
    m_bus->setWorkspace(tray == StageShell::Tray::Finish ? QStringLiteral("Finish")
                        : tray == StageShell::Tray::Deliver ? QStringLiteral("Deliver")
                                                            : QStringLiteral("Review"),
                        /*reapplied=*/true);
    const QWidget *open = m_shell->widgetOf(tray);
    m_chat->setCoveredBelow(open ? open->height() : 0);
    syncPanelCommands();
    settleFocus();
}

void MainWindow::settleFocus() {
    // A sheet or tray that held the keys may have closed, the timeline may
    // just have come back from being disabled, or none held them yet; Qt hands
    // the keys on as if Tab were pressed, to whichever control comes next. The
    // timeline's keys only work from the timeline, so they go there, unless
    // someone is typing or working in a sheet or tray. This window's own focus
    // child: a menu or dialog over it keeps its own. With no timeline open the
    // keys wait in the conversation.
    QWidget *holder = focusWidget();
    if (m_shell && !m_shell->panes().timeline) {
        if (!holder && m_chat) m_chat->focusComposer();
        return;
    }
    if (!m_timeline || !m_timeline->isEnabled()) return;
    if (holder) {
        if (holder == m_timeline || qobject_cast<QAbstractSpinBox *>(holder) || qobject_cast<QLineEdit *>(holder)) return;
        if (const auto *text = qobject_cast<QPlainTextEdit *>(holder); text && !text->isReadOnly()) return;
        if (const auto *text = qobject_cast<QTextEdit *>(holder); text && !text->isReadOnly()) return;
        if (m_shell && m_shell->holdsOpen(holder)) return;
    }
    m_timeline->setFocus(Qt::OtherFocusReason);
}

void MainWindow::uncoverComposer() {
    // At wide formats a sheet and a tray lie over the composer.
    const QWidget *composer = m_chat->composer();
    const QRect rect(composer->mapTo(m_shell, QPoint(0, 0)), composer->size());
    const auto covers = [&rect](const QWidget *over) { return over && over->isVisible() && over->geometry().intersects(rect); };
    if (covers(m_shell->widgetOf(m_shell->sheet()))) m_shell->setSheet(StageShell::Sheet::None);
    if (covers(m_shell->widgetOf(m_shell->tray()))) m_shell->setTray(StageShell::Tray::None);
}

void MainWindow::syncPanelCommands() {
    if (!m_commands || !m_shell) return;
    const StageShell::Sheet sheet = m_shell->sheet();
    const StageShell::Tray tray = m_shell->tray();
    const bool inspector = sheet == StageShell::Sheet::Inspector;
    const bool browser = sheet == StageShell::Sheet::Browser;
    const bool presets = browser && m_browserPanel->page() == BrowserPanel::Page::Presets;
    const bool source = inspector && m_inspectorColumn->page() == InspectorColumn::Page::Source;
    const QList<QPair<QString, bool>> states{
        {QStringLiteral("window.project"), sheet == StageShell::Sheet::Media},
        {QStringLiteral("window.inspector"), inspector && !source},
        {QStringLiteral("window.source"), source},
        {QStringLiteral("window.effects"), browser && !presets},
        {QStringLiteral("window.presets"), presets},
        {QStringLiteral("inspector.video"), m_inspectorColumn && m_inspectorColumn->kind() == InspectorColumn::Kind::Video},
        {QStringLiteral("inspector.audio"), m_inspectorColumn && m_inspectorColumn->kind() == InspectorColumn::Kind::Audio},
        {QStringLiteral("inspector.info"), m_inspectorColumn && m_inspectorColumn->kind() == InspectorColumn::Kind::Info},
        {QStringLiteral("workspace.review"), tray == StageShell::Tray::None || tray == StageShell::Tray::Graph},
        {QStringLiteral("workspace.finish"), tray == StageShell::Tray::Finish},
        {QStringLiteral("workspace.deliver"), tray == StageShell::Tray::Deliver},
        {QStringLiteral("view.sidebar"), m_shell->panes().sidebar},
        {QStringLiteral("view.picture"), m_shell->panes().picture},
        {QStringLiteral("view.timeline"), m_shell->panes().timeline},
        {QStringLiteral("view.graph"), tray == StageShell::Tray::Graph},
        {QStringLiteral("view.followKerf"), m_followKerf},
    };
    for (const auto &[id, checked] : states)
        if (QAction *action = m_commands->action(id)) {
            const QSignalBlocker blocker(action);
            action->setChecked(checked);
        }
}

} // namespace kerf
