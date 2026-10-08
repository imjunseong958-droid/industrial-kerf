#pragma once

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

namespace cut {

class AskPrompt;
class CommandRegistry;
class EffectControlsPanel;
class ProjectPanel;
class SourceMonitorPanel;
class Workspace;
class MediaArtwork;
class TimelineWidget;
class ViewerWidget;

class MainWindow final : public QMainWindow {
    Q_OBJECT
    friend class EditorTest;
public:
    MainWindow(const QString &projectDirectory, const QString &cutdBinary, QWidget *parent = nullptr);
    ~MainWindow() override;

private slots:
    void pollBridge();
    void importMedia();
    void exportProject();
    void exportEditingPreset();
    void importEditingPreset();
    void showPresetBindings(const QString &path, const QJsonObject &preset);
    void renderPreview();
    void canvasPresetChanged(int index);
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

    // Edits go to cutd as typed operations on the model. Amounts and
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
    // Ask Cut about the selected clip, or the sequence at the playhead.
    void askAi();
    void askSubmitted(const QString &text, const QString &scope);
    void updateAiBar();
    // Cut Cloud, the hosted model, or a key of one's own (Advanced).
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

protected:
    bool eventFilter(QObject *object, QEvent *event) override;
    void closeEvent(QCloseEvent *event) override;

private:
    void buildUi();
    // Window ▸ Workspaces: the workspaces, and saving or resetting the current one.
    void buildWorkspaceMenu(QMenu *menu);
    void syncCanvasPreset(const QSize &size);
    void setupActions();
    // Resolve a command in the current context and send its request.
    bool execute(const QString &id, const QJsonObject &params = {});
    // What commands read: connection, selection, playhead, history, preview.
    QJsonObject commandContext() const;
    void updateCommandStates();

    // Transport under the viewer: playhead timecode, the usual buttons, and
    // the program length.
    QWidget *buildTransport();
    void updateTransport();
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
    void updateInspector();

    // A proposal is the script the agent wants to run and the diff of the
    // document it would produce.
    void updateProposal(const QJsonValue &value);
    void appendConversation(const QString &speaker, const QString &text);
    void appendConversationNote(const QString &kind, const QString &text);
    void addActivity(const QString &kind, const QString &text);
    void showError(const QString &message);
    void setConnection(const QString &status, const QString &message = QString());

    MediaArtwork *m_artwork = nullptr;
    QLabel *m_projectTitle = nullptr;
    QString m_projectDirectory;
    QString m_cutdBinary;
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
    QList<QToolButton *> m_transportButtons;
    QToolButton *m_transportPlay = nullptr;
    QLabel *m_transportTime = nullptr;
    QLabel *m_transportDuration = nullptr;
    // Panel details shown in the panels' title strips.
    QAction *m_viewerDetail = nullptr;
    QAction *m_timelineDetail = nullptr;
    Workspace *m_workspace = nullptr;
    EffectControlsPanel *m_effectControls = nullptr;
    ProjectPanel *m_projectPanel = nullptr;
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
    QLabel *m_statusAccount = nullptr;
    // The last account report from the bridge.
    QString m_cloudAccess;
    QJsonObject m_cloudAccount;
    QString m_cloudMessage;
    QDialog *m_preferences = nullptr;
    QLabel *m_statusConnection = nullptr;
    QLabel *m_statusRevision = nullptr;
    bool m_transportShowsPause = false;
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
    // The latest preview status: required, unavailable, running, or ready.
    QString m_previewStatus;
    bool m_hoverSkimming = false;
    bool m_pointerOwnsPlayhead = false;
    bool m_hasShownFrame = false;
    bool m_updatingCanvasPreset = false;
    bool m_conversationEmpty = true;
    bool m_agentBusy = false;
    QString m_agentRunState = QStringLiteral("idle");
    QString m_agentText;
    QString m_selectedClip;
    QString m_proposalId;
    QStringList m_activityLog;
    QSize m_canvasSize{1920, 1080};

    ViewerWidget *m_viewer = nullptr;
    TimelineWidget *m_timeline = nullptr;
    QTextEdit *m_conversation = nullptr;
    QTextCursor m_agentCursor;
    QLabel *m_chatActivity = nullptr;
    QFrame *m_chatComposer = nullptr;
    QLabel *m_composerStatus = nullptr;
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
    QToolButton *m_abortButton = nullptr;
    QListWidget *m_activityList = nullptr;
    QLabel *m_agentStatus = nullptr;
    QLabel *m_proposalSummary = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
    QPushButton *m_acceptButton = nullptr;
    QPushButton *m_rejectButton = nullptr;
    QGroupBox *m_proposalBox = nullptr;
    QComboBox *m_quality = nullptr;
    QComboBox *m_canvasPreset = nullptr;
    QProgressBar *m_exportProgress = nullptr;
};

} // namespace cut
