#pragma once

#include "editor_common.h"
#include "review_model.h"
#include "test_friends.h"

#include <QAction>
#include <QSet>
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
#include <memory>

struct KerfFrame; // bridge.h

namespace kerf {

class AskPrompt;
class ChatPanel;
class EditorBus;
// The regions' wiring (ui/wiring/<region>_wiring.h), each a friend that holds
// its region's own state.
class BinWiring;
class ReviewWiring;
class StageWiring;
class TimelineWiring;
class ToolbarWiring;
struct ProposalChange;
struct CutPicture;
class CommentRow;
class CommandRegistry;
class ExportButton;
class FormatBank;
class ProjectPanel;
class PreviewChip;
class ChangeReel;
class ChangeReelBar;
class ProjectSidebar;
class SidebarRow;
class StageMotion;
class StageShell;
namespace ui {
class IconButton;
} // namespace ui
class MediaArtwork;
class TimelineWidget;
class ViewerWidget;
} // namespace kerf

class QStackedWidget;

namespace kerf {

// The editor's window. Its content is a StageShell (ui/stage_shell.h): the
// picture, the conversation and the timeline, the rest on demand. Its member
// functions are split by region into main_window_<region>.cpp (review, stage,
// toolbar, status, timeline, bin), each region's own state in
// its wiring class; the regions talk through the EditorBus (ui/editor_bus.h) it
// owns.
class MainWindow final : public QMainWindow {
    Q_OBJECT
    KERF_TEST_FRIENDS
    friend class BinWiring;
    friend class ReviewWiring;
    friend class StageWiring;
    friend class TimelineWiring;
    friend class ToolbarWiring;
public:
    MainWindow(const QString &projectDirectory, const QString &kerfdBinary, QWidget *parent = nullptr);
    ~MainWindow() override;
    // The bus the window's regions share.
    EditorBus *bus() const { return m_bus; }
    // The project folder this editor has open (absolute; empty for none).
    QString projectDirectory() const { return m_projectDirectory; }
    // The slugs a kerf:// review link may name this project by: its
    // folder's and its document's name (links::slug).
    QStringList projectSlugs() const;
    // A kerf:// link followed to this project: once it is connected and its
    // programme is known, seek to `timecode` ("HH:MM:SS:FF"; empty stays at
    // the playhead), and say so when the link names another revision.
    void followLink(qint64 revision, const QString &timecode);
    bool linkPending() const { return m_linkPending; }
    // What to do once the project is connected: bring the folder's media in
    // (a folder that was no project yet), then send Kerf the request.
    void startWith(const QString &request, bool importFolder);
    // Takes the place of a start window that looked like `look`, its request box
    // flying from `box` to the conversation's.
    void takeOverFrom(const QPixmap &look, const QRect &box);

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
    // ⌘-click or ⇧-click on a clip in the timeline.
    void extendClipSelection(const QString &id, bool toggle);
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
    // The media chosen in the bin, as the project describes it; empty for none.
    QJsonObject binMedia() const;
    // The tracks Insert and Overwrite place the bin's media on.
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
    // Kerf's next steps as buttons in the AI bar and over the composer;
    // pressing one asks for it with the focus of the last request.
    void showNextSteps(const QJsonArray &steps);
    // Kerf Cloud, the hosted model, or a key of one's own (Advanced).
    bool usesCloud() const;
    bool usesClaudeCode() const;
    // Signed in to Kerf Cloud, whichever connection chat uses.
    bool cloudSignedIn() const;
    // Who the sidebar's foot says is here: the account, or the connection.
    void updateAccountRow();
    // Settings, at `page` ("account", "ai", "general", "advanced"; empty: where it was).
    void showSettings(const QString &page = QString());
    bool sendCloud(const QJsonObject &command);
    // What the registry needs to know about the agent's connection.
    QString aiAccess() const;
    void updateCloudStatus();
    void updateComposerHint();
    void updateChatControls();
    QString activeProviderId() const;
    QJsonObject agentSettingsPayload() const;
    void loadInboundMcpConfig(const QString &path, bool imported);
    void saveInboundMcpConfig();
    void updateAgentModelChoices();
    void loadAgentSettings();
    void saveAgentSettings();
    // Help > About Kerf: the version, license and where the source and releases are.
    void showAbout();
    // A timeline with its clips named after their media, where they have no name.
    static QJsonObject withClipNames(QJsonObject timeline, const QJsonArray &sources);

protected:
    bool eventFilter(QObject *object, QEvent *event) override;
    void closeEvent(QCloseEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void changeEvent(QEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;

private:
    // A program frame from the bridge (pollBridge): shown in the viewer and
    // handed to the bus; false when it is not shown (offline, short, or
    // farther from the scrub target than the one on screen).
    bool showProgramFrame(const KerfFrame &frame);
    void buildUi();
    // The regions' parts of buildUi, each in its region's file.
    void buildReview();
    void buildStage();
    // The picture's mask tools (main_window_stage.cpp).
    void buildMaskTools();
    void buildTimeline();
    void buildBin();
    // Settings (main_window.cpp).
    void buildPreferences();
    // The shell: the regions, the sheets and the bars, once the
    // regions are built.
    void buildShell();
    // The timeline's gestures, once every region exists (main_window_timeline.cpp).
    void wireTimeline();
    // The toolbar, once the registry's actions exist (main_window_toolbar.cpp).
    void setupToolbar(const QHash<QString, QMenu *> &menus);
    // The picture's commands, once the registry's actions exist.
    void setupPictureCommands();
    // The sheets' commands checked as they show.
    void syncPanelCommands();
    // A registry command by id for the bus: a frontend command runs its
    // handler, an engine command goes through execute().
    bool runCommand(const QString &id, const QJsonObject &params);
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
    void syncCanvasPreset(const QSize &size);
    void setupActions();
    // Resolve a command in the current context and send its request.
    bool execute(const QString &id, const QJsonObject &params = {});
    // What commands read: connection, selection, playhead, history, preview.
    QJsonObject commandContext() const;
    void updateCommandStates();

    // Safe zones over the picture, remembered on this Mac; off by default.
    void setSafeZones(bool shown);
    void seekToStart() { queueSeek(0.0); }
    void seekToEnd() { queueSeek(m_duration); }
    // Step from the frame on screen so each press moves exactly one frame.
    void stepFrames(int frames);
    void stepBackward() { stepFrames(-1); }
    void stepForward() { stepFrames(1); }
    // A passing message in the toolbar's subtitle, for `timeout` ms (0: until
    // the next one), then the normal subtitle again.
    void showStatus(const QString &message, int timeout = 0);
    QJsonValue revisionValue() const { return QJsonValue::fromVariant(QVariant::fromValue(m_revision)); }
    bool sendSimple(const QJsonObject &value);
    bool sendSeek(double position, bool scrub);
    void flushSeek();
    // A bridge event: the window handles it (routeEvent), then the bus sends
    // it on to the regions.
    void handleEvent(QJsonObject event);
    void routeEvent(const QJsonObject &event);
    // The agent's events (agent, agent_busy, agent_state, agent_next); false
    // for any other (main_window_review.cpp).
    bool handleAgentEvent(const QJsonObject &event);
    void applySnapshot(const QJsonObject &snapshot);
    QString selectedTrackType() const;
    QJsonObject selectedClipObject() const;
    // The size of the selected clip's picture, in its own pixels, and
    // whether it is fitted to the canvas; empty when it has none to mask.
    QSizeF selectedPictureSize(bool *fit) const;
    // What a picture clip on a track of `trackType` draws, in its own pixels; empty
    // for one that draws none. `fit` says whether it is fitted to the canvas.
    QSizeF pictureSizeOf(const QJsonObject &clip, const QString &trackType, bool *fit) const;
    // What the parents of `clip` have done to it at sequence time `at`, in canvas pixels.
    QTransform parentsMotion(const QJsonObject &clip, const QSizeF &canvas, double at) const;
    // A selected picture clip with no `path=` and not in depth can start one: two
    // points from where it is, and `along` keyed across the clip, as one edit.
    bool canStartMotionPath(const QJsonObject &clip) const;
    void startMotionPath();
    // The selected picture clip's masks on the viewer, where its picture
    // lands at the playhead.
    void updateViewerMasks();

    // A proposal is the script the agent wants to run and the diff of the
    // document it would produce.
    void updateProposal(const QJsonValue &value);
    // A proposal's script line as the change its pinned comment lists.
    ProposalChange describeScriptLine(const QString &line) const;
    void appendConversation(const QString &speaker, const QString &text);
    void appendConversationNote(const QString &kind, const QString &text);
    // The quiet status line after the latest comment.
    void updateChatStatus();
    // The activity log, from the conversation's ⋯ menu.
    void showActivity();
    void addActivity(const QString &kind, const QString &text);
    void showError(const QString &message);
    void setConnection(const QString &status, const QString &message = QString());
    // Requests a fresh connection needs: Git status,
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
    // Swap the file behind `mediaId`; asks for the file when `path` is empty.
    void replaceMedia(const QString &mediaId, const QString &path = QString());
    void revealMedia(const QString &mediaId);
    QString mediaPath(const QString &mediaId) const;
    // The recipe `mediaId` names (recipe= on its media line), or empty.
    QString mediaRecipe(const QString &mediaId) const;
    // The takes a media's recipe has made, as the project lists them: each one's number,
    // its path, and whether it is the one the media is.
    QJsonArray mediaTakes(const QString &mediaId) const;
    void useTake(const QString &mediaId, int take, const QString &path);
    // Ask the bridge what making `mediaId` from its recipe would run; it
    // answers media_make_plan and runs nothing.
    void makeMedia(const QString &mediaId);
    // What a person is asked before a recipe's command is run, from a
    // media_make_plan event.
    static QString makeQuestion(const QJsonObject &plan);
    // Run the recipe of `mediaId`: only after a person's yes to its plan.
    void runMake(const QString &mediaId);
    // Stop the making of `mediaId` that is running: its step is asked to end.
    void stopMake(const QString &mediaId);
    // What the toolbar says of a making under way, from a media_making event.
    static QString makingStatus(const QJsonObject &making);
    // The bin's context menu for `mediaId` (empty for the bin itself).
    QMenu *binMenu(const QString &mediaId);

    EditorBus *m_bus = nullptr;
    std::unique_ptr<ReviewWiring> m_reviewWiring;
    std::unique_ptr<StageWiring> m_stageWiring;
    std::unique_ptr<ToolbarWiring> m_toolbarWiring;
    std::unique_ptr<TimelineWiring> m_timelineWiring;
    std::unique_ptr<BinWiring> m_binWiring;
    // The regions' containers: the shell and the timeline's panel.
    StageShell *m_shell = nullptr;
    ProjectSidebar *m_sidebar = nullptr;
    ChangeReel *m_changeReel = nullptr;
    ChangeReelBar *m_reelBar = nullptr;
    StageMotion *m_motion = nullptr;
    // The sheet of footage shown as Kerf starts on an empty project: what flies
    // from it next, then the rest of the conversation's foot.
    bool m_holdFlights = false;
    class WorkChip *m_workChip = nullptr;
    QList<ReviewModel::ClipChange> m_pendingFlights;
    ui::IconButton *m_followButton = nullptr;
    bool m_followKerf = true;
    // How far a seek may go past the program: what a proposal makes can be longer.
    double m_seekLimit = 0.0;
    QSet<QString> m_liveChanges;
    bool m_returnAfterPreview = false;
    ui::IconButton *m_sidebarButton = nullptr;
    // The one button for the panel beside the conversation, and in the
    // timeline's strip the way to the Media sheet.
    ui::IconButton *m_panelButton = nullptr;
    ui::IconButton *m_mediaButton = nullptr;
    bool m_panesFollowContent = true;
    QString m_startRequest;
    bool m_startImport = false;
    QWidget *m_timelinePanel = nullptr;
    MediaArtwork *m_artwork = nullptr;
    QToolBar *m_toolbar = nullptr;
    // The room at the toolbar's leading end for the window buttons (macOS).
    QWidget *m_windowButtons = nullptr;
    QLabel *m_projectTitle = nullptr;
    QLabel *m_projectSubtitle = nullptr;
    QString m_projectName;
    // A followed kerf:// link not yet applied (followLink).
    bool m_linkPending = false;
    qint64 m_linkRevision = -1;
    QString m_linkTimecode;
    void applyPendingLink();
    // The keys to the timeline when Qt handed them to a button as if Tab had
    // been pressed: after a sheet closes, and as the window first
    // shows and first becomes active (m_focusSettled).
    void settleFocus();
    // The sidebar and the panel beside the conversation.
    void setPaneOpen(const QString &command, bool open);
    // The picture and the timeline together, as the toolbar's button has them.
    void setPanelOpen(bool open);
    void revealPanel(bool picture, bool timeline, const QRect &origin = QRect());
    void applyPanes();
    // The panel opens by itself for a program there is to see, until one opens
    // or closes a pane by hand. A cut Kerf makes stays in the conversation, as
    // its card's picture, until that is pressed.
    void followContentWithPanes();
    // The stage opens out of `origin`, `poster` standing in for the picture
    // until its first frame; the timeline comes in after it.
    void growStage(const QRect &origin, const QImage &poster, bool waitForRender);
    // A first cut's picture for its card; empty for a proposal on a program.
    CutPicture cutPicture(const QString &proposalId) const;
    // A card's picture was pressed: the stage grows out of it.
    void openCutFromCard(const QString &proposalId);
    // Where the picture goes back to as the stage closes: the card it came
    // out of, while that is in sight; else empty.
    QRect cardPictureHome() const;
    void refreshSidebar();
    void buildSidebar();
    // Kerf at work (main_window_agent.cpp).
    void buildAgentViews();
    void setFollowKerf(bool follow);
    void showAgentAt(double seconds);
    void followAgentFocus(const QJsonObject &event);
    void followLiveProposal();
    void endAgentFollow();
    QList<ReviewModel::ClipChange> namedChanges(QList<ReviewModel::ClipChange> changes) const;
    void watchChanges(const QString &proposalId);
    void showSheet();
    void showProposedCut();
    void goHome();
    bool m_turnRendered = false;
    void followReelCompare();
    QString m_reelCompareAt;
    bool m_reelWasActive = false;
    void quietenCutPreview();
    void flyChange(const ReviewModel::ClipChange &change, int index);
    void flushFlights();
    QRect flightStart(const QString &source = QString());
    QString sourceOfClip(const QString &clip) const;
    void coverPicture(const QImage &image);
    QRect toShell(const QWidget *widget, const QRect &rect) const;
    QImage posterForClip(const QString &clip) const;
    void flyExportHome();
    void runStart();
    // Closes a sheet that lies over the composer.
    void uncoverComposer();
    bool m_focusSettled = false;
    QString m_projectDirectory;
    QString m_kerfdBinary;
    void *m_bridge = nullptr;
    QTimer m_pollTimer;
    QTimer m_exactSeekTimer;
    // The cut Kerf is proposing, shown while the program has none.
    QTimer m_cutPreview;
    QString m_previewedCut;
    bool m_cutRendering = false;
    double m_proposedDuration = 0.0;
    bool m_sheetWanted = false;
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
    // Tool palette actions, whose icons are redrawn when the appearance changes.
    QList<QAction *> m_toolGlyphs;
    // Rectangle, Ellipse, and Pen: the mask tools.
    QList<QAction *> m_maskTools;
    QJsonObject m_lastFocus;
    QString m_lastFocusLabel;
    ProjectPanel *m_projectPanel = nullptr;
    // The name of the preset being saved, until the bridge answers.
    QString m_savingPreset;
    // project.kerf changed outside Kerf: load it or keep the editor's version.
    QFrame *m_divergedBanner = nullptr;
    QLabel *m_divergedGlyph = nullptr;
    QPushButton *m_loadFromDisk = nullptr;
    QPushButton *m_keepEditorVersion = nullptr;
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
    // The media being made from their recipes, each with the line its steps
    // last said: what the bin's menu offers to stop.
    QHash<QString, QString> m_making;
    AskPrompt *m_askPrompt = nullptr;
    QString m_agentFocusLabel;
    bool m_proposalBlocked = false;
    QRadioButton *m_cloudRadio = nullptr;
    QRadioButton *m_ownKeyRadio = nullptr;
    QRadioButton *m_claudeCodeRadio = nullptr;
    QLabel *m_cloudStatus = nullptr;
    QLineEdit *m_cloudToken = nullptr;
    QPushButton *m_cloudSignIn = nullptr;
    QPushButton *m_cloudSignOut = nullptr;
    QWidget *m_advancedSettings = nullptr;
    // Where Kerf Cloud stands, on the Account page.
    QLabel *m_accountStatus = nullptr;
    // The last account report from the bridge.
    QString m_cloudAccess;
    QJsonObject m_cloudAccount;
    QString m_cloudMessage;
    // Settings: a page the shell lays over the conversation and the panel. Its
    // body is what comes up into place; its pages are named down its side.
    QWidget *m_preferences = nullptr;
    QWidget *m_settingsBody = nullptr;
    QStackedWidget *m_settingsPages = nullptr;
    QStringList m_settingsIds;
    QList<SidebarRow *> m_settingsNav;
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
    QString m_agentText;
    QString m_selectedClip;
    // Every selected clip, m_selectedClip (the primary one) among them.
    QStringList m_selectedClips;
    QString m_proposalId;
    QStringList m_activityLog;
    QSize m_canvasSize{1920, 1080};

    ViewerWidget *m_viewer = nullptr;
    TimelineWidget *m_timeline = nullptr;
    // The conversation, always open.
    ChatPanel *m_chat = nullptr;
    // Kerf's comment for this turn, which streamed text and steps go to, and
    // steps that came before it.
    QPointer<CommentRow> m_agentRow;
    QStringList m_pendingReplies;
    QFrame *m_activityPopup = nullptr;
    // Parts of m_chat, by role.
    QScrollArea *m_chatScroll = nullptr;
    // The agent's latest progress line, for the turn's drafting row.
    QString m_activityDetail;
    QFrame *m_chatComposer = nullptr;
    QTextEdit *m_chatInput = nullptr;
    QRadioButton *m_anthropicActiveRadio = nullptr;
    QRadioButton *m_openaiActiveRadio = nullptr;
    // The model each chat request names (the More menu's Model, else the
    // active connection's), and the ones offered.
    QString m_agentModel;
    QStringList m_agentModels;
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
    // The format bank, in the timeline's strip, and the toolbar's Export (its
    // menu holds the quality and Render Preview).
    FormatBank *m_formatBank = nullptr;
    ExportButton *m_exportButton = nullptr;
};

} // namespace kerf
