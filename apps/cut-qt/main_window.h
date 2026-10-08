#pragma once

#include <QAction>
#include <QCheckBox>
#include <QComboBox>
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
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QTextCursor>
#include <QTextEdit>
#include <QTimer>
#include <QToolButton>
#include <QVector>
#include <QWidget>

#include <cmath>
#include <functional>

namespace cut {

class CommandRegistry;
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
    void clipValueEdited();

    // A catalog property edit; the default value clears the property.
    void propertyEdited(const QString &name, const QJsonValue &value);

protected:
    bool eventFilter(QObject *object, QEvent *event) override;

private:
    void buildUi();
    QDoubleSpinBox *inspectorSpin(double value, double minimum, double maximum, double step, const QString &field);

    // An editor for one catalog property definition.
    QWidget *propertyEditor(const QJsonObject &def);
    static void setPropertyValue(QWidget *editor, const QJsonValue &value);
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
    QWidget *m_agentPanel = nullptr;
    QScrollArea *m_timelineScroll = nullptr;
    CommandRegistry *m_commands = nullptr;
    QHash<QString, std::function<void()>> m_commandHandlers;
    QList<QToolButton *> m_transportButtons;
    QToolButton *m_transportPlay = nullptr;
    QLabel *m_transportTime = nullptr;
    QLabel *m_transportDuration = nullptr;
    QLabel *m_viewerDetail = nullptr;
    QLabel *m_timelineDetail = nullptr;
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
    bool m_updatingInspector = false;
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
    QLabel *m_inspectorClip = nullptr;
    QDoubleSpinBox *m_position = nullptr;
    QDoubleSpinBox *m_durationSpin = nullptr;
    QDoubleSpinBox *m_inSpin = nullptr;
    QDoubleSpinBox *m_outSpin = nullptr;
    QDoubleSpinBox *m_speedSpin = nullptr;
    QFormLayout *m_propertyForm = nullptr;
    QJsonArray m_propertyDefs;
    QHash<QString, QWidget *> m_propertyEditors;
    QComboBox *m_quality = nullptr;
    QComboBox *m_canvasPreset = nullptr;
    QProgressBar *m_exportProgress = nullptr;
};

} // namespace cut
