#pragma once

// The toolbar region's wiring: the toolbar and the status bar. A friend of
// MainWindow that holds the region's own state, so a region changes the
// window's behaviour only here and in main_window_toolbar.cpp and
// main_window_status.cpp.
//
// Engine paths (plan.md section 3, mockup-spec 1, 9, 11, 14):
//   Import/Undo/Redo/   the registry's file.import, edit.undo, edit.redo,
//   Split/Delete        clip.split and clip.delete actions (enabled from
//                       kerf_commands_states, their keys from the keymap)
//   workspaces          Workspace::activate; Window ▸ Workspaces' builder
//   title               snapshot timeline.name + timeline.sequences[].name,
//                       revisionTime and revision
//   format ⌥1-4         the registry's sequence.setFormat.169/.916/.11/.45 ->
//                       sequence.setFormat {width, height} (edit set-sequence)
//   sequence button     bus frameSize/fps; Frame size presets as above;
//                       sequence.settings (⌘0) -> SequenceSettingsDialog ->
//                       edit {set-sequence {name?, width?, height?, start?,
//                       rate? (empty sequences only)}}; kerfd's refusal shows
//                       in the dialog, the next revision closes it
//   keyboard            keyboard.customise (⌥⌘K) -> KeymapEditor over the
//                       registry; one's own keys kept by KeymapCustomiser
//                       (QSettings commands/custom/<keymap>/<id>)
//   composer.draw       D while the composer has the focus: its Draw
//   Export              export {quality: master, codec: prores_hq} or
//                       {quality: review, codec: h264}; Render preview is
//                       sequence.renderPreview
//   preflight           bus lastEvent("check").summary.rows; Ask Kerf is bus
//                       openThread(commentId) + askPrefill(text, commentId);
//                       Run QC is qc.run
//   blocking notes      ReviewModel::counts().blocking (else the comments
//                       event's open blocking threads); Render master waits
//   EDL locators        deliver::option/setOption (deliver/<project>/edlLocators)
//   status: selection   bus selection + timeline clips and tracks
//   status: rate        bus fps; clock {dropped}
//   status: guest       bus viewAs / ReviewModel::viewAsName
//   status: progress    export / render_queue events for a non-master job,
//                       preview {status: running, progress}
//   status: keymap      kerf_commands_catalog keymaps (CommandRegistry)
//   status: sync        collab {status}, collab_sync; the collab.* commands
//   status: agent       the agent's access (Settings), connection {status}

#include "test_friends.h"

#include <QDateTime>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QMenu>
#include <QObject>
#include <QPair>
#include <QPointer>
#include <QSize>
#include <QString>
#include <QStringList>

#include <functional>

class QAction;
class QTimer;
class QWidget;

namespace kerf {

class AgentStatus;
class KeymapCustomiser;
class KeymapEditor;
class SequenceSettingsDialog;
class EditorBus;
class ExportButton;
class FormatBank;
class MainWindow;
class StatusChip;
class StatusProgress;
class StatusRow;
class StatusText;
class ToolbarSegments;

namespace ui {
class GhostButton;
class IconButton;
} // namespace ui

class ToolbarWiring final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
    friend class MainWindow;
public:
    ToolbarWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // ---- built by MainWindow::setupToolbar and setupStatusBar
    void buildToolbar(const QHash<QString, QMenu *> &menus);
    void buildStatusBar();

    // ---- the title, centred on the window
    // "Atlas · 30 s spot v7": the project, then the program sequence's name.
    QString titleText() const;
    // "Edited 2 min ago · rev 22" (just "rev 22" without a revision time).
    QString subtitleText() const;
    void layoutTitle();
    // The clock relative times are read against (tests set their own).
    void setClock(std::function<QDateTime()> clock);

    // ---- the window buttons' room
    // Where the first control starts: 16 in, past the macOS window buttons
    // (52 + 16 reserved), or 16 without them.
    static int firstControlX(int windowButtonsRight, bool fullScreen);

    // ---- export and preflight
    struct PreflightRow {
        QString code;
        QString label;    // "True peak · −0.4 dBTP"
        QString key;      // the limit, for a passed row
        QString ask;      // the Ask Kerf text, for a failed row
        QString commentId;
        bool passed = false;
    };
    // The rows the Export menu shows: every failed row (at most `most`) and
    // passed rows, in the check's order, until there are `least`.
    static QList<PreflightRow> preflightRows(const QJsonObject &summary, int least = 4, int most = 6);
    // The failed rows the menu leaves out.
    static int preflightHidden(const QJsonObject &summary, int most = 6);
    // "@Kerf bring true peak to ≤ −1 dBTP (now −0.4 dBTP)" for a failed row.
    static QString askText(const QJsonObject &row);
    // Open threads with an open blocking comment, from a comments event.
    static int blockingThreads(const QJsonArray &comments);
    // The blocking notes open now (the review model's count when there is
    // one, else the last comments event's).
    int blockingCount() const;
    // Sends export {quality, codec} for "master" or "review".
    bool sendExport(const QString &quality);
    // The Export menu, built each time it opens.
    void buildExportMenu();
    // The QSettings key of the EDL-locators choice for this project: the
    // Deliver paperwork's own (deliver::settingsKey); empty without a project.
    QString edlLocatorsKey() const;

    // ---- status items
    // "B003_C011 · V1 · " and the mono "00:00:08:12–00:00:13:05".
    static QPair<QString, QString> selectionText(const QJsonObject &timeline, const QStringList &selection, double fps);
    // "25p · 0 dropped" ("25p" before the first clock).
    QString rateText() const;
    // "Premiere keys" for the keymap labelled "Premiere Pro".
    static QString keymapChipText(const QString &label);
    // The sync chip and its menu's header: "Revision 22 · 2 commits to pull".
    QString syncSummary() const;
    void updateSyncChip();
    void updateAgent();
    void updateSelection();
    void updateRate();
    void updateGuest();
    void updateKeymap();
    void updateProgress();
    // Keeps a non-master render's label and progress, by jobId.
    void trackExport(const QString &job, const QString &quality, const QString &name, double fraction, bool queued);

    // ---- menus (built each time they open)
    void buildSequenceMenu(QMenu *menu);
    void buildSyncMenu(QMenu *menu);
    void buildKeymapMenu(QMenu *menu);

    // Follows the registry: enablement and tips of the toolbar's commands.
    void syncCommands();

    // ---- the commands the window carries out for the features (W3)
    // The registry's format commands, in canvasPresets() order.
    static QStringList formatCommands();
    // Registers sequence.setFormat.*, sequence.settings,
    // keyboard.customise and composer.draw, and the bin's and the source's.
    void installCommands();
    // Sequence Settings…: opens (or raises) the dialog over the program
    // sequence; it is not modal, so the window goes on as it is open.
    SequenceSettingsDialog *openSequenceSettings();
    SequenceSettingsDialog *sequenceSettings() const;
    // Sends the dialog's set-sequence op as an edit.
    bool applySequenceSettings(const QJsonObject &op);
    // Customise Keyboard…
    KeymapEditor *openKeymapEditor();
    KeymapEditor *keymapEditor() const;
    KeymapCustomiser *keymapCustomiser() const { return m_customiser; }
    // composer.draw: the composer's Draw when it has the focus.
    bool drawInComposer();
    void setWorkspace(const QString &name);
    void updateSequence();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void followReview();
    void chooseFormat(int index);
    QString projectLabel() const;

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    std::function<QDateTime()> m_clock;

    // Toolbar.
    QWidget *m_content = nullptr;
    QList<QPair<ui::IconButton *, QString>> m_commandButtons; // button, command id
    ui::IconButton *m_appMenu = nullptr;
    ToolbarSegments *m_workspaces = nullptr;
    ui::GhostButton *m_sequence = nullptr;
    ui::IconButton *m_settings = nullptr;
    QMenu *m_workspaceMenu = nullptr;
    QMenu *m_sequenceMenu = nullptr;
    QList<QAction *> m_formatActions; // ⌥1-4: the registry's sequence.setFormat.*
    KeymapCustomiser *m_customiser = nullptr;
    QPointer<KeymapEditor> m_keymapEditor;
    QPointer<SequenceSettingsDialog> m_sequenceSettings;
    // A set-sequence edit sent from the dialog, and the revision it went at.
    bool m_sequencePending = false;
    quint64 m_sequenceRevision = 0;
    // The set-sequence op that is waiting for kerfd's answer.
    QJsonObject m_sequenceOp;
    QTimer *m_titleTimer = nullptr;

    // Status bar.
    StatusRow *m_row = nullptr;
    StatusText *m_selection = nullptr;
    StatusText *m_rate = nullptr;
    StatusText *m_guest = nullptr;
    StatusProgress *m_progress = nullptr;
    StatusChip *m_keymap = nullptr;
    AgentStatus *m_agent = nullptr;
    QMenu *m_syncMenu = nullptr;
    QMenu *m_keymapMenu = nullptr;
    // Holds the window's older status labels (deprecated, removed in W3)
    // out of sight: they are still written to by main_window.cpp.
    QWidget *m_retired = nullptr;

    // State.
    QJsonObject m_checkSummary; // the last finished check's summary
    QString m_checkStatus;      // "", running, done, failed
    double m_checkProgress = 0;
    QString m_checkMessage;
    qint64 m_dropped = -1; // frames dropped, -1 before the first clock
    QString m_connection = QStringLiteral("connecting");
    QHash<QString, QString> m_jobQuality; // render_queue jobId -> quality
    QString m_sentQuality;                // what the toolbar last exported
    // The non-master renders running or queued, by jobId (the status bar
    // shows the one started last, a running one before a queued one; the
    // master's progress is Deliver's).
    struct RunningExport {
        QString label;
        double fraction = 0;
        quint64 started = 0;
        bool queued = false;
    };
    QHash<QString, RunningExport> m_runningExports;
    quint64 m_exportsStarted = 0;
    double m_previewFraction = 0;
    bool m_previewRunning = false;
    QPointer<QObject> m_review;
};

} // namespace kerf
