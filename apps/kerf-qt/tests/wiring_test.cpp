// Tests for how the editor turns actions into bridge requests.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "bridge.h"
#include "editor_bus.h"
#include "inspector_column.h"
#include "chat_panel.h"
#include "stage_rail.h"
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

class WiringTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void timelineEditsAreTypedOperationsOnTheModel() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"width", 1920}, {"height", 1080},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 1.0},
                            {"duration", 2.0}, {"startFrame", 24}, {"durationFrames", 48},
                            {"in", 5.0}, {"out", 7.0}, {"speed", 1.0}}}}}}}}}});
        auto op = [&window] {
            const QJsonObject command = window.m_lastCommand;
            if (command.value("type").toString() != QStringLiteral("edit")) return QJsonObject{};
            if (command.value("revision").toInt() != 3) return QJsonObject{};
            return command.value("ops").toArray().first().toObject();
        };
        window.moveClip(QStringLiteral("c1"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("move"));
        QCOMPARE(op().value("frames").toInt(), 12);
        window.trimClip(QStringLiteral("c1"), false, 66);
        QCOMPARE(op().value("op").toString(), QStringLiteral("trim"));
        QCOMPARE(op().value("edge").toString(), QStringLiteral("out"));
        QCOMPARE(op().value("frames").toInt(), -6);
        window.trimClip(QStringLiteral("c1"), true, 30);
        QCOMPARE(op().value("edge").toString(), QStringLiteral("in"));
        QCOMPARE(op().value("frames").toInt(), 6);
        window.selectClip(QStringLiteral("c1"));
        window.m_playhead = 2.0;
        window.splitClip();
        QCOMPARE(op().value("op").toString(), QStringLiteral("split"));
        QCOMPARE(op().value("at").toInt(), 48);
        window.deleteSelected();
        QCOMPARE(op().value("clips").toArray().first().toString(), QStringLiteral("c1"));
        const QJsonObject crop = MainWindow::cropProps(QJsonObject{{"x", 0.25}, {"y", 0.0}, {"width", 0.5}, {"height", 1.0}});
        QCOMPARE(crop.value("crop_l").toDouble(), 0.25);
        QCOMPARE(crop.value("crop_r").toDouble(), 0.25);
        QVERIFY(MainWindow::cropProps(QJsonObject{}).value("crop_l").isNull());
        const QJsonObject box = MainWindow::cropBox(QJsonObject{{"props", crop}});
        QCOMPARE(box.value("x").toDouble(), 0.25);
        QCOMPARE(box.value("width").toDouble(), 0.5);
        QVERIFY(MainWindow::cropBox(QJsonObject{}).isEmpty());
    }

    void commandsBuildMenusShortcutsAndRequests() {
        MainWindow window{QString(), QString()};
        QStringList menus;
        for (const QAction *menu : window.menuBar()->actions()) menus.append(menu->text());
        QCOMPARE(menus, (QStringList{"File", "Edit", "Clip", "Sequence", "Markers", "Playback", "View", "Collaborate", "Window"}));
        QVERIFY(window.findChild<QAction *>(QStringLiteral("keymap.premiere")));
        // Projects open from File; Import Media keeps Ctrl+O.
        QVERIFY(window.findChild<QMenu *>(QStringLiteral("openRecentMenu")));
        QCOMPARE(window.m_commands->action(QStringLiteral("file.openProject"))->shortcut(),
                 QKeySequence::fromString(QStringLiteral("Ctrl+Shift+O"), QKeySequence::PortableText));
        QCOMPARE(window.m_commands->action(QStringLiteral("file.import"))->shortcut(),
                 QKeySequence::fromString(QStringLiteral("Ctrl+O"), QKeySequence::PortableText));
        QVERIFY(window.m_commands->action(QStringLiteral("collab.switchBranch"))->menu());

        QAction *split = window.m_commands->action(QStringLiteral("clip.split"));
        QAction *undo = window.m_commands->action(QStringLiteral("edit.undo"));
        QVERIFY(split && undo);
        // Unavailable commands say why.
        QVERIFY(!split->isEnabled());
        QVERIFY(split->toolTip().contains(QStringLiteral("Waiting for the project connection")));

        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 4}, {"canUndo", true}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"timelineStart", 1.0}, {"duration", 2.0},
                            {"startFrame", 24}, {"durationFrames", 48}}}}}}}}}});
        QCOMPARE(window.m_commands->reason(QStringLiteral("clip.split")), QStringLiteral("Select a clip first"));
        QVERIFY(undo->isEnabled());
        undo->trigger();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("undo"));
        QCOMPARE(window.m_lastCommand.value("revision").toInt(), 3);

        window.selectClip(QStringLiteral("c1"));
        window.queueSeek(2.0);
        QVERIFY(split->isEnabled());
        window.m_lastCommand = {};
        window.show();
        window.m_timeline->setFocus();
        QTRY_VERIFY(window.m_timeline->hasFocus());
        QTest::keyClick(window.m_timeline, Qt::Key_S);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("split"));
        QCOMPARE(op.value("at").toInt(), 48);

        // Keymaps rebind the same actions; transport tooltips follow.
        const QKeySequence portable = QKeySequence::fromString(QStringLiteral("Ctrl+K"), QKeySequence::PortableText);
        window.m_commands->setKeymap(QStringLiteral("premiere"));
        QCOMPARE(split->shortcut(), portable);
        window.m_commands->setKeymap(QStringLiteral("kerf"));
        QCOMPARE(split->shortcut(), QKeySequence(Qt::Key_S));
        QVERIFY(window.m_viewer->overlay()->controlTip(PlayerOverlay::Control::Play).startsWith(QStringLiteral("Play (")));
    }

    void liveSyncAndCommitsAreCollabRequests() {
        QSettings().remove(QStringLiteral("collab/liveProjects"));
        QTemporaryDir project;
        const QString path = QDir::cleanPath(QFileInfo(project.path()).absoluteFilePath());
        QJsonObject status = sharedStatus();
        status.insert("changed", true);
        {
            MainWindow window{project.path(), QString()};
            window.setConnection(QStringLiteral("ready"));
            // Without live sync remembered, connecting asks for the status and the library.
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preset_library"}}));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            QAction *live = window.m_commands->action(QStringLiteral("collab.live"));
            QVERIFY(live->isCheckable() && live->isEnabled() && !live->isChecked());
            live->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", true}}));
            QVERIFY(QSettings().value(QStringLiteral("collab/liveProjects")).toStringList().contains(path));
            status.insert("live", true);
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("live"));
            QVERIFY(window.m_syncChip->text().endsWith(QStringLiteral("· live")));

            window.m_commands->action(QStringLiteral("collab.sync"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_sync"}}));
            answerInput(&window, QStringLiteral("Tighten the intro"));
            window.m_commands->action(QStringLiteral("collab.commit"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_commit"}, {"message", "Tighten the intro"}}));
            answerInput(&window, QStringLiteral("color-pass"));
            window.m_commands->action(QStringLiteral("collab.newBranch"))->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_branch"}, {"name", "color-pass"}, {"create", true}}));
        }
        {
            // The next window for the project turns live sync on as it connects.
            MainWindow window{project.path(), QString()};
            QAction *live = window.m_commands->action(QStringLiteral("collab.live"));
            QVERIFY(live->isChecked());
            window.setConnection(QStringLiteral("ready"));
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", true}}));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
            live->trigger();
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_live"}, {"enabled", false}}));
            QVERIFY(!QSettings().value(QStringLiteral("collab/liveProjects")).toStringList().contains(path));
        }
        QSettings().remove(QStringLiteral("collab/liveProjects"));
    }

    void divergedProjectionIsLoadedOrKept() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        const QJsonObject timeline{{"fps", 24}, {"tracks", QJsonArray{}}};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 0}, {"projectionStatus", "diverged"}, {"timeline", timeline}});
        QVERIFY(!window.m_divergedBanner->isHidden());
        QVERIFY(window.findChild<QLabel *>(QStringLiteral("divergedTitle"))->text().contains(QStringLiteral("project.kerf changed on disk")));
        // kerfd refuses to sync until one side is chosen.
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")),
                 QStringLiteral("project.kerf changed on disk: load it or keep the editor's version first"));
        window.m_loadFromDisk->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "projection_import"}}));
        window.m_keepEditorVersion->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "projection_repair"}}));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 0}, {"projectionStatus", "clean"}, {"timeline", timeline}});
        QVERIFY(window.m_divergedBanner->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("projection.import")), QStringLiteral("project.kerf matches the editor"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
    }

    void pullRequestsOpenWithoutGh() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject status = sharedStatus();
        status.insert("branch", "color");
        status.insert("gh", false);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));
        // Without gh the bridge answers with GitHub's page for starting one.
        const QString compare = QStringLiteral("https://github.com/jun/film/compare/color?expand=1");
        window.handleEvent(QJsonObject{{"type", "collab_pull_request"}, {"url", compare}, {"created", false}});
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Opened GitHub's page for a new pull request: ") + compare);
        const QString existing = QStringLiteral("https://github.com/jun/compare/pull/4");
        window.handleEvent(QJsonObject{{"type", "collab_pull_request"}, {"url", existing}, {"created", false}});
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("This branch already has a pull request: ") + existing);
    }

    // Every command the registry lists for the editor is wired: a frontend
    // command that can run has a handler here or is a switch the window
    // follows; one still to come says so. The new ones reach their places.
    void everyEditorCommandIsWired() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        char *raw = kerf_commands_catalog("kerf");
        QVERIFY(raw);
        const QJsonArray commands = QJsonDocument::fromJson(QByteArray(raw)).object().value("commands").toArray();
        kerf_bridge_string_free(raw);
        QVERIFY(commands.size() > 80);
        QStringList unwired;
        QStringList coming;
        for (const QJsonValue &value : commands) {
            const QJsonObject command = value.toObject();
            const QString id = command.value("id").toString();
            QAction *action = window.m_commands->action(id);
            QVERIFY2(action, qPrintable(id));
            if (command.value("kind").toString() != QStringLiteral("frontend")) continue;
            if (window.m_commands->reason(id) == QStringLiteral("Coming in this release")) {
                coming << id;
                continue;
            }
            if (!action->isCheckable() && !window.m_commandHandlers.contains(id)) unwired << id;
        }
        QVERIFY2(unwired.isEmpty(), qPrintable(unwired.join(QStringLiteral(", "))));
        coming.sort();
        // Nothing is left "Coming in this release" (W3: Sequence Settings and
        // Customise Keyboard have their dialogs).
        QVERIFY2(coming.isEmpty(), qPrintable(coming.join(QStringLiteral(", "))));
        // Export Master is ⌘E; Export… keeps the toolbar's chosen quality.
        QAction *master = window.m_commands->action(QStringLiteral("export.master"));
        QCOMPARE(master->shortcut(), QKeySequence::fromString(QStringLiteral("Ctrl+E"), QKeySequence::PortableText));
        QVERIFY(window.m_commands->action(QStringLiteral("file.export"))->shortcut().isEmpty());
        master->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "master"}}));
        window.m_commands->action(QStringLiteral("qc.run"))->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "check"}}));
        // Add Track (⌘⇧T) is the timeline's own command, its menu the handler's.
        QCOMPARE(window.m_commands->action(QStringLiteral("timeline.addTrack"))->shortcut(),
                 QKeySequence::fromString(QStringLiteral("Ctrl+Shift+T"), QKeySequence::PortableText));
        // The viewer's switches follow the viewer, and drive it.
        QAction *safe = window.m_commands->action(QStringLiteral("view.safeZones"));
        QCOMPARE(safe->isChecked(), window.m_viewer->safeZonesShown());
        safe->trigger();
        QCOMPARE(window.m_viewer->safeZonesShown(), safe->isChecked());
        QCOMPARE(window.m_safeZonesAction->isChecked(), safe->isChecked());
        window.m_safeZonesAction->trigger();
        QCOMPARE(safe->isChecked(), window.m_viewer->safeZonesShown());
        QAction *device = window.m_commands->action(QStringLiteral("view.devicePreview"));
        const bool shown = device->isChecked();
        device->trigger();
        QCOMPARE(device->isChecked(), !shown);
        QCOMPARE(window.m_rail->deviceButton()->isChecked(), !shown);
        device->trigger();
        // Comments is the assistant's switch.
        QAction *comments = window.m_commands->action(QStringLiteral("view.comments"));
        QVERIFY(comments->isChecked() && window.chatOpen());
        comments->trigger();
        QVERIFY(!window.chatOpen());
        QVERIFY(!window.m_commands->action(QStringLiteral("window.assistant"))->isChecked());
        comments->trigger();
        QVERIFY(window.chatOpen());
        // The card's Accept and Reject go out as the proposal's.
        window.bus()->run(QStringLiteral("comment.reject"), QJsonObject{{"proposal", "p4"}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "reject"}, {"proposalId", "p4"}, {"revision", 3}}));
        // The workspaces' commands are a radio group.
        window.m_commands->action(QStringLiteral("workspace.deliver"))->trigger();
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Deliver"));
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.deliver"))->isChecked());
        QVERIFY(!window.m_commands->action(QStringLiteral("workspace.review"))->isChecked());
        window.m_commands->action(QStringLiteral("workspace.review"))->trigger();
        QSettings().remove(QStringLiteral("workspace"));
    }

    void syncConflictsSayWhereAndWhoseVersionWasKept() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{}}, {"fastForward", false}, {"pushed", false},
            {"conflicts", QJsonArray{
                QJsonObject{{"path", "s1/V1/c3"}, {"what", "duration changed on both sides"}, {"kept", "ours"}},
                QJsonObject{{"path", "s1/V1/c9"}, {"what", "overlapped c5 on V1; moved to V2"}, {"kept", "theirs"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{}}}}});
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("Kept yours · s1/V1/c3: duration changed on both sides")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept theirs · s1/V1/c9: overlapped c5 on V1; moved to V2")));
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Synced: 1 change kept yours, 1 change kept theirs"));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::WiringTest)

#include "wiring_test.moc"
