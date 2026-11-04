// Tests for the status bar (mockup-spec 9, 14) and connection recovery.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "device_frame.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "player_overlay.h"
#include "presets_panel.h"
#include "program_level.h"
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
#include "workspace.h"

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

#include <DockAreaTabBar.h>
#include <DockAreaTitleBar.h>
#include <DockAreaWidget.h>
#include <DockManager.h>
#include <DockSplitter.h>
#include <DockWidget.h>
#include <DockWidgetTab.h>
#include <ElidingLabel.h>

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

    // The sync chip: the branch glyph and "main ↑2 ↓1" in mono, its tip the
    // revision and what there is to pull and push; a click opens the Sync
    // menu (the branches, New branch…, Sync now, Live sync, Commit…, Open
    // pull request…). While a sync runs it says what it does; a failure or
    // an unfinished merge reads in red.
    void syncChipFollowsTheCollabStatus() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        auto *chip = qobject_cast<StatusChip *>(window.m_syncChip);
        QVERIFY(chip);
        QVERIFY(chip->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Reading the project's Git status"));
        window.applySnapshot(showcaseSnapshot(22, QStringLiteral("clean")));
        const QJsonObject status = sharedStatus();
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QVERIFY(!chip->isHidden());
        QCOMPARE(chip->text(), QStringLiteral("main ↑2 ↓1"));
        QCOMPARE(chip->iconName(), QStringLiteral("g-branch"));
        QCOMPARE(chip->height(), StatusChip::kHeight);
        QCOMPARE(chip->property("state").toString(), QStringLiteral("idle"));
        QCOMPARE(ui::tipName(chip), QStringLiteral("Revision 22 · 1 commit to pull · 2 commits to push"));
        QVERIFY(chip->accessibleDescription().contains(QStringLiteral("Syncs with origin/main")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.commit")), QStringLiteral("Nothing to commit"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.publish")), QStringLiteral("The project is already published"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));

        // A click opens the Sync menu.
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QMenu *menu = wiring->m_syncMenu;
        QTest::mouseClick(chip, Qt::LeftButton);
        QTRY_VERIFY(menu->isVisible());
        QCOMPARE(menuTexts(menu), (QStringList{"Revision 22 · 1 commit to pull · 2 commits to push", "main", "color", "New Branch…", "-",
                                               "Sync Now", "Live Sync", "Commit…", "Open Pull Request…"}));
        QCOMPARE(ui::menuRole(menu->actions().first()), ui::MenuRole::Header);
        QVERIFY(findAction(menu, QStringLiteral("syncBranch.main"))->isChecked());
        menu->hide();
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

        // While a sync runs the chip says what it is doing, and Sync Now waits.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}});
        QCOMPARE(chip->text(), QStringLiteral("Pulling…"));
        QCOMPARE(chip->property("state").toString(), QStringLiteral("syncing"));
        QCOMPARE(chip->ink(), tokens::color(tokens::accentInk));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("A sync is running"));

        // A finished sync lists what came in and what kept yours, then
        // reads the preset library again.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{QJsonObject{{"commit", "a1b2c3d"}, {"author", "jun"}, {"subject", "Trimmed c3"}}}},
            {"fastForward", false}, {"pushed", true},
            {"conflicts", QJsonArray{QJsonObject{{"path", "project.kerf"}, {"what", "c3 duration"}, {"kept", "ours"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{"Trimmed c3"}}}}});
        QCOMPARE(chip->property("state").toString(), QStringLiteral("idle"));
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("jun · Trimmed c3")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept yours · project.kerf: c3 duration")));
        QVERIFY(chip->accessibleDescription().contains(QStringLiteral("jun · Trimmed c3")));
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Synced: 1 commit in, pushed, 1 change kept yours"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("preset_library"));
        // The status bar's message shows in the row, in place of its left items.
        QCOMPARE(wiring->m_row->message(), window.statusBar()->currentMessage());
        QVERIFY(wiring->m_row->leftItems()->isHidden());
        window.statusBar()->clearMessage();
        QVERIFY(!wiring->m_row->leftItems()->isHidden());

        // A failure goes to the error banner, and the chip reads in red.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "the remote rejected the push"}});
        QCOMPARE(chip->property("state").toString(), QStringLiteral("failed"));
        QVERIFY(chip->text().endsWith(QStringLiteral("sync failed")));
        QCOMPARE(chip->ink(), tokens::color(tokens::neg));
        QVERIFY(!window.m_errorBanner->isHidden());
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Sync failed: the remote rejected the push"));
        wiring->buildSyncMenu(menu);
        QCOMPARE(findAction(menu, QStringLiteral("syncProblem"))->text(), QStringLiteral("Sync failed: the remote rejected the push"));

        QJsonObject conflicted = status;
        conflicted.insert("conflicted", true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", conflicted}});
        QCOMPARE(chip->property("state").toString(), QStringLiteral("conflict"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Finish the merge in progress with git first"));

        const QJsonObject local{{"repository", false}, {"branches", QJsonArray{}}, {"gh", false}, {"live", false}, {"busy", false}};
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", local}});
        QCOMPARE(chip->text(), QStringLiteral("Not shared"));
        QCOMPARE(chip->property("state").toString(), QStringLiteral("local"));
        QVERIFY(window.m_commands->reason(QStringLiteral("collab.sync")).startsWith(QStringLiteral("Share the project with Git first")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.publish")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.pullRequest")), window.m_commands->reason(QStringLiteral("collab.sync")));
        wiring->buildSyncMenu(menu);
        QCOMPARE(menu->actions().last(), window.m_commands->action(QStringLiteral("collab.publish")));

        // Every state's words read at 4.5:1 on the chrome.
        QJsonObject live = status;
        live.insert("live", true);
        const QList<QJsonObject> events{
            QJsonObject{{"type", "collab"}, {"status", status}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{{"incoming", QJsonArray{}}, {"pushed", false}}}},
            QJsonObject{{"type", "collab"}, {"status", live}},
            QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "offline"}},
            QJsonObject{{"type", "collab"}, {"status", conflicted}},
            QJsonObject{{"type", "collab"}, {"status", local}}};
        QSet<QString> states;
        for (const QJsonObject &event : events) {
            window.handleEvent(event);
            const QString state = chip->property("state").toString();
            states.insert(state);
            const qreal ratio = contrastRatio(chip->ink(), tokens::color(tokens::chrome));
            QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1: %2").arg(state).arg(ratio, 0, 'f', 2)));
        }
        QCOMPARE(states, (QSet<QString>{"idle", "syncing", "live", "failed", "conflict", "local"}));
    }

    // The status bar is the design's row: 24 tall on the chrome with its
    // hairline on top, pad 16 and 12; the selection names the clip, its
    // track and its range; the rate and the frames dropped come from clock.
    void statusRowShowsTheSelectionAndDroppedFrames() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QStatusBar *bar = window.statusBar();
        QCOMPARE(bar->height(), 24);
        QCOMPARE(wiring->m_row->geometry(), bar->rect());
        // Before the project answers: no rate to name.
        QVERIFY(wiring->m_rate->isHidden());
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        QVERIFY(!wiring->m_rate->isHidden());
        QCOMPARE(wiring->m_rate->text(), QStringLiteral("24p"));
        QCOMPARE(wiring->m_rate->color(), tokens::color(tokens::ink3));
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 1.0}, {"duration", 18.0}, {"dropped", 3}});
        QCOMPARE(wiring->m_rate->text(), QStringLiteral("24p · 3 dropped"));
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 2.0}, {"duration", 18.0}, {"dropped", 0}});
        QCOMPARE(wiring->m_rate->text(), QStringLiteral("24p · 0 dropped"));
        QCOMPARE(frameRateLabel(23.976), QStringLiteral("23.98p"));
        QCOMPARE(frameRateLabel(25.0), QStringLiteral("25p"));
        QCOMPARE(frameRateLabel(29.97), QStringLiteral("29.97p"));

        // The selection: clip · track · in–out (the range in mono).
        QVERIFY(wiring->m_selection->isHidden());
        window.selectClip(QStringLiteral("v1b"));
        QVERIFY(!wiring->m_selection->isHidden());
        QVERIFY2(wiring->m_selection->text().endsWith(QStringLiteral(" · V1 · ")), qPrintable(wiring->m_selection->text()));
        QCOMPARE(wiring->m_selection->monoTail(), QStringLiteral("00:00:06:00–00:00:11:00"));
        const QString name = window.clipObject(QStringLiteral("v1b")).value("name").toString();
        QVERIFY(!name.isEmpty());
        QCOMPARE(wiring->m_selection->text(), QStringLiteral("%1 · V1 · ").arg(name));
        window.extendClipSelection(QStringLiteral("v1c"), false);
        QCOMPARE(wiring->m_selection->text(), QStringLiteral("2 clips · "));
        QCOMPARE(wiring->m_selection->monoTail(), QStringLiteral("00:00:06:00–00:00:18:00"));
        // A named track names itself.
        const auto [text, span] = ToolbarWiring::selectionText(window.m_bus->timeline(), {QStringLiteral("a2")}, 24);
        QVERIFY(text.endsWith(QStringLiteral(" · Music · ")));
        QCOMPARE(span, QStringLiteral("00:00:00:00–00:00:18:00"));
        window.selectClip(QString());
        QVERIFY(wiring->m_selection->isHidden());

        // The row's ends: 16 in on the left, 12 on the right.
        window.selectClip(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QCOMPARE(frameIn(wiring->m_selection, bar).left(), 16);
        QCOMPARE(frameIn(wiring->m_agent, bar).right() + 1, bar->width() - 12);
        QCOMPARE(frameIn(wiring->m_rate, bar).left() - frameIn(wiring->m_selection, bar).right() - 1, 16);
        // Chrome, with the hairline on top.
        const QImage image = bar->grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        QCOMPARE(image.pixelColor(QPoint(bar->width() / 2, 12) * ratio).rgb(), tokens::chrome);
        QVERIFY(image.pixelColor(QPoint(bar->width() / 2, 0) * ratio).lightness() > tokens::color(tokens::chrome).lightness());
        // The older labels are out of sight.
        for (const QLabel *retired : window.findChildren<QLabel *>(QStringLiteral("statusSelection"))) QVERIFY(!retired->isVisible());
        QVERIFY(!window.m_exportProgress->isVisible());
    }

    // "Viewing as Mina Park" in guest mode, named by the review model when
    // there is one.
    void statusNamesTheGuest() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QVERIFY(wiring->m_guest->isHidden());
        window.m_bus->setViewAs(QStringLiteral("mina@example.com"));
        QVERIFY(!wiring->m_guest->isHidden());
        QCOMPARE(wiring->m_guest->text(), QStringLiteral("Viewing as mina@example.com"));
        QCOMPARE(wiring->m_guest->color(), tokens::color(tokens::ink));

        ReviewModel review;
        review.setIdentity(QJsonObject{{"name", "Jun"}, {"email", "jun@example.com"}});
        review.setComments(QJsonArray{QJsonObject{{"id", "c-1"}, {"parent", QJsonValue()}, {"author", QJsonObject{{"kind", "person"},
            {"name", "Mina Park"}, {"id", "mina@example.com"}}}, {"at", 1.0}, {"body", "Tighter"}, {"created", "2026-11-04T10:00:00Z"}}});
        review.setViewAs(QStringLiteral("mina@example.com"));
        window.m_bus->setReview(&review);
        QCOMPARE(wiring->m_guest->text(), QStringLiteral("Viewing as Mina Park"));
        review.setViewAs(QString());
        QVERIFY(wiring->m_guest->isHidden());
        window.m_bus->setReview(nullptr);
        window.m_bus->setViewAs(QString());
        QVERIFY(wiring->m_guest->isHidden());
    }

    // Draft export progress: "Draft export", the 64 pt bar and the
    // percentage for a running non-master job (the one started last of
    // several); a master's progress stays in the Deliver queue and leaves a
    // running draft in view; a preview render shows the same way.
    void statusShowsDraftExportProgress() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        StatusProgress *progress = wiring->m_progress;
        QVERIFY(progress->isHidden());
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "review"}, {"jobId", "j1"},
                                       {"name", "Draft.mp4"}, {"progress", 0.38}});
        QVERIFY(!progress->isHidden());
        QCOMPARE(progress->label(), QStringLiteral("Draft export"));
        QCOMPARE(progress->percentText(), QStringLiteral("38%"));
        QCOMPARE(progress->barRect().width(), qreal(StatusProgress::kBarWidth));
        QCOMPARE(progress->barRect().height(), 4.0);
        QCOMPARE(ui::tipName(progress), QStringLiteral("Draft export · H.264 review copy"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"jobId", "j1"}, {"progress", 1.0}});
        QVERIFY(progress->isHidden());
        // A master: not here.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "master"}, {"progress", 0.2}});
        QVERIFY(progress->isHidden());
        // A job known from the render queue.
        window.handleEvent(QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{
            QJsonObject{{"jobId", "j2"}, {"name", "Master.mov"}, {"quality", "master"}, {"status", "running"}, {"progress", 0.5}},
            QJsonObject{{"jobId", "j3"}, {"name", "Draft 2.mp4"}, {"quality", "review"}, {"status", "running"}, {"progress", 0.1}}}}});
        QVERIFY(!progress->isHidden());
        QCOMPARE(progress->percentText(), QStringLiteral("10%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"jobId", "j3"}, {"progress", 0.64}});
        QCOMPARE(progress->percentText(), QStringLiteral("64%"));
        // The master's own progress leaves the running draft in view.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"jobId", "j2"}, {"progress", 0.7}});
        QVERIFY(!progress->isHidden());
        QCOMPARE(progress->percentText(), QStringLiteral("64%"));
        // A second draft: the one started last is shown; when it ends the
        // first is shown again.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"quality", "full"}, {"jobId", "j4"},
                                       {"name", "Vertical.mp4"}, {"progress", 0.25}});
        QCOMPARE(progress->label(), QStringLiteral("Vertical.mp4"));
        QCOMPARE(progress->percentText(), QStringLiteral("25%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "running"}, {"jobId", "j4"}, {"progress", 0.5}});
        QCOMPARE(progress->label(), QStringLiteral("Vertical.mp4"));
        QCOMPARE(progress->percentText(), QStringLiteral("50%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"jobId", "j4"}, {"progress", 1.0}});
        QVERIFY(!progress->isHidden());
        QCOMPARE(progress->label(), QStringLiteral("Draft export"));
        QCOMPARE(progress->percentText(), QStringLiteral("64%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "failed"}, {"jobId", "j3"}});
        QVERIFY(progress->isHidden());
        // Paperwork is no render.
        window.handleEvent(QJsonObject{{"type", "export"}, {"status", "completed"}, {"kind", "paperwork"}, {"path", "/tmp/c.csv"}});
        QVERIFY(progress->isHidden());
        // A preview render.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}, {"progress", 0.5}});
        QVERIFY(!progress->isHidden());
        QCOMPARE(progress->label(), QStringLiteral("Rendering preview"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(progress->isHidden());
    }

    // The keymap chip names the keymap in use ("Premiere keys") and opens
    // the Keyboard menu: the keymaps of kerf_commands_catalog, then
    // Customise… (the registry's keyboard.customise).
    void statusKeymapChipChoosesTheKeymap() {
        const QVariant before = QSettings().value(QStringLiteral("commands/keymap"));
        {
            MainWindow window{QString(), QString()};
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            ToolbarWiring *wiring = window.m_toolbarWiring.get();
            StatusChip *chip = wiring->m_keymap;
            window.m_commands->setKeymap(QStringLiteral("premiere"));
            QCOMPARE(chip->text(), QStringLiteral("Premiere keys"));
            QCOMPARE(ui::tipName(chip), QStringLiteral("Keyboard shortcuts"));
            QCOMPARE(ToolbarWiring::keymapChipText(QStringLiteral("Final Cut Pro")), QStringLiteral("Final Cut keys"));
            QCOMPARE(ToolbarWiring::keymapChipText(QStringLiteral("Kerf")), QStringLiteral("Kerf keys"));

            QTest::mouseClick(chip, Qt::LeftButton);
            QMenu *menu = wiring->m_keymapMenu;
            QTRY_VERIFY(menu->isVisible());
            QStringList expected;
            for (const QJsonValue &value : window.m_commands->keymaps()) expected << value.toObject().value("label").toString();
            expected << QStringLiteral("-") << window.m_commands->action(QStringLiteral("keyboard.customise"))->text();
            QCOMPARE(menuTexts(menu), expected);
            QVERIFY(findAction(menu, QStringLiteral("keymapChoice.premiere"))->isChecked());
            QCOMPARE(menu->actions().last(), window.m_commands->action(QStringLiteral("keyboard.customise")));
            menu->hide();
            findAction(menu, QStringLiteral("keymapChoice.fcp"))->trigger();
            QCOMPARE(window.m_commands->keymap(), QStringLiteral("fcp"));
            QCOMPARE(QSettings().value(QStringLiteral("commands/keymap")).toString(), QStringLiteral("fcp"));
            QCOMPARE(chip->text(), QStringLiteral("Final Cut keys"));
            // Edit ▸ Keyboard Shortcuts checks the same.
            for (QAction *action : window.findChildren<QAction *>())
                if (action->objectName() == QStringLiteral("keymap.fcp")) QVERIFY(action->isChecked());
            // The toolbar's tips follow the keymap.
            QCOMPARE(ui::tipShortcut(window.m_exportButton->mainPart()),
                     window.m_commands->action(QStringLiteral("export.master"))->shortcut().toString(QKeySequence::NativeText));
        }
        if (before.isValid()) QSettings().setValue(QStringLiteral("commands/keymap"), before);
        else QSettings().remove(QStringLiteral("commands/keymap"));
    }

    // "Agent: own key", the dot and the connection: green and Connected,
    // amber while connecting, red and Offline; no key says so.
    void statusAgentShowsAccessAndConnection() {
        MainWindow window{QString(), QString()};
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        AgentStatus *agent = wiring->m_agent;
        QCOMPARE(agent->connectionText(), QStringLiteral("Connecting…"));
        QCOMPARE(agent->dotColor(), tokens::color(tokens::warn));
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(agent->connectionText(), QStringLiteral("Connected"));
        QCOMPARE(agent->dotColor(), tokens::color(tokens::pos));
        window.setConnection(QStringLiteral("disconnected"), QStringLiteral("gone"));
        QCOMPARE(agent->connectionText(), QStringLiteral("Offline"));
        QCOMPARE(agent->dotColor(), tokens::color(tokens::neg));
        QCOMPARE(agent->dotRect().size(), QSizeF(6, 6));
        // The access as Settings has it.
        window.m_ownKeyRadio->setChecked(true);
        const QString expected = window.aiAccess() == QStringLiteral("noKey") ? QStringLiteral("Agent: no key") : QStringLiteral("Agent: own key");
        QCOMPARE(agent->agentText(), expected);
        window.m_anthropicKey->setText(QStringLiteral("sk-test-value"));
        if (window.activeProviderId() == QStringLiteral("anthropic")) QCOMPARE(agent->agentText(), QStringLiteral("Agent: own key"));
        window.m_anthropicKey->clear();
        window.m_cloudRadio->setChecked(true);
        QVERIFY(agent->agentText().startsWith(QStringLiteral("Agent: Kerf Cloud")));
        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "ready"}, {"account", QJsonObject{{"email", "jun@example.com"},
                                                                                                    {"plan", "Pro"}}}});
        QCOMPARE(agent->agentText(), QStringLiteral("Agent: Kerf Cloud"));
        window.handleEvent(QJsonObject{{"type", "cloud"}, {"access", "signedOut"}});
        QCOMPARE(agent->agentText(), QStringLiteral("Agent: Kerf Cloud · signed out"));
        QSettings().remove(QStringLiteral("agent/connection"));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StatusTest)

#include "status_test.moc"
