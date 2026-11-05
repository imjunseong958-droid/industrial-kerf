// Tests for what the status bar used to say, now in the toolbar's subtitle, and
// connection recovery.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "presets_panel.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "source_monitor_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "components.h"
#include "editor_bus.h"
#include "review_model.h"
#include "wiring/toolbar_wiring.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"

#include <QAbstractItemView>
#include <QAccessible>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCheckBox>
#include <QClipboard>
#include <QComboBox>
#include <QCoreApplication>
#include <QDialog>
#include <QDialogButtonBox>
#include <QDir>
#include <QDoubleSpinBox>
#include <QElapsedTimer>
#include <QFile>
#include <QFileInfo>
#include <QFormLayout>
#include <QFrame>
#include <QGroupBox>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QImage>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMenu>
#include <QMenuBar>
#include <QMimeData>
#include <QPaintDevice>
#include <QPaintEngine>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QRadioButton>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QScreen>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QSettings>
#include <QSignalBlocker>
#include <QSignalSpy>
#include <QStandardPaths>
#include <QStatusBar>
#include <QStyleOptionViewItem>
#include <QTabWidget>
#include <QTemporaryDir>
#include <QTest>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextEdit>
#include <QTextFrame>
#include <QTextLayout>
#include <QToolBar>
#include <QToolButton>
#include <QToolTip>
#include <QWheelEvent>
#include <QWidget>


#include <algorithm>
#include <climits>
#include <cmath>
#include <functional>
#include <vector>

namespace kerf {

using namespace test;

namespace {

QAction *findAction(QMenu *menu, const QString &name) {
    for (QAction *action : menu->actions())
        if (action->objectName() == name) return action;
    return nullptr;
}

QStringList menuTexts(QMenu *menu) {
    QStringList items;
    for (const QAction *action : menu->actions()) items << (action->isSeparator() ? QStringLiteral("-") : action->text());
    return items;
}

QRect frameIn(QWidget *widget, QWidget *ancestor) { return QRect(widget->mapTo(ancestor, QPoint()), widget->size()); }

} // namespace

class StatusTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // The status bar's agent line has its place in the composer: what is
    // missing, and the registry's reason on its tip.
    void theComposerSaysWhenTheAgentCannotAsk() {
        const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("agent/connection")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_cloudRadio->setChecked(true);
        window.m_cloudAccess = QStringLiteral("signedOut");
        window.updateCommandStates();
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Comment, or set up Kerf in Settings to ask"));
        QCOMPARE(window.m_chatInput->toolTip(), QStringLiteral("Sign in to Kerf Cloud in Settings"));
        window.m_cloudAccess = QStringLiteral("ready");
        window.updateCommandStates();
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf, or leave a note"));
        // Own key, and none set (the environment's counts too).
        window.m_ownKeyRadio->setChecked(true);
        window.m_anthropicKey->setText(QStringLiteral("test-key"));
        window.updateCommandStates();
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf, or leave a note"));
        window.m_anthropicKey->clear();
        window.updateCommandStates();
        if (qEnvironmentVariableIsEmpty("ANTHROPIC_API_KEY")) {
            QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Comment, or set up Kerf in Settings to ask"));
            QCOMPARE(window.m_chatInput->toolTip(), QStringLiteral("Add your API key in Settings → Advanced"));
        }
        // Busy is not a setup problem.
        window.m_anthropicKey->setText(QStringLiteral("test-key"));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Ask Kerf, or leave a note"));
    }

    void disconnectedControlsAndRecovery() {
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Connect to comment or ask Kerf"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Connect to comment or ask Kerf"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_timeline->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_chatInput->placeholderText(), QStringLiteral("Connect to comment or ask Kerf"));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "ready"}});
        QVERIFY(window.m_connectionBanner->isHidden());
        QVERIFY(window.m_timeline->isEnabled());
        window.m_chatInput->setText(QStringLiteral("테스트 편집"));
        QVERIFY(window.m_sendButton->isEnabled());
        const auto conversation = window.m_chat->transcript();
        const int activityCount = window.m_activityList->count();
        window.showError(QStringLiteral("Render failed"));
        window.showError(QStringLiteral("Render failed"));
        QCOMPARE(window.m_chat->transcript(), conversation);
        QCOMPARE(window.m_activityList->count(), activityCount);
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Render failed"));
    }

    // There is no status bar; a window never makes one.
    void theWindowHasNoStatusBar() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        window.showStatus(QStringLiteral("Hello"), 100);
        QVERIFY(!window.findChild<QStatusBar *>());
        window.close();
    }

    // A sync's stage and its outcome reach the subtitle; a failure stays.
    void syncFollowsTheCollabStatus() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Reading the project's Git status"));
        window.applySnapshot(showcaseSnapshot(22, QStringLiteral("clean")));
        // Test builds have no runtime to send to; that is not this test's subject.
        window.m_errorBanner->hide();
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        const QString normal = wiring->subtitleText();
        QVERIFY(normal.contains(QStringLiteral("rev 22")));
        const QJsonObject status = sharedStatus();
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QCOMPARE(wiring->syncSummary(), QStringLiteral("Revision 22 · 1 commit to pull · 2 commits to push"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.commit")), QStringLiteral("Nothing to commit"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.publish")), QStringLiteral("The project is already published"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));

        // ≡ ▸ View ▸ Sync holds what the chip's menu held.
        wiring->buildAppMenu();
        QMenu *menu = wiring->m_syncMenu;
        QVERIFY(menu);
        QCOMPARE(menuTexts(menu), (QStringList{"Revision 22 · 1 commit to pull · 2 commits to push", "main", "color", "New Branch…", "-",
                                               "Sync Now", "Live Sync", "Commit…", "Open Pull Request…"}));
        QCOMPARE(ui::menuRole(menu->actions().first()), ui::MenuRole::Header);
        QVERIFY(findAction(menu, QStringLiteral("syncBranch.main"))->isChecked());
        // Sync now syncs; another branch switches to it.
        window.m_lastCommand = {};
        window.m_commands->action(QStringLiteral("collab.sync"))->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_sync"}}));
        wiring->buildSyncMenu(menu);
        findAction(menu, QStringLiteral("syncBranch.color"))->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_branch"}, {"name", "color"}, {"create", false}}));
        // Switch Branch still lists the other branches.
        window.buildBranchMenu(window.m_branchMenu);
        QCOMPARE(window.m_branchMenu->actions().size(), 1);

        // While a sync runs the subtitle says what it is doing, and Sync Now waits.
        window.m_errorBanner->hide();
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}});
        QCOMPARE(wiring->subtitleText(), QStringLiteral("Pulling…"));
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Pulling…"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("A sync is running"));

        // A finished sync lists what came in and what kept yours, then reads
        // the preset library again; its summary shows for a moment.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{QJsonObject{{"commit", "a1b2c3d"}, {"author", "jun"}, {"subject", "Trimmed c3"}}}},
            {"fastForward", false}, {"pushed", true},
            {"conflicts", QJsonArray{QJsonObject{{"path", "project.kerf"}, {"what", "c3 duration"}, {"kept", "ours"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{"Trimmed c3"}}}}});
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("jun · Trimmed c3")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept yours · project.kerf: c3 duration")));
        QCOMPARE(wiring->message(), QStringLiteral("Synced: 1 commit in, pushed, 1 change kept yours"));
        QCOMPARE(window.m_projectSubtitle->text(), wiring->message());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("preset_library"));
        wiring->showMessage(QString(), 0);
        window.m_errorBanner->hide();
        QCOMPARE(wiring->subtitleText(), normal);

        // A failure goes to the error banner and stays in the subtitle, in red.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "the remote rejected the push"}});
        QVERIFY(!window.m_errorBanner->isHidden());
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Sync failed: the remote rejected the push"));
        QCOMPARE(wiring->subtitleText(), QStringLiteral("Sync failed: the remote rejected the push"));
        QVERIFY(window.m_projectSubtitle->property("problem").toBool());
        wiring->buildSyncMenu(menu);
        QCOMPARE(findAction(menu, QStringLiteral("syncProblem"))->text(), QStringLiteral("Sync failed: the remote rejected the push"));
        // Dismissed, the failed sync itself still stands until a sync goes well.
        window.m_errorBanner->hide();
        QCOMPARE(wiring->standingProblem(), QStringLiteral("Sync failed: the remote rejected the push"));
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{{"incoming", QJsonArray{}}, {"pushed", false}}}});
        window.m_errorBanner->hide();
        QVERIFY(wiring->standingProblem().isEmpty());

        QJsonObject conflicted = status;
        conflicted.insert("conflicted", true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", conflicted}});
        QCOMPARE(wiring->standingProblem(), QStringLiteral("A merge is unfinished; finish it with git"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Finish the merge in progress with git first"));

        const QJsonObject local{{"repository", false}, {"branches", QJsonArray{}}, {"gh", false}, {"live", false}, {"busy", false}};
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", local}});
        QVERIFY(wiring->standingProblem().isEmpty());
        QVERIFY(window.m_commands->reason(QStringLiteral("collab.sync")).startsWith(QStringLiteral("Share the project with Git first")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.publish")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.pullRequest")), window.m_commands->reason(QStringLiteral("collab.sync")));
        wiring->buildSyncMenu(menu);
        QCOMPARE(menu->actions().last(), window.m_commands->action(QStringLiteral("collab.publish")));
    }

    // A passing message shows in the subtitle for its timeout, then the normal
    // line again; offline and an error stay until they clear.
    void messagesShowInTheSubtitleForTheirTimeout() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QLabel *subtitle = window.m_projectSubtitle;
        // Before the first answer it says so; offline stays.
        QCOMPARE(subtitle->text(), QStringLiteral("Connecting…"));
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(subtitle->text(), QStringLiteral("Offline"));
        QVERIFY(subtitle->property("problem").toBool());
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(22, QStringLiteral("clean")));
        window.m_errorBanner->hide();
        const QString normal = subtitle->text();
        QVERIFY(normal.contains(QStringLiteral("rev 22")));
        QVERIFY(!subtitle->property("problem").toBool());
        // For its timeout.
        window.showStatus(QStringLiteral("Saved preset looks/dusk.preset"), 60);
        QCOMPARE(subtitle->text(), QStringLiteral("Saved preset looks/dusk.preset"));
        QTRY_COMPARE(subtitle->text(), normal);
        // A timeout of 0 keeps it until the next one.
        window.showStatus(QStringLiteral("Held"), 0);
        QTest::qWait(30);
        QCOMPARE(subtitle->text(), QStringLiteral("Held"));
        window.showStatus(QString(), 0);
        QCOMPARE(subtitle->text(), normal);
        // A refused command's reason.
        window.showStatus(QStringLiteral("A roll needs a clip on the other side of the cut"), 4000);
        QCOMPARE(wiring->message(), QStringLiteral("A roll needs a clip on the other side of the cut"));
        window.showStatus(QString(), 0);
        // An error stays until it is dismissed; a message passes over it.
        window.showError(QStringLiteral("Render failed"));
        QCOMPARE(subtitle->text(), QStringLiteral("Render failed"));
        QVERIFY(subtitle->property("problem").toBool());
        window.showStatus(QStringLiteral("Preview ready"), 40);
        QCOMPARE(subtitle->text(), QStringLiteral("Preview ready"));
        QVERIFY(!subtitle->property("problem").toBool());
        QTRY_COMPARE(subtitle->text(), QStringLiteral("Render failed"));
        window.m_errorBanner->findChildren<QPushButton *>().first()->click();
        QCOMPARE(subtitle->text(), normal);
        window.close();
    }

    void subtitleNamesTheGuest() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(22, QStringLiteral("clean")));
        window.m_errorBanner->hide();
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QVERIFY(!wiring->subtitleText().contains(QStringLiteral("Viewing as")));
        window.m_bus->setViewAs(QStringLiteral("mina@example.com"));
        QVERIFY(wiring->subtitleText().endsWith(QStringLiteral(" · Viewing as mina@example.com")));
        QCOMPARE(window.m_projectSubtitle->text(), wiring->subtitleText());

        ReviewModel review;
        review.setIdentity(QJsonObject{{"name", "Jun"}, {"email", "jun@example.com"}});
        review.setComments(QJsonArray{QJsonObject{{"id", "c-1"}, {"parent", QJsonValue()}, {"author", QJsonObject{{"kind", "person"},
            {"name", "Mina Park"}, {"id", "mina@example.com"}}}, {"at", 1.0}, {"body", "Tighter"}, {"created", "2026-11-04T10:00:00Z"}, {"revision", 1}}});
        review.setViewAs(QStringLiteral("mina@example.com"));
        window.m_bus->setReview(&review);
        QVERIFY(wiring->subtitleText().endsWith(QStringLiteral(" · Viewing as Mina Park")));
        review.setViewAs(QString());
        QVERIFY(!wiring->subtitleText().contains(QStringLiteral("Viewing as")));
        window.m_bus->setReview(nullptr);
        window.m_bus->setViewAs(QString());
        QVERIFY(!wiring->subtitleText().contains(QStringLiteral("Viewing as")));
        window.close();
    }

    void subtitleShowsWhatIsRendering() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(22, QStringLiteral("clean")));
        window.m_errorBanner->hide();
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QVERIFY(wiring->runningText().isEmpty());
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "review"}, {"jobId", "j1"},
                                       {"name", "Draft.mp4"}, {"progress", 0.38}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Draft export · 38%"));
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Draft export · 38%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"jobId", "j1"}, {"progress", 1.0}});
        QVERIFY(wiring->runningText().isEmpty());
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Export completed"));
        wiring->showMessage(QString(), 0);
        // A master shows too, now the status bar's Deliver exception is gone.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "master"}, {"jobId", "m"}, {"progress", 0.2}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Master export · 20%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"jobId", "m"}});
        wiring->showMessage(QString(), 0);
        // Jobs known from the render queue; a running one before a queued one.
        window.handleEvent(QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{
            QJsonObject{{"jobId", "j2"}, {"name", "Master.mov"}, {"quality", "master"}, {"status", "queued"}, {"progress", 0.0}},
            QJsonObject{{"jobId", "j3"}, {"name", "Draft 2.mp4"}, {"quality", "review"}, {"status", "running"}, {"progress", 0.1}}}}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Draft export · 10%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"jobId", "j3"}, {"progress", 0.64}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Draft export · 64%"));
        // A second draft: the one started last is shown; when it ends the
        // first is shown again.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "full"}, {"jobId", "j4"},
                                       {"name", "Vertical.mp4"}, {"progress", 0.25}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Vertical.mp4 · 25%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"jobId", "j4"}, {"progress", 0.5}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Vertical.mp4 · 50%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"jobId", "j4"}, {"progress", 1.0}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Draft export · 64%"));
        window.handleEvent(QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{}}});
        QVERIFY(wiring->runningText().isEmpty());
        // Paperwork is no render.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"kind", "paperwork"}, {"jobId", "p"}});
        QVERIFY(wiring->runningText().isEmpty());
        // A preview render.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}, {"progress", 0.5}});
        QCOMPARE(wiring->runningText(), QStringLiteral("Rendering preview · 50%"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(wiring->runningText().isEmpty());
        window.close();
    }

};

} // namespace kerf

KERF_TEST_CLASS(kerf::StatusTest)

#include "status_test.moc"
