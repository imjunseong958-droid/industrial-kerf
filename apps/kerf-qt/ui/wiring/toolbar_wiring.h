#pragma once

// The toolbar region's wiring. A friend of MainWindow that holds the region's
// own state, so a region changes the window's behaviour only here and in
// main_window_toolbar.cpp and main_window_status.cpp.
//
// The toolbar (mockup, minimal pass; 40 tall): the traffic lights' room, one
// ≡ button (appMenu), the centred title and subtitle, the two format icons and
// the Export split. Engine paths:
//   ≡ menu              the registry's commands: window.project, window.effects,
//                       window.inspector (the sheets), workspace.finish,
//                       workspace.deliver (the trays), file.import,
//                       sequence.settings, app.preferences; View ▸ the mask
//                       tools, view.annotations, view.safeZones, viewer.zoom,
//                       the keymap choice and the collab.* commands
//   title               snapshot timeline.name + timeline.sequences[].name,
//                       revisionTime and revision
//   subtitle            as the status bar's successor: a passing message for
//                       its timeout (MainWindow::showStatus), else offline or
//                       an error, which stay, else the sync's stage or a
//                       render's progress (export, render_queue, preview), else
//                       "Edited 2 min ago · rev 22"
//   format ⌥1-2         the registry's sequence.setFormat.169/.916 ->
//                       sequence.setFormat {width, height} (edit set-sequence)
//   sequence settings   sequence.settings (⌘0) -> SequenceSettingsDialog ->
//                       edit {set-sequence {name?, width?, height?, start?,
//                       rate? (empty sequences only)}}; kerfd's refusal shows
//                       in the dialog, the next revision closes it
//   composer.draw       D while the composer has the focus: its Draw
//   Export              export {quality: master, codec: prores_hq} or
//                       {quality: review, codec: h264}; Render preview is
//                       sequence.renderPreview
//   preflight           bus lastEvent("check").summary.rows; Ask Kerf is bus
//                       openThread(commentId) + askPrefill(text, commentId);
//                       Run QC is qc.run
//   blocking notes      ReviewModel::counts().blocking (else the comments
//                       event's open blocking threads); Render master and
//                       Export's master wait (kerfd refuses them too)
//   EDL locators        deliver::option/setOption (deliver/<project>/edlLocators)

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

class SequenceSettingsDialog;
class EditorBus;
class ExportButton;
class FormatBank;
class MainWindow;

namespace ui {
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

    // ---- built by MainWindow::setupToolbar
    void buildToolbar(const QHash<QString, QMenu *> &menus);
    // The ≡ menu, built each time it opens; its View submenu with it.
    QMenu *appMenu() const { return m_appMenu; }
    void buildAppMenu();
    ui::IconButton *appMenuButton() const { return m_appButton; }

    // ---- the title, centred on the window
    // "Atlas · 30 s spot v7": the project, then the program sequence's name.
    QString titleText() const;
    // The line under the title: a passing message, else offline or an error,
    // else the sync's stage or a render's progress, else the normal line.
    QString subtitleText() const;
    // "Edited 2 min ago · rev 22" (just "rev 22" without a revision time),
    // with "Viewing as Ana" while a guest's view is on.
    QString normalSubtitle() const;
    // What stays until it clears: "Offline", "Connecting…", the error shown,
    // a failed sync; empty when all is well.
    QString standingProblem() const;
    // What runs now: "Pulling…", "Draft export · 42%", "Rendering preview · 8%".
    QString runningText() const;
    // A passing message; timeout 0 keeps it until the next one.
    void showMessage(const QString &message, int timeout);
    QString message() const { return m_message; }
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
    // Open threads with an open blocking comment, from a comments event:
    // their roots, in the order they were made (kerfd's rule, review.rs
    // blocking_threads).
    static QJsonArray blockingRoots(const QJsonArray &comments);
    static int blockingThreads(const QJsonArray &comments) { return int(blockingRoots(comments).size()); }
    // The blocking notes open now (the review model's count when there is
    // one, else the last comments event's).
    int blockingCount() const;
    // Sends export {quality, codec} for "master" or "review". A master waits
    // while a blocking note is open, as Deliver's Render master does: it is
    // not sent, and holdForNotes says why.
    bool sendExport(const QString &quality);
    // Says which note holds a master back and where, and shows the blocking
    // notes in Review to resolve; `refusal` is kerfd's, when it refused.
    void holdForNotes(const QString &refusal = QString());
    // The Export menu, built each time it opens.
    void buildExportMenu();
    // The QSettings key of the EDL-locators choice for this project: the
    // Deliver paperwork's own (deliver::settingsKey); empty without a project.
    QString edlLocatorsKey() const;

    // "Revision 22 · 2 commits to pull": the Sync menu's header.
    QString syncSummary() const;
    // Keeps a render's label and progress, by jobId.
    void trackExport(const QString &job, const QString &quality, const QString &name, double fraction, bool queued);

    // ---- menus (built each time they open)
    void buildSyncMenu(QMenu *menu);
    void buildKeymapMenu(QMenu *menu);

    // Follows the registry: enablement and tips of the toolbar's commands.
    void syncCommands();

    // ---- the commands the window carries out for the features (W3)
    // The registry's format commands, in canvasPresets() order.
    static QStringList formatCommands();
    // Registers sequence.setFormat.*, sequence.settings and composer.draw,
    // and the bin's and the source's.
    void installCommands();
    // Sequence Settings…: opens (or raises) the dialog over the program
    // sequence; it is not modal, so the window goes on as it is open.
    SequenceSettingsDialog *openSequenceSettings();
    SequenceSettingsDialog *sequenceSettings() const;
    // Sends the dialog's set-sequence op as an edit.
    bool applySequenceSettings(const QJsonObject &op);
    // composer.draw: the composer's Draw when it has the focus.
    bool drawInComposer();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void followReview();
    void chooseFormat(int index);
    void addCommandItem(QMenu *menu, const QString &id, const QString &label, const char *name);
    QString projectLabel() const;

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    std::function<QDateTime()> m_clock;

    // Toolbar.
    QWidget *m_content = nullptr;
    ui::IconButton *m_appButton = nullptr;
    QMenu *m_appMenu = nullptr;
    QMenu *m_viewMenu = nullptr;
    QMenu *m_syncMenu = nullptr;
    QMenu *m_keymapMenu = nullptr;
    // The native menus, kept reachable where there is no menu bar.
    QHash<QString, QMenu *> m_menus;
    QList<QAction *> m_formatActions; // ⌥1-2: the registry's sequence.setFormat.*
    QPointer<SequenceSettingsDialog> m_sequenceSettings;
    // A set-sequence edit sent from the dialog, and the revision it went at.
    bool m_sequencePending = false;
    quint64 m_sequenceRevision = 0;
    // The set-sequence op that is waiting for kerfd's answer.
    QJsonObject m_sequenceOp;
    QTimer *m_titleTimer = nullptr;

    // The passing message and when it ends.
    QString m_message;
    QTimer *m_messageTimer = nullptr;

    // State.
    QJsonObject m_checkSummary; // the last finished check's summary
    QString m_checkStatus;      // "", running, done, failed
    double m_checkProgress = 0;
    QString m_checkMessage;
    QString m_connection = QStringLiteral("connecting");
    QHash<QString, QString> m_jobQuality; // render_queue jobId -> quality
    QString m_sentQuality;                // what the toolbar last exported
    // The renders running or queued, by jobId (the subtitle shows the one
    // started last, a running one before a queued one).
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
