#pragma once

#include "device_frame.h"
#include "editor_common.h"

#include <QAction>
#include <QCheckBox>
#include <QComboBox>
#include <QDialog>
#include <QDoubleSpinBox>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMainWindow>
#include <QPointer>
#include <QMenu>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QTextCursor>
#include <QTextEdit>
#include <QTimer>
#include <QToolBar>
#include <QToolButton>
#include <QVector>
#include <QWidget>

#include <cmath>
#include <functional>

namespace kerf {

class AskPrompt;
class ChatPanel;
struct ProposalChange;
class CommentRow;
class StageRail;
class CommandRegistry;
class ExportButton;
class FormatBank;
class InspectorPanel;
class EffectsPanel;
class PresetsPanel;
class ProjectPanel;
class SourceMonitorPanel;
class Workspace;
class MediaArtwork;
class TimelineWidget;
class TimelineZoomControl;
class ViewerWidget;

class MainWindow final : public QMainWindow {
    Q_OBJECT
    friend class EditorTest;
public:
    MainWindow(const QString &projectDirectory, const QString &kerfdBinary, QWidget *parent = nullptr);
    ~MainWindow() override;

signals:
    // File ▸ New Project, Open Project, and Open Recent: open `path` in a
    // window of its own and close this one.
    void projectOpenRequested(const QString &path);

private slots:
    void pollBridge();
    void importMedia();
    void exportProject();
    void exportEditingPreset();
    void importEditingPreset();
    void showPresetBindings(const QString &path, const QJsonObject &preset);
    void renderPreview();
    // A key of the toolbar's format bank was chosen.
    void canvasPresetChanged(const QSize &size);
    void addTrack();
    // Run a command from a menu, shortcut, or control: its handler when the
    // window carries it out, otherwise the request it resolves to.
    void triggerCommand(const QString &id);
    void togglePlayback();
    void queueSeek(double time);
    void queueHoverSeek(double time);
    void endHoverSeek();
    void settlePointerSeek();
    bool scrubFromShownFrame(double time) const;
    void selectClip(const QString &id);
    // A clip clicked on the timeline: selected, and the Inspector brought
    // forward in its column (see Workspace::raiseInspector).
    // ⌘-click or ⇧-click on a clip in the timeline.
    void extendClipSelection(const QString &id, bool toggle);
    void updateSelectionStatus();
    void clickSelectClip(const QString &id);
    // Open the conversation with `subject` ("@clip") in the composer.
    void askAboutInChat(const QString &subject);

    // Edits go to kerfd as typed operations on the model. Amounts and
    // positions count frames of the sequence.
    bool sendEdit(const QJsonArray &ops);
    qint64 frameAt(double seconds) const { return std::llround(seconds * m_fps); }
    QJsonObject clipObject(const QString &id) const;
    static qint64 clipFrames(const QJsonObject &clip, const QString &key);
    void moveClip(const QString &id, double start);
    void trimClip(const QString &id, bool leading, double frame);
    void splitClip();
    QJsonObject clipTrack(const QString &id) const;
    // Load bin media into the source monitor and bring it forward.
    void openSource(const QString &mediaId);
    QStringList sourceTargets() const;

    // A crop box {x, y, width, height} as the model's crop properties; an
    // empty box restores the full frame.
    static QJsonObject cropProps(const QJsonObject &box);

    // The inverse of cropProps: the clip's static crop as a box, empty when
    // uncropped.
    static QJsonObject cropBox(const QJsonObject &clip);
    QImage sourceFrame(const QJsonObject &clip, double timelineTime) const;
    double firstSceneCut(const QJsonObject &clip, double start, double end) const;

    QVector<QPair<qint64, QRectF>> trackRegion(const QJsonObject &clip, double start, double end,
                                                const QRectF &seed, double *reliableEnd) const;

    void focusRegion();
    void deleteSelected();
    void sendChat();
    // An agent request, anchored to `focus` when it is about a selection.
    bool sendAgent(const QString &text, const QJsonObject &focus, const QString &focusLabel);
    // Ask Kerf about the selected clip, or the sequence at the playhead.
    void askAi();
    void askSubmitted(const QString &text, const QString &scope);
    void updateAiBar();
    // Kerf's next steps as buttons in the AI bar and over the composer;
    // pressing one asks for it with the focus of the last request.
    void showNextSteps(const QJsonArray &steps);
    // Kerf Cloud, the hosted model, or a key of one's own (Advanced).
    bool usesCloud() const;
    bool sendCloud(const QJsonObject &command);
    // What the registry needs to know about the agent's connection.
    QString aiAccess() const;
    void updateCloudStatus();
    void updateChatControls();
    QString activeProviderId() const;
    QJsonObject agentSettingsPayload() const;
    void loadInboundMcpConfig(const QString &path, bool imported);
    void saveInboundMcpConfig();
    void updateAgentModelChoices();
    void loadAgentSettings();
    void saveAgentSettings();
    void acceptProposal();
    void rejectProposal();
    // Kerf's conversation, in the Program monitor's stage beside the
    // picture: open there (and the Program monitor with it, in front), or
    // closed. Remembered on this Mac, never in the project.
    bool chatOpen() const;
    void setChatOpen(bool open);
    // Bring the switches that open it in line with chatOpen().
    void syncChatSwitches();

protected:
    bool eventFilter(QObject *object, QEvent *event) override;
    void closeEvent(QCloseEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    void buildUi();
    // On macOS, where the toolbar is the title bar: fill the title bar's
    // height and start past the window buttons (none in full screen).
    void fitTitleBar();
    // The project's name as the window's title and the toolbar's.
    void setProjectTitle(const QString &name);
    // Centre the title on the toolbar, clear of the groups either side.
    void layoutProjectTitle();
    // "Edited · 9:16 · 24 fps": the line under the title.
    QString projectSubtitle() const;
    // Rebuild what caches the theme's colours: symbols drawn in code and the
    // conversation's messages.
    void applyAppearance();
    // Size Settings to what it shows, up to the screen's height; it scrolls past that.
    void fitPreferences();
    // Window ▸ Workspaces: the workspaces, and saving or resetting the current one.
    void buildWorkspaceMenu(QMenu *menu);
    void syncCanvasPreset(const QSize &size);
    void setupActions();
    // Resolve a command in the current context and send its request.
    bool execute(const QString &id, const QJsonObject &params = {});
    // What commands read: connection, selection, playhead, history, preview.
    QJsonObject commandContext() const;
    void updateCommandStates();

    // The transport, as the player's controls on the Program monitor's
    // picture (see PlayerOverlay): what they ask for runs here, and they
    // show the playhead's time, the length, and what can be pressed.
    void setupPlayerOverlay();
    void updateTransport();
    // The phone a portrait picture shows in and its safe zones, and the
    // laptop a wide one shows in: from the stage's rail and Window ▸ Viewer,
    // remembered on this Mac.
    void setDevicePreview(DevicePreview preview);
    void setLaptopPreview(LaptopPreview preview);
    void setSafeZones(bool shown);
    void showDeviceMenu();
    void updateViewerRail();
    void fitPanelDetails();
    void seekToStart() { queueSeek(0.0); }
    void seekToEnd() { queueSeek(m_duration); }
    // Step from the frame on screen so each press moves exactly one frame.
    void stepFrames(int frames);
    void stepBackward() { stepFrames(-1); }
    void stepForward() { stepFrames(1); }
    void setupStatusBar();
    QJsonValue revisionValue() const { return QJsonValue::fromVariant(QVariant::fromValue(m_revision)); }
    bool sendSimple(const QJsonObject &value);
    bool sendSeek(double position, bool scrub);
    void flushSeek();
    void handleEvent(QJsonObject event);
    void applySnapshot(const QJsonObject &snapshot);
    QString selectedTrackType() const;
    QJsonObject selectedClipObject() const;
    // The size of the selected clip's picture, in its own pixels, and
    // whether it is fitted to the canvas; empty when it has none to mask.
    QSizeF selectedPictureSize(bool *fit) const;
    // The selected picture clip's masks on the viewer, where its picture
    // lands at the playhead.
    void updateViewerMasks();
    void updateInspector();

    // A proposal is the script the agent wants to run and the diff of the
    // document it would produce.
    void updateProposal(const QJsonValue &value);
    // Show m_proposalText in m_proposalDiff, its lines coloured by kind.
    void showProposalText();
    // A proposal's script line as the change its pinned comment lists.
    ProposalChange describeScriptLine(const QString &line) const;
    void appendConversation(const QString &speaker, const QString &text);
    void appendConversationNote(const QString &kind, const QString &text);
    // The number of messages under the rail's chat button.
    void updateChatCount();
    void updateChatTips();
    // The quiet status line after the latest comment.
    void updateChatStatus();
    // The activity log, from the conversation's ⋯ menu.
    void showActivity();
    void addActivity(const QString &kind, const QString &text);
    void showError(const QString &message);
    void setConnection(const QString &status, const QString &message = QString());
    // Requests a fresh connection needs: Git status, the preset library,
    // and live sync when it was on for this project.
    void startSession();

    // Projects: the launcher, a folder, or a recent one.
    void newProject();
    void openProjectFolder();
    void openProject(const QString &path);
    void buildRecentMenu(QMenu *menu);

    // Collaboration over Git. The bridge runs every Git operation; the
    // window shows where the project stands and what a sync brought in.
    void handleCollabEvent(const QJsonObject &event);
    QJsonObject collabContext() const;
    void updateSyncChip();
    void buildBranchMenu(QMenu *menu);
    void commitChanges();
    void newBranch();
    void openPullRequest();
    void publishProject();
    bool liveSyncRemembered() const;
    void rememberLiveSync(bool enabled);

    // Presets in the project's library, and the media behind its slots.
    void savePreset();
    // The bridge refused the save in progress because `path` exists: ask,
    // then save again over it.
    void replaceSavedPreset(const QString &path);
    void applyLibraryPreset(const QString &path);
    // Swap the file behind `mediaId`; asks for the file when `path` is empty.
    void replaceMedia(const QString &mediaId, const QString &path = QString());
    void revealMedia(const QString &mediaId);
    QString mediaPath(const QString &mediaId) const;
    // The bin's context menu for `mediaId` (empty for the bin itself).
    QMenu *binMenu(const QString &mediaId);

    MediaArtwork *m_artwork = nullptr;
    QToolBar *m_toolbar = nullptr;
    // The room at the toolbar's leading end for the window buttons (macOS).
    QWidget *m_windowButtons = nullptr;
    QLabel *m_projectTitle = nullptr;
    QLabel *m_projectSubtitle = nullptr;
    QString m_projectName;
    QToolButton *m_toolbarLeftEnd = nullptr; // The last command before the title.
    // The toolbar's last control: opens and closes Kerf's conversation.
    QToolButton *m_chatToggle = nullptr;
    QString m_projectDirectory;
    QString m_kerfdBinary;
    void *m_bridge = nullptr;
    QTimer m_pollTimer;
    QTimer m_exactSeekTimer;
    QJsonObject m_snapshot;
    QJsonObject m_project;
    // The last command handed to the bridge, for tests.
    QJsonObject m_lastCommand;
    bool m_connected = false;
    QFrame *m_connectionBanner = nullptr;
    QLabel *m_connectionMessage = nullptr;
    QPushButton *m_reconnectButton = nullptr;
    QFrame *m_errorBanner = nullptr;
    QLabel *m_errorMessage = nullptr;
    QScrollArea *m_timelineScroll = nullptr;
    CommandRegistry *m_commands = nullptr;
    QHash<QString, std::function<void()>> m_commandHandlers;
    // Toolbar actions and the symbols drawn for them.
    QList<QPair<QAction *, QString>> m_toolbarGlyphs;
    QList<QPair<QAction *, TransportGlyph>> m_sourceGlyphs; // The Source monitor's marks and edits.
    // Tool palette actions, whose icons are redrawn when the appearance changes.
    QList<QAction *> m_toolGlyphs;
    // Disclosure buttons (Settings' Advanced), whose chevrons are too.
    QList<QToolButton *> m_disclosures;
    // Window ▸ Viewer: the phone (Island, Punch-Hole, Off), the laptop
    // (Laptop, Off) and safe zones.
    QList<QAction *> m_deviceActions;
    QList<QAction *> m_laptopActions;
    QAction *m_safeZonesAction = nullptr;
    // Panel details shown in the panels' title strips.
    QAction *m_viewerDetail = nullptr;
    QLabel *m_timelineDetail = nullptr;
    TimelineZoomControl *m_timelineZoom = nullptr;
    Workspace *m_workspace = nullptr;
    InspectorPanel *m_inspector = nullptr;
    // Add effect in the Inspector raised the Effects browser: adding one
    // brings the Inspector back.
    bool m_effectsFromInspector = false;
    // Rectangle, Ellipse, and Pen: the mask tools.
    QList<QAction *> m_maskTools;
    EffectsPanel *m_effects = nullptr;
    QWidget *m_nextSteps = nullptr;
    QWidget *m_chatNextSteps = nullptr;
    QJsonArray m_steps;
    QJsonObject m_lastFocus;
    QString m_lastFocusLabel;
    ProjectPanel *m_projectPanel = nullptr;
    PresetsPanel *m_presets = nullptr;
    // The name of the preset being saved, until the bridge answers.
    QString m_savingPreset;
    // project.kerf changed outside Kerf: load it or keep the editor's version.
    QFrame *m_divergedBanner = nullptr;
    QLabel *m_divergedGlyph = nullptr;
    QPushButton *m_loadFromDisk = nullptr;
    QPushButton *m_keepEditorVersion = nullptr;
    // The branch and how far it is from its upstream; a click syncs.
    QToolButton *m_syncChip = nullptr;
    QMenu *m_recentMenu = nullptr;
    QMenu *m_branchMenu = nullptr;
    // The bridge's last `collab` status, empty until one arrives.
    QJsonObject m_collabStatus;
    // The running sync's stage, empty when none runs.
    QString m_syncStage;
    // Sync Now was asked for; live sync's own cycles report only news.
    bool m_syncRequested = false;
    QString m_syncFailure;
    // What the last sync brought in, one line each.
    QStringList m_syncLines;
    QJsonObject m_mediaStatus;
    SourceMonitorPanel *m_sourceMonitor = nullptr;
    QToolBar *m_toolPalette = nullptr;
    AskPrompt *m_askPrompt = nullptr;
    QFrame *m_aiBar = nullptr;
    QLabel *m_aiBarText = nullptr;
    QString m_agentFocusLabel;
    bool m_proposalBlocked = false;
    QRadioButton *m_cloudRadio = nullptr;
    QRadioButton *m_ownKeyRadio = nullptr;
    QLabel *m_cloudStatus = nullptr;
    QLineEdit *m_cloudToken = nullptr;
    QPushButton *m_cloudSignIn = nullptr;
    QPushButton *m_cloudSignOut = nullptr;
    QWidget *m_advancedSettings = nullptr;
    QWidget *m_preferencesContent = nullptr;
    QLabel *m_statusAccount = nullptr;
    // Where the live preview draws: the GPU adapter, or the CPU.
    QLabel *m_statusDevice = nullptr;
    // The last account report from the bridge.
    QString m_cloudAccess;
    QJsonObject m_cloudAccount;
    QString m_cloudMessage;
    QDialog *m_preferences = nullptr;
    QLabel *m_statusConnection = nullptr;
    QLabel *m_statusRevision = nullptr;
    quint64 m_revision = 0;
    int m_trackCount = 0;
    double m_duration = 0.0;
    double m_fps = 24.0;
    double m_playhead = 0.0;
    double m_pendingSeek = 0.0;
    double m_pendingHoverSeek = 0.0;
    double m_shownFrameTime = 0.0;
    bool m_hasPendingSeek = false;
    bool m_hasPendingHoverSeek = false;
    bool m_pendingSeekScrub = false;
    bool m_pendingHoverScrub = false;
    bool m_playing = false;
    bool m_previewRequired = false;
    // The latest preview status: required, unavailable, running, ready, or
    // live (the compositor draws the picture).
    QString m_previewStatus;
    bool m_hoverSkimming = false;
    bool m_pointerOwnsPlayhead = false;
    bool m_hasShownFrame = false;
    bool m_updatingCanvasPreset = false;
    bool m_agentBusy = false;
    QString m_agentRunState = QStringLiteral("idle");
    QString m_agentText;
    QString m_selectedClip;
    // Every selected clip, m_selectedClip (the primary one) among them.
    QStringList m_selectedClips;
    QLabel *m_selectionStatus = nullptr;
    QString m_proposalId;
    QStringList m_activityLog;
    QSize m_canvasSize{1920, 1080};

    ViewerWidget *m_viewer = nullptr;
    TimelineWidget *m_timeline = nullptr;
    // The conversation and the rail beside the picture.
    ChatPanel *m_chat = nullptr;
    StageRail *m_rail = nullptr;
    // Kerf's comment for this turn, which streamed text and steps go to, and
    // steps that came before it.
    QPointer<CommentRow> m_agentRow;
    QStringList m_pendingReplies;
    QFrame *m_activityPopup = nullptr;
    // Parts of m_chat, by role.
    QScrollArea *m_chatScroll = nullptr;
    QLabel *m_chatActivity = nullptr;
    QFrame *m_chatComposer = nullptr;
    QTextEdit *m_chatInput = nullptr;
    QRadioButton *m_anthropicActiveRadio = nullptr;
    QRadioButton *m_openaiActiveRadio = nullptr;
    QComboBox *m_modelSelect = nullptr;
    QLineEdit *m_anthropicKey = nullptr;
    QLineEdit *m_anthropicBaseUrl = nullptr;
    QLineEdit *m_anthropicModel = nullptr;
    QLineEdit *m_openaiKey = nullptr;
    QLineEdit *m_openaiBaseUrl = nullptr;
    QLineEdit *m_openaiModel = nullptr;
    QComboBox *m_openaiMode = nullptr;
    QCheckBox *m_mcpEnabled = nullptr;
    QTextEdit *m_inboundMcpJson = nullptr;
    QLabel *m_inboundMcpStatus = nullptr;
    quint64 m_mcpConfigVersion = 0;
    QPushButton *m_sendButton = nullptr;
    QPushButton *m_abortButton = nullptr;
    QListWidget *m_activityList = nullptr;
    QLabel *m_agentStatus = nullptr;
    QLabel *m_proposalSummary = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
    QString m_proposalText; // The pending proposal's script and diff, as m_proposalDiff shows them.
    QPushButton *m_acceptButton = nullptr;
    QPushButton *m_rejectButton = nullptr;
    QFrame *m_proposalBox = nullptr;
    // The toolbar's right-hand group: the format bank, then Export (its
    // menu holds the quality and Render Preview).
    FormatBank *m_formatBank = nullptr;
    ExportButton *m_exportButton = nullptr;
    QProgressBar *m_exportProgress = nullptr;
};

} // namespace kerf
