// Tests for the status bar and connection recovery.

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

class StatusTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void disconnectedControlsAndRecovery() {
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Connecting"));
        QVERIFY(!window.m_sendButton->isEnabled());
        window.handleEvent(QJsonObject{{"type", "connection"}, {"status", "disconnected"}, {"message", "Daemon unavailable"}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Offline"));
        QVERIFY(window.m_reconnectButton->isEnabled());
        QVERIFY(!window.m_timeline->isEnabled());
        QVERIFY(!window.m_connectionBanner->isHidden());
        QVERIFY(window.m_connectionMessage->text().contains(QStringLiteral("Daemon unavailable")));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(window.m_agentStatus->text(), QStringLiteral("Offline"));
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

    void syncChipFollowsTheCollabStatus() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(window.m_syncChip->isHidden());
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Reading the project's Git status"));
        const QJsonObject status = sharedStatus();
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QVERIFY(!window.m_syncChip->isHidden());
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⎇ main ↑2 ↓1"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("idle"));
        QVERIFY(window.m_syncChip->toolTip().contains(QStringLiteral("Syncs with origin/main")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.sync")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.commit")), QStringLiteral("Nothing to commit"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.publish")), QStringLiteral("The project is already published"));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.pullRequest")));

        // A click on the chip syncs now.
        window.m_lastCommand = {};
        window.m_syncChip->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_sync"}}));

        // Switch Branch lists the other branches.
        window.buildBranchMenu(window.m_branchMenu);
        QCOMPARE(window.m_branchMenu->actions().size(), 1);
        window.m_branchMenu->actions().first()->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "collab_branch"}, {"name", "color"}, {"create", false}}));

        // While a sync runs the chip says what it is doing, and Sync Now waits.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "fetching"}});
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⟳ Pulling…"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("syncing"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("A sync is running"));

        // A finished sync lists what came in and what kept yours, then
        // reads the preset library again.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "done"}, {"report", QJsonObject{
            {"incoming", QJsonArray{QJsonObject{{"commit", "a1b2c3d"}, {"author", "jun"}, {"subject", "Trimmed c3"}}}},
            {"fastForward", false}, {"pushed", true},
            {"conflicts", QJsonArray{QJsonObject{{"path", "project.kerf"}, {"what", "c3 duration"}, {"kept", "ours"}}}},
            {"renamed", QJsonArray{}}, {"summary", QJsonArray{"Trimmed c3"}}}}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("idle"));
        const int count = window.m_activityList->count();
        QVERIFY(window.m_activityList->item(count - 2)->text().endsWith(QStringLiteral("jun · Trimmed c3")));
        QVERIFY(window.m_activityList->item(count - 1)->text().endsWith(QStringLiteral("Kept yours · project.kerf: c3 duration")));
        QVERIFY(window.m_syncChip->toolTip().contains(QStringLiteral("jun · Trimmed c3")));
        QCOMPARE(window.statusBar()->currentMessage(), QStringLiteral("Synced: 1 commit in, pushed, 1 change kept yours"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("preset_library"));

        // A failure goes to the error banner.
        window.handleEvent(QJsonObject{{"type", "collab_sync"}, {"stage", "failed"}, {"message", "the remote rejected the push"}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("failed"));
        QVERIFY(window.m_syncChip->text().endsWith(QStringLiteral("sync failed")));
        QVERIFY(!window.m_errorBanner->isHidden());
        QCOMPARE(window.m_errorMessage->text(), QStringLiteral("Sync failed: the remote rejected the push"));

        QJsonObject conflicted = status;
        conflicted.insert("conflicted", true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", conflicted}});
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("conflict"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.sync")), QStringLiteral("Finish the merge in progress with git first"));

        const QJsonObject local{{"repository", false}, {"branches", QJsonArray{}}, {"gh", false}, {"live", false}, {"busy", false}};
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", local}});
        QCOMPARE(window.m_syncChip->text(), QStringLiteral("⎇ Not shared"));
        QCOMPARE(window.m_syncChip->property("state").toString(), QStringLiteral("local"));
        QVERIFY(window.m_commands->reason(QStringLiteral("collab.sync")).startsWith(QStringLiteral("Share the project with Git first")));
        QVERIFY(window.m_commands->isEnabled(QStringLiteral("collab.publish")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("collab.pullRequest")), window.m_commands->reason(QStringLiteral("collab.sync")));

        // Every state's words read at 4.5:1 on the status bar, in both
        // appearances; "Not shared" is the only sign a project is not.
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
        {
            QSet<QString> states;
            for (const QJsonObject &event : events) {
                window.handleEvent(event);
                const QString state = window.m_syncChip->property("state").toString();
                states.insert(state);
                const QColor ink = window.m_syncChip->palette().color(QPalette::ButtonText);
                const qreal ratio = contrastRatio(ink, editorTheme().statusBar);
                QVERIFY2(ratio >= 4.5, qPrintable(QStringLiteral("%1: %2 on %3: %4")
                                                      .arg(state, ink.name(), editorTheme().statusBar.name())
                                                      .arg(ratio, 0, 'f', 2)));
            }
            QCOMPARE(states, (QSet<QString>{"idle", "syncing", "live", "failed", "conflict", "local"}));
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StatusTest)

#include "status_test.moc"
