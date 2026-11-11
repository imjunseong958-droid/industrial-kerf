#pragma once

// The toolbar region's wiring. A friend of MainWindow that holds the region's
// own state, so a region changes the window's behaviour only here and in
// main_window_toolbar.cpp and main_window_status.cpp.
//
// What is left of the toolbar (40 tall), in the room's own colour: the window
// buttons' room and the sidebar's top, the sidebar's button, the project and
// what is going on in it, Export once there is a cut, and the panel's button.
// Everything else opens from the account at the sidebar's foot (or, with the
// sidebar away, from a ⋯ button here). Engine paths:
//   the menu            Settings, Account, the guide, Media and Import, then
//                       View ▸ the panes, the mask tools, view.annotations,
//                       view.safeZones, viewer.zoom, the keymap choice and the
//                       collab.* commands; Sign Out when signed in
//   title               snapshot timeline.name + timeline.sequences[].name;
//                       its tip is revisionTime and revision
//   status              beside the title, only when there is something to
//                       say: a passing message for its timeout
//                       (MainWindow::showStatus), else offline or an error,
//                       which stay, else the sync's stage or a render's
//                       progress (export, render_queue, preview), else whose
//                       view a guest's is
//   format ⌥1-2         the registry's sequence.setFormat.169/.916 ->
//                       sequence.setFormat {width, height} (edit set-sequence)
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
    // The row's leading padding.
    static constexpr int kLeadingPad = 16;
    ToolbarWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // ---- built by MainWindow::setupToolbar
    void buildToolbar(const QHash<QString, QMenu *> &menus);
    // The menu the account opens, built each time it opens; its View submenu
    // with it.
    QMenu *appMenu() const { return m_appMenu; }
    void buildAppMenu();
    // Over the account at the sidebar's foot, or under the bar's own button.
    void popupAppMenu(QWidget *anchor);
    // The bar's own way to that menu, shown while the sidebar is away.
    ui::IconButton *appMenuButton() const { return m_appButton; }

    // ---- the title, centred on the window
    // "Atlas · 30 s spot v7": the project, then the program sequence's name.
    QString titleText() const;
    // The line beside the title: a passing message, else offline or an error,
    // else the sync's stage or a render's progress, else a guest's view; empty
    // when there is nothing to say.
    QString subtitleText() const;
    // "Viewing as Ana" while a guest's view is on.
    QString guestNote() const;
    // "Edited 2 min ago · rev 22" (just "rev 22" without a revision time),
    // with "Viewing as Ana" while a guest's view is on: the title's tip.
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

    // ---- the window buttons' room, and the sidebar's
    // Where the first control starts: 16 in, past the macOS window buttons
    // (52 + 16 reserved), or 16 without them.
    static int firstControlX(int windowButtonsRight, bool fullScreen);
    // The width of the room before the first control, for one that starts at
    // `firstControl` and a sidebar that ends at `sidebarEdge` (0: none shows).
    static int leadingRoom(int firstControl, int sidebarEdge);
    // Where the window buttons let the first control start (MainWindow::fitTitleBar).
    void setFirstControl(int x);
    // Where the sidebar ends under the bar now, also while it moves; 0 for none.
    int sidebarEdge() const;
    void layoutLeading();

    // ---- Export, there once the program has a clip
    bool hasCut() const;
    void followCut();

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
    // while a blocking note is open: it is not sent, and holdForNotes says
    // why.
    bool sendExport(const QString &quality);
    // Says which note holds a master back and where, and shows the blocking
    // notes in Review to resolve; `refusal` is kerfd's, when it refused.
    void holdForNotes(const QString &refusal = QString());
    // The Export menu, built each time it opens.
    void buildExportMenu();

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
    // Registers sequence.setFormat.* and composer.draw, and the bin's.
    void installCommands();
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
    QWidget *m_titleBox = nullptr;
    int m_firstControl = kLeadingPad;
    ui::IconButton *m_appButton = nullptr;
    QMenu *m_appMenu = nullptr;
    QMenu *m_viewMenu = nullptr;
    QMenu *m_syncMenu = nullptr;
    QMenu *m_keymapMenu = nullptr;
    // The native menus, kept reachable where there is no menu bar.
    QHash<QString, QMenu *> m_menus;
    QList<QAction *> m_formatActions; // ⌥1-2: the registry's sequence.setFormat.*
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
