// Tests for workspaces, docking and the layout of the editor window.

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
#include "browser_panel.h"
#include "deliver_pane.h"
#include "deliver_tray.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "finish_tray.h"
#include "inspector_column.h"
#include "loudness_view.h"
#include "program_strip.h"
#include "qc_view.h"

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

class WorkspaceTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // The finish pass's small fixes, each as the owner met it: Export makes
    // the finished file unless asked for a draft; the rail's device button
    // keeps showing the preview as it is while its menu is open; a pinned
    // proposal's time ages; the viewer's empty lines stay on the phone's
    // screen.
    void finishPassFixes() {
        MainWindow window{QString(), QString()};
        window.resize(1366, 737);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCOMPARE(window.m_exportButton->qualityName(), QStringLiteral("Master"));
        QCOMPARE(window.m_exportButton->quality(), QStringLiteral("full"));
        QVERIFY(window.m_exportButton->masterAction()->isChecked());
        QCOMPARE(window.m_exportButton->draftAction()->data().toString(), QStringLiteral("preview"));

        QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
        QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
        timeline.insert(QStringLiteral("width"), 1080);
        timeline.insert(QStringLiteral("height"), 1920);
        snapshot.insert(QStringLiteral("timeline"), timeline);
        window.applySnapshot(snapshot);
        QCoreApplication::processEvents();
        RailButton *device = window.m_rail->deviceButton();
        QVERIFY(device->isChecked());
        bool checkedWhileOpen = false;
        bool menuShown = false;
        QTimer::singleShot(50, &window, [&] {
            QWidget *popup = QApplication::activePopupWidget();
            menuShown = qobject_cast<QMenu *>(popup);
            checkedWhileOpen = device->isChecked();
            if (popup) popup->close();
        });
        QTest::mouseClick(device, Qt::LeftButton, {}, device->discRect().center());
        QVERIFY(menuShown);
        QVERIFY(checkedWhileOpen);
        QVERIFY(device->isChecked());

        window.m_chat->setProposalShown(true, QStringLiteral("p1"));
        QCOMPARE(window.m_chat->proposalTime()->text(), QStringLiteral("now"));
        window.m_chat->m_proposalSince = QDateTime::currentDateTime().addSecs(-5 * 60);
        window.m_chat->m_welcomeSince = QDateTime::currentDateTime().addSecs(-5 * 60);
        window.m_chat->refreshTimes();
        const QString aged = relativeTime(QDateTime::currentDateTime().addSecs(-5 * 60), QDateTime::currentDateTime());
        QCOMPARE(window.m_chat->proposalTime()->text(), aged);
        QCOMPARE(window.m_chat->m_welcomeTime->text(), aged);
        window.m_chat->setProposalShown(false, QString());

        // An empty phone: the two lines wrap inside its screen, no ink in
        // the screen's outer 12 points on either side.
        ViewerWidget viewer;
        viewer.resize(560, 460);
        viewer.setProjectInfo(1080, 1920);
        viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&viewer));
        const QImage shot = viewer.grab().toImage();
        const qreal ratio = shot.devicePixelRatio();
        const QRectF card = viewer.stageLayout().card;
        const int middle = qRound(card.center().y() * ratio);
        for (const qreal x : {card.left() + 2, card.left() + 10, card.right() - 10, card.right() - 2}) {
            // The bare screen, well above the lines.
            const QRgb screen = shot.pixelColor(qRound(x * ratio), middle - qRound(80 * ratio)).rgb();
            for (int y = middle - qRound(30 * ratio); y <= middle + qRound(30 * ratio); ++y)
                QCOMPARE(shot.pixelColor(qRound(x * ratio), y).rgb(), screen);
        }
    }

    void timelineFillsTheRemovedSourceRow() {
        MainWindow window{QString(), QString()};
        window.resize(1280, 800);
        window.show();
        QTest::qWait(30);
        QVERIFY(window.findChild<QWidget *>(QStringLiteral("sourceTray")) == nullptr);
        // The viewer panel (with its transport) meets the timeline panel directly.
        auto *viewerPanel = window.findChild<QWidget *>(QStringLiteral("viewerFrame"));
        auto *timelinePanel = window.findChild<QWidget *>(QStringLiteral("timelinePanel"));
        QVERIFY(viewerPanel && timelinePanel);
        const int viewerBottom = viewerPanel->mapTo(&window, QPoint(0, viewerPanel->height())).y();
        const int timelineTop = timelinePanel->mapTo(&window, QPoint(0, 0)).y();
        // Only the timeline's title strip sits between them.
        QVERIFY(timelineTop >= viewerBottom - 4);
        QVERIFY(timelineTop <= viewerBottom + 36);
        // Six tracks at their default heights fit at 1280x800 (three picture
        // tracks over three sound ones), under the ruler.
        QVERIFY2(window.m_timelineScroll->viewport()->height() - kRulerHeight >= 3 * kVideoTrackHeight + 3 * kTrackHeight,
                 qPrintable(QString::number(window.m_timelineScroll->viewport()->height())));
        // A snapshot arriving after the window is laid out: the title
        // strips' details show whole, not as an ellipsis.
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"duration", 8}, {"timeline", QJsonObject{{"name", "Cut"}, {"fps", 24}, {"tracks", QJsonArray{}}}}});
        QCoreApplication::processEvents();
        {
            auto *detail = window.findChild<QToolButton *>(QStringLiteral("viewerDetail"));
            QVERIFY(detail && detail->width() >= detail->fontMetrics().horizontalAdvance(detail->text()));
            auto *tracks = window.findChild<QLabel *>(QStringLiteral("timelineDetail"));
            QVERIFY(tracks && tracks->width() >= tracks->fontMetrics().horizontalAdvance(tracks->text()));
        }
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(QJsonObject{{"duration", 8}, {"timeline", QJsonObject{
                {"name", "Cut"}, {"fps", 24}, {"tracks", QJsonArray{}}
            }}});
            QVERIFY(window.grab().save(screenshot + QStringLiteral(".empty.png")));
            const QString sample = qEnvironmentVariable("KERF_QT_UI_SAMPLE_VIDEO");
            if (!sample.isEmpty()) {
                const QJsonArray sources{QJsonObject{{"id", "sample"}, {"path", sample}}};
                window.applySnapshot(QJsonObject{{"duration", 4}, {"timeline", QJsonObject{
                    {"name", "Cut"}, {"fps", 24},
                    {"sources", sources},
                    {"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "sample-clip"}, {"sourceId", "sample"}, {"timelineStart", 0},
                                    {"duration", 4}, {"in", 0}, {"out", 4}}}}}}}
                }}});
                QTRY_VERIFY_WITH_TIMEOUT(window.m_artwork->frames.contains(QStringLiteral("sample")), 5000);
                window.m_viewer->setFrame(window.m_artwork->frames.value(QStringLiteral("sample")), 0);
                QVERIFY(window.grab().save(screenshot));
            }
        }
    }

    // The four built-in workspaces and one's own: each is a Window ▸
    // Workspaces item and a registry command (⌥⇧1-4); each keeps its
    // changes until reset; a workspace saved as Grading comes back as it
    // was saved, its stage with it.
    void workspacesDockPanelsAndRememberLayouts() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        {
            MainWindow window{QString(), QString()};
            window.show();
            Workspace *workspace = window.m_workspace;
            const auto panel = [workspace](const char *id) { return workspace->panel(QString::fromLatin1(id)); };
            const auto command = [&window](const char *id) { return window.m_commands->action(QString::fromLatin1(id)); };
            ads::CDockWidget *timeline = panel("timeline");
            ads::CDockWidget *project = panel("project");
            QAction *toggle = command("window.timeline");
            QVERIFY(toggle->isChecked());
            QVERIFY(!timeline->isClosed());
            toggle->trigger();
            QVERIFY(timeline->isClosed());
            // Reopening a panel from its own controls keeps the menu in step.
            timeline->toggleView(true);
            QVERIFY(toggle->isChecked());

            QCOMPARE(workspace->names(), (QStringList{"Edit", "Review", "Finish", "Deliver"}));
            QCOMPARE(workspace->current(), QStringLiteral("Review"));
            QCOMPARE(window.m_bus->workspace(), QStringLiteral("Review"));
            // Kerf's conversation starts open, in the Program monitor's stage.
            QVERIFY(window.chatOpen());
            const char *ids[] = {"workspace.edit", "workspace.review", "workspace.finish", "workspace.deliver"};
            for (int index = 0; index < 4; ++index) {
                QAction *choice = command(ids[index]);
                QVERIFY2(choice && choice->isCheckable(), ids[index]);
                QCOMPARE(choice->isChecked(), index == 1);
                QCOMPARE(choice->shortcut(), QKeySequence::fromString(QStringLiteral("Alt+Shift+%1").arg(index + 1), QKeySequence::PortableText));
            }
            QCOMPARE(command("window.resetWorkspace")->shortcut(), QKeySequence::fromString(QStringLiteral("Alt+Shift+0"), QKeySequence::PortableText));

            // Edit: the band as Review's, the panel closed (the Source viewer's place).
            QSignalSpy switched(window.m_bus, &EditorBus::workspaceChanged);
            command("workspace.edit")->trigger();
            QCOMPARE(workspace->current(), QStringLiteral("Edit"));
            QCOMPARE(switched.size(), 1);
            QCOMPARE(switched.first().first().toString(), QStringLiteral("Edit"));
            QVERIFY(!window.chatOpen() && !window.m_viewer->chatOpen());
            QVERIFY(command("workspace.edit")->isChecked() && !command("workspace.review")->isChecked());
            for (const char *id : {"project", "timeline", "browser", "program", "inspector"}) QVERIFY2(!panel(id)->isClosed(), id);
            for (const char *id : {"finish", "deliver"}) QVERIFY2(panel(id)->isClosed(), id);
            QVERIFY(!workspace->stageState().panelOpen);
            // Choosing the current one again leaves it chosen.
            command("workspace.edit")->trigger();
            QVERIFY(command("workspace.edit")->isChecked());
            QCOMPARE(workspace->current(), QStringLiteral("Edit"));
            // Review opens it again.
            command("workspace.review")->trigger();
            QVERIFY(window.chatOpen());
            // Finish and Deliver: their tray in the band's place, Finish dimmed.
            command("workspace.finish")->trigger();
            QCOMPARE(workspace->current(), QStringLiteral("Finish"));
            for (const char *id : {"project", "timeline", "browser", "deliver"}) QVERIFY2(panel(id)->isClosed(), id);
            QVERIFY(!panel("finish")->isClosed() && !panel("inspector")->isClosed() && !panel("program")->isClosed());
            QVERIFY(workspace->stageState().dim);
            QVERIFY(window.chatOpen());
            // Reset in Finish applies its stage again, so the regions hear
            // it although the name is the same: the Inspector column goes
            // back to its Inspector page.
            window.m_inspectorColumn->setPage(InspectorColumn::Page::Source);
            switched.clear();
            command("window.resetWorkspace")->trigger();
            QCOMPARE(switched.size(), 1);
            QCOMPARE(switched.first().first().toString(), QStringLiteral("Finish"));
            QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Inspector);
            QVERIFY(workspace->stageState().dim);
            command("workspace.deliver")->trigger();
            QVERIFY(panel("finish")->isClosed() && !panel("deliver")->isClosed());
            QVERIFY(!workspace->stageState().dim);
            command("workspace.review")->trigger();
            for (const char *id : {"project", "timeline", "browser"}) QVERIFY2(!panel(id)->isClosed(), id);
            for (const char *id : {"finish", "deliver"}) QVERIFY2(panel(id)->isClosed(), id);

            // Each workspace keeps its changes until it is reset.
            project->toggleView(false);
            workspace->activate(QStringLiteral("Edit"));
            QVERIFY(!project->isClosed());
            workspace->activate(QStringLiteral("Review"));
            QVERIFY(project->isClosed());
            QVERIFY(!command("window.project")->isChecked());
            workspace->resetCurrent();
            QVERIFY(!project->isClosed());

            // One's own: Finish with the Inspector column closed, saved as Grading.
            workspace->activate(QStringLiteral("Finish"));
            panel("inspector")->toggleView(false);
            QVERIFY(workspace->saveAs(QStringLiteral("Grading")));
            QVERIFY(!workspace->saveAs(QStringLiteral("grading")));
            QVERIFY(!workspace->saveAs(QStringLiteral("review")));
            QVERIFY(!workspace->saveAs(QStringLiteral("Editing")));
            QCOMPARE(workspace->current(), QStringLiteral("Grading"));
            QCOMPARE(workspace->names().constLast(), QStringLiteral("Grading"));
            QVERIFY(workspace->stageState(QStringLiteral("Grading")).dim);
            // None of the built-ins is chosen while it is.
            for (const char *id : ids) QVERIFY2(!command(id)->isChecked(), id);
            workspace->activate(QStringLiteral("Review"));
            QVERIFY(!panel("inspector")->isClosed() && panel("finish")->isClosed());
            workspace->activate(QStringLiteral("Grading"));
            QVERIFY(panel("inspector")->isClosed() && !panel("finish")->isClosed());
            QCOMPARE(window.m_bus->workspace(), QStringLiteral("Grading"));
            // Changed and saved: the change is the saved layout now.
            panel("inspector")->toggleView(true);
            command("window.saveWorkspace")->trigger();
            QVERIFY(QSettings().value(QStringLiteral("workspace/changes/Grading")).isNull());
            workspace->activate(QStringLiteral("Review"));
            workspace->activate(QStringLiteral("Grading"));
            QVERIFY(!panel("inspector")->isClosed());

            // A built-in's stage saved: Edit with its panel open.
            workspace->activate(QStringLiteral("Edit"));
            QVERIFY(!window.chatOpen());
            window.setChatOpen(true);
            command("window.saveWorkspace")->trigger();
            workspace->activate(QStringLiteral("Review"));
            workspace->activate(QStringLiteral("Edit"));
            QVERIFY(window.chatOpen());
            workspace->activate(QStringLiteral("Review"));

            // Window ▸ Workspaces as the design lists it.
            auto *menu = window.findChild<QMenu *>(QStringLiteral("workspacesMenu"));
            QVERIFY(menu);
            emit menu->aboutToShow();
            QStringList items;
            for (const QAction *action : menu->actions()) items << (action->isSeparator() ? QStringLiteral("-") : action->text());
            QCOMPARE(items, (QStringList{"Workspaces", "Edit", "Review", "Finish", "Deliver", "Grading", "-",
                                         "Save Changes to This Workspace", "Save as New Workspace…", "Reset to Saved Layout"}));
            QVERIFY(!menu->actions().first()->isEnabled());
            QVERIFY(command("workspace.review")->isChecked());
            QAction *grading = menu->actions().at(5);
            QVERIFY(grading->isCheckable() && !grading->isChecked());
            grading->trigger();
            QCOMPARE(workspace->current(), QStringLiteral("Grading"));

            window.m_commands->action(QStringLiteral("app.preferences"))->trigger();
            QVERIFY(window.m_preferences->isVisible());
            window.close();
        }
        {
            // The next window opens in the workspace the last one closed in.
            MainWindow window{QString(), QString()};
            QCOMPARE(window.m_workspace->current(), QStringLiteral("Grading"));
            QCOMPARE(window.m_bus->workspace(), QStringLiteral("Grading"));
            QVERIFY(!window.m_workspace->panel(QStringLiteral("finish"))->isClosed());
            QVERIFY(window.m_workspace->panel(QStringLiteral("timeline"))->isClosed());
        }
    }

    // Finish and Deliver put their tray in the place of the whole band, at
    // the band's height, under the same top row; their panes hear the
    // engine through the bus.
    void finishAndDeliverReplaceTheBand() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        Workspace *workspace = window.m_workspace;
        const auto area = [workspace](const char *id) { return workspace->panel(QString::fromLatin1(id))->dockAreaWidget(); };
        const auto frame = [&window](const QWidget *widget) { return QRect(widget->mapTo(&window, QPoint()), widget->size()); };
        QCoreApplication::processEvents();
        const QRect program = frame(area("program"));
        const QRect inspector = frame(area("inspector"));
        const QRect band = frame(area("timeline")).united(frame(area("project"))).united(frame(area("browser")));

        window.m_commands->action(QStringLiteral("workspace.finish"))->trigger();
        QCoreApplication::processEvents();
        QVERIFY(window.m_finishTray->isVisible());
        const QRect tray = frame(area("finish"));
        QCOMPARE(tray.left(), 0);
        QCOMPARE(tray.right(), window.width() - 1);
        QVERIFY2(std::abs(tray.top() - band.top()) <= 1 && std::abs(tray.height() - band.height()) <= 1,
                 qPrintable(QStringLiteral("%1 %2 / %3 %4").arg(tray.top()).arg(tray.height()).arg(band.top()).arg(band.height())));
        QCOMPARE(frame(area("program")), program);
        QCOMPARE(frame(area("inspector")), inspector);
        // The tray's own bar is its strip: no tab row over it.
        QVERIFY(area("finish")->titleBar()->isHidden());
        // Five panes, 8 apart: the vectorscope and loudness 240, the rest sharing.
        FinishTray *finish = window.m_finishTray;
        QCOMPARE(finish->pane(QStringLiteral("vectorscope"))->width(), 240);
        QCOMPARE(finish->pane(QStringLiteral("loudness"))->width(), 240);
        const int waveform = finish->pane(QStringLiteral("waveform"))->width();
        QVERIFY2(std::abs(waveform - 352) <= 2, qPrintable(QString::number(waveform)));
        QVERIFY(std::abs(finish->pane(QStringLiteral("qc"))->width() - 368) <= 2);
        QCOMPARE(finish->pane(QStringLiteral("vectorscope"))->x() - finish->pane(QStringLiteral("waveform"))->geometry().right() - 1, 8);
        QCOMPARE(finish->pane(QStringLiteral("waveform"))->x(), 8);
        // Run QC and Ask follow their registry actions: offline they are
        // unavailable, with the registry's reason; their tips carry the
        // keymap's shortcut.
        QAction *runQc = window.m_commands->action(QStringLiteral("qc.run"));
        QVERIFY(finish->runQcButton()->isEnabled() && finish->runQcButton()->toolTip() == runQc->toolTip());
        QCOMPARE(finish->askButton()->toolTip(), window.m_commands->action(QStringLiteral("ai.ask"))->toolTip());
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(!finish->runQcButton()->isEnabled());
        QVERIFY(!runQc->statusTip().isEmpty() && finish->runQcButton()->toolTip().contains(runQc->statusTip()));
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(finish->runQcButton()->isEnabled());
        // Run QC asks the bridge for a check; its answer lists the rows, a
        // failure with its thread.
        window.m_lastCommand = {};
        finish->runQcButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "check"}}));
        QSignalSpy threads(window.m_bus, &EditorBus::openThread);
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false},
                                       {"summary", QJsonObject{{"passed", 1}, {"total", 2}, {"blocking", 0},
                                                               {"rows", QJsonArray{
                                                                   QJsonObject{{"code", "loudness.true_peak"}, {"name", "True peak"}, {"passed", false},
                                                                               {"value", "−0.4 dBTP"}, {"limit", "≤ −1.0"}, {"commentId", "c-2718"}},
                                                                   QJsonObject{{"code", "picture.out_of_range"}, {"name", "Gamut Rec.709"}, {"passed", true},
                                                                               {"value", "legal"}}}}}}});
        QCOMPARE(finish->qcView()->rowCount(), 2);
        QCOMPARE(finish->qcView()->summaryText(), QStringLiteral("1 of 2 pass"));
        auto *inThread = finish->qcView()->findChild<QToolButton *>(QStringLiteral("qcInThread"));
        QVERIFY(inThread);
        inThread->click();
        QCOMPARE(threads.size(), 1);
        QCOMPARE(threads.first().first().toString(), QStringLiteral("c-2718"));
        // The programme's loudness, and the meter's while it plays; unmeasured is a dash.
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("I")), QStringLiteral("—"));
        window.handleEvent(QJsonObject{{"type", "loudness"}, {"status", "ready"}, {"integrated", -23.1}, {"lra", 6.2},
                                       {"truePeak", QJsonArray{-0.4, -1.3}}, {"target", -23}, {"limit", -1}});
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("I")), QStringLiteral("−23.1"));
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("LRA")), QStringLiteral("6.2"));
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("TP L")), QStringLiteral("−0.4 dBTP"));
        window.handleEvent(QJsonObject{{"type", "meter"}, {"momentary", -17.8}, {"shortTerm", QJsonValue()}});
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("M")), QStringLiteral("−17.8"));
        QCOMPARE(finish->loudnessView()->statText(QStringLiteral("S")), QStringLiteral("—"));
        // A pane asked for over the bus comes forward outlined.
        emit window.m_bus->highlightPane(QStringLiteral("loudness"));
        QCOMPARE(finish->highlightedPane(), QStringLiteral("loudness"));

        window.m_commands->action(QStringLiteral("workspace.deliver"))->trigger();
        QCoreApplication::processEvents();
        QVERIFY(!window.m_finishTray->isVisible() && window.m_deliverTray->isVisible());
        QCOMPARE(frame(area("deliver")).left(), 0);
        QCOMPARE(frame(area("deliver")).right(), window.width() - 1);
        QVERIFY(std::abs(frame(area("deliver")).height() - band.height()) <= 1);
        // The Inspector shows the Deliver pane (Video chosen).
        QVERIFY(window.m_inspectorColumn->deliverShown());
        QCOMPARE(window.m_inspectorColumn->currentPage(), window.m_inspectorColumn->deliverPane());
        QCOMPARE(window.m_inspectorColumn->deliverPane()->fileText(), QStringLiteral("No master output yet"));
        // The check reached the preflight too.
        QCOMPARE(window.m_deliverTray->qcView()->rowCount(), 2);
        // Blocking notes hold the master back.
        DeliverTray *deliver = window.m_deliverTray;
        const auto comments = [&window](bool resolved) {
            window.handleEvent(QJsonObject{{"type", "comments"}, {"reviewRevision", 4}, {"items", QJsonArray{
                QJsonObject{{"id", "c-1708"}, {"body", "Caption overlap"}, {"blocking", true}, {"resolved", resolved}},
                QJsonObject{{"id", "c-0903"}, {"body", "Warmer"}, {"blocking", false}, {"resolved", false}}}}});
        };
        // Before the notes are known nothing is said of them, and the
        // master waits for them.
        QCOMPARE(deliver->blockingCount(), -1);
        QVERIFY(deliver->blockingText().isEmpty());
        QVERIFY(!deliver->renderButton()->isEnabled());
        QCOMPARE(deliver->renderButton()->toolTip(), QStringLiteral("Waiting for the review notes"));
        comments(false);
        QCOMPARE(deliver->blockingCount(), 1);
        QCOMPARE(deliver->blockingText(), QStringLiteral("1 blocking note open"));
        QVERIFY(!deliver->renderButton()->isEnabled());
        QCOMPARE(deliver->renderButton()->toolTip(), QStringLiteral("Resolve blocking notes first"));
        comments(true);
        QCOMPARE(deliver->blockingText(), QStringLiteral("No blocking notes"));
        QVERIFY(deliver->renderButton()->isEnabled());
        QCOMPARE(deliver->renderButton()->toolTip(), window.m_commands->action(QStringLiteral("export.master"))->toolTip());
        window.m_lastCommand = {};
        deliver->renderButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "master"}}));
        // The render queue, one row a job.
        window.handleEvent(QJsonObject{{"type", "render_queue"}, {"jobs", QJsonArray{
            QJsonObject{{"jobId", "j1"}, {"name", "Master"}, {"quality", "master"}, {"codec", "prores_hq"}, {"status", "running"}, {"progress", 0.38}},
            QJsonObject{{"jobId", "j2"}, {"name", "Draft"}, {"quality", "review"}, {"status", "queued"}}}}});
        QCOMPARE(deliver->queueRowCount(), 2);

        // Back in Review the band returns, and the Inspector is the Inspector.
        window.m_commands->action(QStringLiteral("workspace.review"))->trigger();
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("timeline")).united(frame(area("project"))).united(frame(area("browser"))), band);
        QCOMPARE(window.m_inspectorColumn->currentPage(), window.m_inspector);
        // A wider window: the tray keeps its height and its fixed panes.
        window.m_commands->action(QStringLiteral("workspace.finish"))->trigger();
        window.resize(1920, 1080);
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("finish")).right(), window.width() - 1);
        QVERIFY(std::abs(frame(area("finish")).height() - band.height()) <= 1);
        QCOMPARE(finish->pane(QStringLiteral("vectorscope"))->width(), 240);
    }

    // Settings from before version 9: the Editing workspace and its
    // layouts become Review's (which then takes its own arrangement, the
    // old tree being gone), one's own named like a built-in is renamed, and
    // one's own from before today's tree opens as Review is arranged.
    void layoutsFromVersion8MigrateToReview() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray older;
        {
            MainWindow window{QString(), QString()};
            window.m_workspace->panel(QStringLiteral("timeline"))->toggleView(false);
            older = window.m_workspace->manager()->saveState(8);
            if (!older.startsWith("<?xml")) older = qUncompress(older);
            older.replace("Name=\"inspector\"", "Name=\"effectControls\"");
            // As version 8 had it: Effects where the browser is with Presets
            // beside it, a Source panel, and no trays.
            QString xml = QString::fromUtf8(older);
            xml.replace(QStringLiteral("Name=\"browser\""), QStringLiteral("Name=\"effects\""));
            xml.replace(QStringLiteral("Current=\"browser\""), QStringLiteral("Current=\"effects\""));
            xml.replace(QRegularExpression(QStringLiteral(R"((<Widget Name="effects"[^>]*/>))")),
                        QStringLiteral(R"(\1<Widget Name="presets" Closed="0"/><Widget Name="source" Closed="0"/>)"));
            xml.remove(QRegularExpression(QStringLiteral(R"re(<Widget Name="(finish|deliver)"[^>]*/>)re")));
            QVERIFY(xml.contains(QStringLiteral("Name=\"presets\"")) && !xml.contains(QStringLiteral("Name=\"finish\"")));
            older = xml.toUtf8();
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Grading"), QStringLiteral("Review")});
            settings.setValue(QStringLiteral("workspace/saved/Editing"), older);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), older);
            settings.setValue(QStringLiteral("workspace/saved/Grading"), older);
            settings.setValue(QStringLiteral("workspace/saved/Review"), older);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        QCOMPARE(workspace->current(), QStringLiteral("Review"));
        QCOMPARE(workspace->names(), (QStringList{"Edit", "Review", "Finish", "Deliver", "Grading", "Review (own)"}));
        const QSettings settings;
        QVERIFY(!settings.contains(QStringLiteral("workspace/changes/Editing")));
        QVERIFY(!settings.contains(QStringLiteral("workspace/saved/Editing")));
        QVERIFY(settings.contains(QStringLiteral("workspace/saved/Review (own)")));
        // The version-8 layout gave way: Review is as it is arranged.
        for (const char *id : {"program", "inspector", "project", "timeline", "browser"})
            QVERIFY2(!workspace->panel(QString::fromLatin1(id))->isClosed(), id);
        for (const char *id : {"finish", "deliver"}) QVERIFY2(workspace->panel(QString::fromLatin1(id))->isClosed(), id);
        // One's own workspace keeps its version-8 layout: its timeline
        // closed as it was saved, the browser in Effects' place, the
        // Inspector column; the trays it had none of are placed, closed.
        workspace->activate(QStringLiteral("Grading"));
        QCOMPARE(workspace->current(), QStringLiteral("Grading"));
        QVERIFY(workspace->panel(QStringLiteral("timeline"))->isClosed());
        for (const char *id : {"program", "inspector", "project", "browser"})
            QVERIFY2(!workspace->panel(QString::fromLatin1(id))->isClosed(), id);
        for (const char *id : {"finish", "deliver"}) {
            QVERIFY2(workspace->panel(QString::fromLatin1(id))->isClosed(), id);
            QVERIFY2(workspace->panel(QString::fromLatin1(id))->dockAreaWidget(), id);
        }
        // Finish, a built-in, has its tray as it is arranged.
        workspace->activate(QStringLiteral("Finish"));
        QVERIFY(!workspace->panel(QStringLiteral("finish"))->isClosed());
        workspace->activate(QStringLiteral("Grading"));
        // Panels that became pages are found by their old ids.
        QCOMPARE(workspace->panel(QStringLiteral("effects")), workspace->panel(QStringLiteral("browser")));
        QCOMPARE(workspace->panel(QStringLiteral("presets")), workspace->panel(QStringLiteral("browser")));
        QCOMPARE(workspace->panel(QStringLiteral("source")), workspace->panel(QStringLiteral("inspector")));
        QVERIFY(!workspace->panel(QStringLiteral("assistant")));
        // The conversation is the stage's, whatever the workspace.
        QVERIFY(window.chatOpen());
    }

    // The Review workspace: the Program monitor is the page, the widest
    // panel of its row, and in its stage the picture, the action rail and
    // Kerf's conversation form one group, centred, the conversation as tall
    // as the picture and level with it. The Inspector column is on its
    // right, and the band of the bin, the timeline and the browser under it.
    void reviewShowsTheVideoWithItsConversationInItsStage() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        const auto panel = [workspace](const char *id) { return workspace->panel(QString::fromLatin1(id)); };
        const auto area = [&panel](const char *id) { return panel(id)->dockAreaWidget(); };
        const auto frame = [&window](const QWidget *widget) { return QRect(widget->mapTo(&window, QPoint()), widget->size()); };

        // No dock of its own any more: it is open in the stage.
        QVERIFY(!panel("assistant"));
        QVERIFY(window.chatOpen());
        QAction *toggle = window.m_commands->action(QStringLiteral("window.assistant"));
        QVERIFY(toggle->isCheckable() && toggle->isChecked());
        QVERIFY(window.m_chatToggle && window.m_chatToggle->isChecked());
        QCOMPARE(window.m_chatToggle->defaultAction(), toggle);
        // Each region in an area of its own; the Source monitor is a page of
        // the Inspector column, Effects and Presets pages of the browser.
        QCOMPARE(panel("source"), panel("inspector"));
        QCOMPARE(panel("effects"), panel("browser"));
        for (const char *id : {"program", "inspector", "project", "timeline", "browser"})
            for (const char *other : {"program", "inspector", "project", "timeline", "browser"})
                if (qstrcmp(id, other) != 0) QVERIFY2(area(id) != area(other), id);
        // Every panel is open, and Window lists each one.
        for (const char *id : {"project", "program", "timeline", "inspector", "browser"}) QVERIFY2(!panel(id)->isClosed(), id);
        for (const char *id : {"project", "program", "timeline", "inspector", "effects"}) {
            QAction *item = window.m_commands->action(QStringLiteral("window.%1").arg(QString::fromLatin1(id)));
            QVERIFY2(item && item->isCheckable() && item->isChecked(), id);
        }
        QVERIFY(!window.m_commands->action(QStringLiteral("window.source"))->isChecked());
        QVERIFY(window.m_commands->action(QStringLiteral("window.presets"))->isChecked());

        const QRect browser = frame(area("inspector"));
        const QRect viewer = frame(area("program"));
        const QRect timeline = frame(area("timeline"));
        // One row: the viewer from the window's edge, then the Inspector column to the other.
        QCOMPARE(viewer.left(), 0);
        QVERIFY(viewer.right() < browser.left());
        QCOMPARE(browser.right(), window.width() - 1);
        QCOMPARE(browser.top(), viewer.top());
        QVERIFY(viewer.width() > 2 * browser.width());
        QVERIFY(timeline.top() > viewer.bottom());
        QCOMPARE(frame(area("project")).left(), 0);
        QCOMPARE(frame(area("browser")).right(), window.width() - 1);
        QVERIFY(viewer.height() > timeline.height());

        // The stage: the picture's laptop (or card), rail, conversation, one
        // group centred in it, the conversation level with the body and as
        // tall.
        const auto checkGroup = [&window](const char *what) {
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            const QRect body = stage.body.toAlignedRect();
            QVERIFY2(!stage.overlay, what);
            QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(window.m_rail->geometry(), stage.rail);
            QVERIFY2(window.m_chat->isVisible() && window.m_rail->isVisible(), what);
            QCOMPARE(stage.rail.left() - qRound(stage.body.right()), window.m_viewer->stageGap());
            QCOMPARE(stage.panel.left() - stage.rail.right() - 1, window.m_viewer->stageGap());
            const int leftSpace = body.left();
            const int rightSpace = window.m_viewer->width() - stage.panel.right() - 1;
            QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1: %2 | %3").arg(what).arg(leftSpace).arg(rightSpace)));
            QVERIFY2(std::abs(stage.body.center().y() - QRectF(stage.panel).center().y()) <= 1, what);
            QCOMPARE(stage.panel.top(), qRound(stage.body.top()));
            QVERIFY2(std::abs(stage.panel.height() - stage.body.height()) <= 1, what);
            QVERIFY2(stage.panel.height() >= ViewerWidget::kLeastChatHeight, what);
        };
        checkGroup("1440");
        // A wide picture keeps four fifths of its width with the
        // conversation closed: the conversation narrows for it from its
        // default, never under its least.
        const int chatWidth = window.m_chat->width();
        QVERIFY2(chatWidth >= ViewerWidget::kMinChatWidth && chatWidth <= ViewerWidget::kDefaultChatWidth, qPrintable(QString::number(chatWidth)));
        const QRectF card = window.m_viewer->videoRect();
        // A wider window widens the viewer; the browsers keep theirs, the
        // conversation widens back towards its own, and the picture takes
        // the rest.
        window.resize(1680, 900);
        QCoreApplication::processEvents();
        QCOMPARE(frame(area("inspector")).width(), browser.width());
        QCOMPARE(frame(area("program")).width(), viewer.width() + 240);
        // (In the design's shorter stage it may have its own width already.)
        QVERIFY(window.m_chat->width() >= chatWidth && window.m_chat->width() <= ViewerWidget::kDefaultChatWidth);
        QVERIFY(window.m_viewer->videoRect().width() >= card.width() - 0.5);
        checkGroup("1680");
        // A laptop's window: the conversation gives way before the picture,
        // which keeps at least four fifths of its width with the
        // conversation closed and stays the larger of the two; the gaps
        // narrow in a narrow stage.
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        {
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            QVERIFY(!stage.overlay);
            QCOMPARE(window.m_viewer->stageGap(), ViewerWidget::kNarrowStageGap);
            // (Which of the picture and the panel is larger in a stage this
            // short is the stage layout's to say: Q-STAGE, W2.)
            QVERIFY(stage.panel.width() >= ViewerWidget::kMinChatWidth);
            window.setChatOpen(false);
            QCoreApplication::processEvents();
            const double closedWidth = window.m_viewer->videoRect().width();
            window.setChatOpen(true);
            QCoreApplication::processEvents();
            QVERIFY(stage.card.width() >= ViewerWidget::kCardKeeps * closedWidth - 1 || stage.panel.width() == ViewerWidget::kMinChatWidth);
            checkGroup("1280");
        }
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        QCOMPARE(window.m_viewer->videoRect(), card);

        // The toolbar's bubble closes it; the picture is centred alone, the
        // rail on its right, and the browsers keep their width.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !window.m_chat->isVisible());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked() && !window.m_rail->chatButton()->isChecked());
        QVERIFY(std::abs(window.m_viewer->videoRect().center().x() - window.m_viewer->width() / 2.0) <= 1);
        QVERIFY(window.m_rail->isVisible());
        QCOMPARE(window.m_rail->x() - qRound(window.m_viewer->stageLayout().body.right()), ViewerWidget::kStageGap);
        QCOMPARE(frame(area("inspector")).width(), browser.width());
        // And opens it again as it was.
        window.m_chatToggle->click();
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && window.m_rail->chatButton()->isChecked());
        QCOMPARE(window.m_viewer->videoRect(), card);
        checkGroup("reopened");

        // The header's close button, with its shortcut in its tip.
        ChatIconButton *close = window.m_chat->closeButton();
        QVERIFY(close->isVisible());
        QVERIFY(close->toolTip().startsWith(QStringLiteral("Close Kerf Assistant")));
        QTest::mouseClick(close, Qt::LeftButton);
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !toggle->isChecked());
        // ⇧⌘J opens it.
        QCOMPARE(toggle->shortcut(), QKeySequence(QStringLiteral("Ctrl+Shift+J")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && window.m_chatToggle->isChecked());
        // The rail's chat button, beside the picture, is the same switch.
        RailButton *rail = window.m_rail->chatButton();
        QVERIFY(rail->isChecked());
        QTest::mouseClick(rail, Qt::LeftButton, {}, rail->discRect().center());
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !toggle->isChecked() && !rail->isChecked());
        QTest::mouseClick(rail, Qt::LeftButton, {}, rail->discRect().center());
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && rail->isChecked());
        // Window ▸ Kerf Assistant too.
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(!window.chatOpen() && !window.m_chatToggle->isChecked());
        // With the Program monitor closed, opening the conversation brings it
        // back, its stage holding the conversation.
        panel("program")->toggleView(false);
        QCoreApplication::processEvents();
        QVERIFY(!window.m_viewer->isVisible());
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(!panel("program")->isClosed() && window.m_chat->isVisible());
        // Open but out of sight with the Program monitor closed, it reads as
        // closed: the timeline's bar stands in for it with Kerf's proposal,
        // and ⇧⌘J brings it back into sight.
        window.setConnection(QStringLiteral("ready"));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "p1"}, {"script", "remove v1c\n"},
                                          {"documentDiff", "-    clip v1c\n"}});
        panel("program")->toggleView(false);
        QCoreApplication::processEvents();
        QVERIFY(window.m_viewer->chatOpen() && !window.chatOpen());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked());
        QVERIFY(window.m_aiBar->isVisible());
        QVERIFY(window.m_aiBarText->text().startsWith(QStringLiteral("Proposal")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(!panel("program")->isClosed() && window.chatOpen() && window.m_chat->isVisible());
        QVERIFY(toggle->isChecked() && window.m_chatToggle->isChecked());
        QVERIFY(!window.m_aiBar->isVisible());
        window.updateProposal(QJsonValue());
        toggle->trigger();
        // The AI bar's Details opens it.
        QToolButton *details = nullptr;
        for (QToolButton *button : window.m_aiBar->findChildren<QToolButton *>())
            if (button->text() == QStringLiteral("Details")) details = button;
        QVERIFY(details);
        details->click();
        QCoreApplication::processEvents();
        QVERIFY(window.chatOpen() && toggle->isChecked() && window.m_chatToggle->isChecked());

        // Dragging its leading edge widens it, within its bounds; the
        // picture gives the room, and the width is this Mac's, kept. (In a
        // window wide enough that the wide picture leaves it its own width.)
        window.resize(1680, 900);
        QCoreApplication::processEvents();
        const int ownWidth = window.m_chat->width();
        QCOMPARE(ownWidth, ViewerWidget::kDefaultChatWidth);
        const QRectF wideCard = window.m_viewer->videoRect();
        QWidget *handle = window.m_chat->resizeHandle();
        QVERIFY(handle->isVisible());
        QCOMPARE(handle->cursor().shape(), Qt::SizeHorCursor);
        QTest::mousePress(handle, Qt::LeftButton, {}, QPoint(2, 60));
        QTest::mouseMove(handle, QPoint(2 - 40, 60));
        QTest::mouseRelease(handle, Qt::LeftButton, {}, QPoint(2 - 40, 60));
        QCoreApplication::processEvents();
        QCOMPARE(window.m_chat->width(), ownWidth + 40);
        QCOMPARE(QSettings().value(QStringLiteral("viewer/chatWidth")).toInt(), ownWidth + 40);
        QVERIFY(window.m_viewer->videoRect().width() <= wideCard.width());
        checkGroup("wider");
        window.m_viewer->setChatWidth(10000);
        QCOMPARE(window.m_viewer->chatWidth(), ViewerWidget::kMaxChatWidth);
        // Shown that wide where the picture keeps its share beside it.
        QVERIFY(window.m_chat->width() <= ViewerWidget::kMaxChatWidth);
        window.resize(1920, 1000);
        QCoreApplication::processEvents();
        QCOMPARE(window.m_chat->width(), ViewerWidget::kMaxChatWidth);
        window.resize(1440, 900);
        QCoreApplication::processEvents();
        window.m_viewer->setChatWidth(10);
        QCOMPARE(window.m_chat->width(), ViewerWidget::kMinChatWidth);
        // Whether it is open is kept too, never in the project.
        window.close();
        QCOMPARE(QSettings().value(QStringLiteral("viewer/chatOpen")).toBool(), true);
    }

    // Add effect brings the browser's Effects page up with its search
    // ready, the Inspector staying in view; adding one opens it there.
    // Dropping an effect on the Inspector inserts it where it lands.
    void inspectorAddsAndDropsEffects() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto snapshot = [&window](int revision, int effects) {
            QJsonArray stack;
            for (int index = 0; index < effects; ++index)
                stack.append(QJsonObject{{"fx", index % 2 ? "blur.gaussian" : "light.edgeglow"}, {"enabled", true}, {"params", QJsonObject{}}});
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                                {"effects", stack}}}}}}}}}});
        };
        snapshot(1, 2);
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("c1"));
        Workspace *workspace = window.m_workspace;
        QVERIFY(window.m_inspector->isVisible());
        InspectorPanel *inspector = window.m_inspector;
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        QCoreApplication::processEvents();
        auto *add = inspector->findChild<QPushButton *>(QStringLiteral("addEffect"));
        QVERIFY(add && add->isVisible());
        QCOMPARE(add->text(), QStringLiteral("Add effect"));
        window.m_browserPanel->setPage(BrowserPanel::Page::Presets);
        add->click();
        QCOMPARE(window.m_browserPanel->page(), BrowserPanel::Page::Effects);
        QVERIFY(window.m_inspector->isVisible());
        QVERIFY(window.m_effects->m_search->hasFocus() || QApplication::focusWidget() == window.m_effects->m_search);
        // Adding: one fx-add, and the Inspector is back.
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        const QJsonObject added = takeOps(window).first().toObject();
        QCOMPARE(added.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(added.value("fx").toString(), QStringLiteral("blur.gaussian"));
        QVERIFY(window.m_inspector->isVisible());
        // The snapshot with it shows it open.
        snapshot(2, 3);
        QCoreApplication::processEvents();
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("effectBox"));
        QCOMPARE(boxes.size(), 3);
        QVERIFY(!boxes.at(2)->findChild<QWidget *>(QStringLiteral("effectParams"))->isHidden());
        // The section's + does what Add effect does.
        QSignalSpy wanted(inspector, &InspectorPanel::addEffectRequested);
        inspector->findChild<GlyphButton *>(QStringLiteral("effectsAdd"))->click();
        QCOMPARE(wanted.size(), 1);
        QCOMPARE(wanted.first().first().toBool(), false);
        // A drop between the first and second effects inserts at 1.
        QCoreApplication::processEvents();
        const QWidget *second = boxes.at(1);
        const QPoint at = second->mapTo(inspector, QPoint(20, 4));
        QMimeData data;
        data.setData(QStringLiteral("application/x-kerf-fx"), QByteArray("color.basic"));
        QDragEnterEvent enter(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &enter);
        QVERIFY(enter.isAccepted());
        QDragMoveEvent move(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &move);
        QCOMPARE(inspector->m_dropIndex, 1);
        QVERIFY(inspector->m_dropLine->isVisible());
        QCOMPARE(inspector->m_dropLine->height(), 2);
        QDropEvent drop(at, Qt::CopyAction, &data, Qt::LeftButton, Qt::NoModifier);
        QCoreApplication::sendEvent(inspector, &drop);
        const QJsonObject dropped = takeOps(window).first().toObject();
        QCOMPARE(dropped.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(dropped.value("fx").toString(), QStringLiteral("color.basic"));
        QCOMPARE(dropped.value("index").toInt(), 1);
        QVERIFY(!inspector->m_dropLine->isVisible());
        // The Effects browser's cards drag as their effect.
        QListWidgetItem *card = window.m_effects->m_items.value(QStringLiteral("blur.gaussian"));
        QVERIFY(card);
        std::unique_ptr<QMimeData> dragged(window.m_effects->m_cards->model()->mimeData({window.m_effects->m_cards->model()->index(window.m_effects->m_cards->row(card), 0)}));
        QVERIFY(dragged);
        QCOMPARE(dragged->data(QStringLiteral("application/x-kerf-fx")), QByteArray("blur.gaussian"));
    }

    // Clicking a clip brings the Inspector page forward over the column's
    // Source page and remembers it came from there; clearing the selection
    // goes back, unless the Inspector was chosen by hand. A clip selected by
    // an edit raises nothing, a closed column stays closed, and the browser
    // in the band is never in the way. Keyboard focus stays put.
    void inspectorFollowsClickedClips() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspectorFollowsSelection"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspectorFollowsSelection"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        InspectorColumn *column = window.m_inspectorColumn;
        ads::CDockWidget *inspector = workspace->panel(QStringLiteral("inspector"));
        using Page = InspectorColumn::Page;
        QCOMPARE(column->page(), Page::Inspector);
        const QRect chat = window.m_chat->geometry();
        // The Source page chosen by hand.
        column->pageButton(Page::Source)->click();
        QCOMPARE(column->page(), Page::Source);
        QVERIFY(window.m_commands->action(QStringLiteral("window.source"))->isChecked());
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QCoreApplication::processEvents();
        QWidget *focus = QApplication::focusWidget();
        // A click on a clip: the Inspector, showing it; focus stays.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QCOMPARE(column->page(), Page::Inspector);
        QVERIFY(workspace->inspectorRaisedBySelection());
        QCOMPARE(window.m_inspector->nameText(), QStringLiteral("fisher-close"));
        QCOMPARE(QApplication::focusWidget(), focus);
        QVERIFY(!window.m_commands->action(QStringLiteral("window.source"))->isChecked());
        // Another clip: only the values change.
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCOMPARE(column->page(), Page::Inspector);
        // Deselected (an empty lane, Esc): back to Source.
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QCOMPARE(column->page(), Page::Source);
        QVERIFY(window.m_selectedClip.isEmpty());
        // A selection by an edit raises nothing.
        window.selectClip(QStringLiteral("v1b"));
        QCOMPARE(column->page(), Page::Source);
        window.selectClip(QString());
        // Source chosen while a clip is selected stays until a new clip is clicked.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCOMPARE(column->page(), Page::Inspector);
        column->pageButton(Page::Source)->click();
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCOMPARE(column->page(), Page::Source);
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCOMPARE(column->page(), Page::Inspector);
        emit window.m_timeline->deselectRequested();
        QCOMPARE(column->page(), Page::Source);
        // ⌘4: the Inspector by hand, which stays when the selection clears
        // and shows the sequence; ⌘4 again goes back to Source.
        QAction *show = window.m_commands->action(QStringLiteral("window.inspector"));
        show->trigger();
        QCoreApplication::processEvents();
        QCOMPARE(column->page(), Page::Inspector);
        QVERIFY(show->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        emit window.m_timeline->deselectRequested();
        QCOMPARE(column->page(), Page::Inspector);
        QCOMPARE(window.m_inspector->m_mode, InspectorPanel::Mode::Sequence);
        show->trigger();
        QCoreApplication::processEvents();
        QCOMPARE(column->page(), Page::Source);
        QVERIFY(!inspector->isClosed() && show->isChecked());
        // Following off: clicks leave the column alone, and it is remembered.
        QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
        QVERIFY(follows->isCheckable() && follows->isChecked());
        follows->trigger();
        QVERIFY(!QSettings().value(QStringLiteral("editor/inspectorFollowsSelection")).toBool());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCOMPARE(column->page(), Page::Source);
        follows->trigger();
        QVERIFY(Workspace::inspectorFollowsSelection());
        emit window.m_timeline->deselectRequested();
        // A closed column is not reopened by a click; ⌘4 opens it on the Inspector.
        inspector->toggleView(false);
        QVERIFY(!show->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isClosed());
        show->trigger();
        QVERIFY(!inspector->isClosed());
        QCOMPARE(column->page(), Page::Inspector);
        // The browser is in the band, beside the timeline: choosing its pages
        // never hides the Inspector, which shows the clip clicked.
        BrowserPanel *browser = window.m_browserPanel;
        QVERIFY(workspace->panel(QStringLiteral("browser"))->dockAreaWidget() != inspector->dockAreaWidget());
        browser->pageButton(BrowserPanel::Page::Presets)->click();
        QCOMPARE(browser->page(), BrowserPanel::Page::Presets);
        // Window ▸ Effects and Presets read as open while the browser is.
        QVERIFY(window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        QVERIFY(window.m_commands->action(QStringLiteral("window.effects"))->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(window.m_inspector->isVisible());
        QCOMPARE(browser->page(), BrowserPanel::Page::Presets);
        // Titles is not built yet: it says so and cannot be chosen.
        QVERIFY(!browser->pageButton(BrowserPanel::Page::Titles)->isEnabled());
        QVERIFY(browser->pageButton(BrowserPanel::Page::Titles)->toolTip().contains(QStringLiteral("Coming in this release")));
        browser->setPage(BrowserPanel::Page::Titles);
        QCOMPARE(browser->page(), BrowserPanel::Page::Presets);
        // Window ▸ Effects closes the browser, and opens it on that page.
        QAction *effects = window.m_commands->action(QStringLiteral("window.effects"));
        effects->trigger();
        QVERIFY(workspace->panel(QStringLiteral("browser"))->isClosed());
        QVERIFY(!effects->isChecked() && !window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        effects->trigger();
        QVERIFY(!workspace->panel(QStringLiteral("browser"))->isClosed());
        QCOMPARE(browser->page(), BrowserPanel::Page::Effects);
        QCOMPARE(browser->currentPage(), window.m_effects);
        // The Inspector's Add effect brings Effects up with its search ready;
        // the effect goes to the clip the Inspector shows.
        browser->setPage(BrowserPanel::Page::Presets);
        takeOps(window);
        emit window.m_inspector->addEffectRequested(false);
        QCoreApplication::processEvents();
        QCOMPARE(browser->page(), BrowserPanel::Page::Effects);
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        {
            const QJsonArray added = takeOps(window);
            QCOMPARE(added.size(), 1);
            QCOMPARE(added.at(0).toObject().value("op").toString(), QStringLiteral("fx-add"));
            QCOMPARE(added.at(0).toObject().value("clip").toString(), QStringLiteral("v1a"));
        }
        QVERIFY(window.m_inspector->isVisible());
        // The conversation never moved.
        QVERIFY(window.m_chat->isVisible());
        QCOMPARE(window.m_chat->geometry(), chat);
    }

    // ⌘4 is Window ▸ Inspector (Effect Controls' old command still finds
    // it), ⌥⌘4 Inspector Follows Selection, ⌃⌥4-6 the column's kinds; the
    // column's Source page is Window ▸ Source Monitor, and a bin item opens
    // there.
    void inspectorCommandsAndSourcePage() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        QAction *inspector = window.m_commands->action(QStringLiteral("window.inspector"));
        QVERIFY(inspector && inspector->isCheckable());
        QCOMPARE(inspector->text(), QStringLiteral("Inspector"));
        QCOMPARE(inspector->shortcut(), QKeySequence(Qt::CTRL | Qt::Key_4));
        QCOMPARE(window.m_commands->action(QStringLiteral("window.effectControls")), inspector);
        QCOMPARE(CommandRegistry::resolve(QStringLiteral("window.effectControls")), QStringLiteral("window.inspector"));
        QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
        QVERIFY(follows && follows->isCheckable());
        QCOMPARE(follows->shortcut(), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_4));
        // The kinds: Video now; Audio and Info say they are coming.
        InspectorColumn *column = window.m_inspectorColumn;
        QAction *video = window.m_commands->action(QStringLiteral("inspector.video"));
        QVERIFY(video->isChecked() && video->isEnabled());
        QCOMPARE(video->shortcut(), QKeySequence(Qt::META | Qt::ALT | Qt::Key_4));
        for (const char *id : {"inspector.audio", "inspector.info"}) {
            QAction *kind = window.m_commands->action(QString::fromLatin1(id));
            QVERIFY2(!kind->isEnabled(), id);
            QCOMPARE(window.m_commands->reason(QString::fromLatin1(id)), QStringLiteral("Coming in this release"));
        }
        QVERIFY(!column->kindButton(InspectorColumn::Kind::Audio)->isEnabled());
        QVERIFY(column->kindButton(InspectorColumn::Kind::Info)->toolTip().contains(QStringLiteral("Coming in this release")));
        column->setKind(InspectorColumn::Kind::Info);
        QCOMPARE(column->kind(), InspectorColumn::Kind::Video);
        // Window ▸ Source Monitor shows the column's Source page, and back.
        QAction *source = window.m_commands->action(QStringLiteral("window.source"));
        source->trigger();
        QCOMPARE(column->page(), InspectorColumn::Page::Source);
        QCOMPARE(column->currentPage(), window.m_sourceMonitor);
        QVERIFY(source->isChecked());
        source->trigger();
        QCOMPARE(column->page(), InspectorColumn::Page::Inspector);
        // ⌃⌥4 brings the Video Inspector back from the Source page.
        source->trigger();
        video->trigger();
        QCOMPARE(column->page(), InspectorColumn::Page::Inspector);
        QVERIFY(video->isChecked());
        // A bin item opens on the Source page; the stage's Source viewer
        // waits for its place (Q-STAGE).
        const QString media = showcaseSnapshot(3, QStringLiteral("clean")).value("timeline").toObject().value("sources").toArray()
                                  .first().toObject().value("id").toString();
        QVERIFY(!media.isEmpty());
        window.openSource(media);
        QCOMPARE(column->page(), InspectorColumn::Page::Source);
        QVERIFY(window.m_sourceMonitor->isOpen());
        QVERIFY(window.m_stageSource && window.m_stageSource != window.m_sourceMonitor && window.m_stageSource->isHidden());
        // A workspace switch goes back to the Inspector (mockup-spec 10).
        window.m_commands->action(QStringLiteral("workspace.edit"))->trigger();
        QCOMPARE(column->page(), InspectorColumn::Page::Inspector);
    }

    // At 1600x1000 the Inspector fits its column: no row wider than the
    // column, and a video clip's identity, its primary rows and every
    // folded section header in view without scrolling.
    // Dragged all the way up, the band's split closes the row over it
    // altogether instead of stopping at a strip of cut-off panels, and comes
    // back when dragged down; the band itself never closes. Short of that,
    // each dock area stops at its panels' least size, so nothing in it is
    // cut off, and the browser keeps its width whichever page is chosen.
    void timelineSplitClosesTheRowOverIt() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        Workspace *workspace = window.m_workspace;
        ads::CDockAreaWidget *timeline = workspace->panel(QStringLiteral("timeline"))->dockAreaWidget();
        ads::CDockAreaWidget *browser = workspace->panel(QStringLiteral("browser"))->dockAreaWidget();
        ads::CDockAreaWidget *viewer = workspace->panel(QStringLiteral("program"))->dockAreaWidget();
        // The rows: the top row, the band (holding the timeline), the trays.
        QWidget *band = timeline->parentWidget();
        QWidget *top = viewer->parentWidget();
        auto *rows = qobject_cast<QSplitter *>(band->parentWidget());
        QVERIFY(rows && rows->orientation() == Qt::Vertical && rows->count() == 3 && top->parentWidget() == rows);
        const int timelineIndex = rows->indexOf(band);
        const int topIndex = rows->indexOf(top);
        QCOMPARE(topIndex, 0);
        QCOMPARE(timelineIndex, 1);
        QVERIFY(rows->isCollapsible(topIndex) && !rows->isCollapsible(timelineIndex) && !rows->isCollapsible(2));
        // Each area's least size is its panels' largest.
        for (const char *id : {"project", "inspector", "browser", "program"}) {
            ads::CDockWidget *dock = workspace->panel(QString::fromLatin1(id));
            const QSize least = dock->dockAreaWidget()->minimumSizeHint();
            QVERIFY2(least.width() >= dock->widget()->minimumSizeHint().width() && least.height() >= dock->widget()->minimumSizeHint().height(),
                     id);
        }
        QVERIFY(timeline->minimumSizeHint().height() >= workspace->panel(QStringLiteral("timeline"))->widget()->minimumHeight());
        // The Editing arrangement holds the timeline's least height.
        QVERIFY(timeline->height() >= timeline->minimumSizeHint().height());
        // Choosing another page leaves the browser's width alone.
        const int browserWidth = browser->width();
        for (const BrowserPanel::Page page : {BrowserPanel::Page::Presets, BrowserPanel::Page::Effects}) {
            window.m_browserPanel->setPage(page);
            QCoreApplication::processEvents();
            QCOMPARE(browser->width(), browserWidth);
        }
        // Part way up: the row stops at its least height, its panels whole.
        const QList<int> arranged = rows->sizes();
        const int total = arranged.at(0) + arranged.at(1);
        QList<int> partWay(3);
        partWay[topIndex] = 120;
        partWay[timelineIndex] = total - 120;
        rows->setSizes(partWay);
        QCoreApplication::processEvents();
        QVERIFY(viewer->height() >= viewer->minimumSizeHint().height());
        // All the way up, as a drag of its handle to the top: closed, the
        // timeline as tall as the rows.
        rows->setSizes(arranged);
        QCoreApplication::processEvents();
        QSplitterHandle *handle = rows->handle(1);
        const QPoint grip(handle->width() / 2, handle->height() / 2);
        QTest::mousePress(handle, Qt::LeftButton, Qt::NoModifier, grip);
        QTest::mouseMove(handle, grip - QPoint(0, handle->mapTo(rows, QPoint()).y() + 40));
        QTest::mouseRelease(handle, Qt::LeftButton, Qt::NoModifier, grip - QPoint(0, handle->mapTo(rows, QPoint()).y() + 40));
        QCoreApplication::processEvents();
        QCOMPARE(rows->sizes().at(topIndex), 0);
        QCOMPARE(rows->sizes().at(timelineIndex), total);
        // Dragged down again, the row comes back.
        rows->setSizes(arranged);
        QCoreApplication::processEvents();
        QVERIFY(viewer->height() > 0 && viewer->isVisible());
        // As it does from a restored layout.
        workspace->resetCurrent();
        QCoreApplication::processEvents();
        rows = qobject_cast<QSplitter *>(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->parentWidget()->parentWidget());
        QVERIFY(rows && rows->childrenCollapsible());
        QVERIFY(!rows->isCollapsible(rows->indexOf(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->parentWidget())));
    }

    void inspectorFitsItsColumn() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("editor/inspector"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("editor/inspector"));
        });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(everyKindSnapshot(1));
        fakeArtwork(window.m_artwork);
        // The design's window: the column's 564 under its strip.
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.clickSelectClip(QStringLiteral("v"));
        QCoreApplication::processEvents();
        InspectorPanel *inspector = window.m_inspector;
        QVERIFY(inspector->isVisible());
        const auto unclipped = [inspector](const QString &what) {
            for (InspectorRow *row : inspector->findChildren<InspectorRow *>()) {
                if (!row->isVisibleTo(inspector)) continue;
                for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly)) {
                    if (!child->isVisibleTo(row)) continue;
                    QVERIFY2(child->geometry().right() < row->width() && child->x() >= 0,
                             qPrintable(QStringLiteral("%1: %2 in %3").arg(what, child->objectName(), row->label()->text())));
                }
                // Every value whole: "+0.40 EV", "−8.0 dB", "−48 px".
                for (ScrubField *field : row->findChildren<ScrubField *>())
                    if (field->isVisibleTo(row))
                        QVERIFY2(field->showsWhole(), qPrintable(QStringLiteral("%1: %2 %3").arg(what, row->label()->text(), field->text())));
                QVERIFY2(row->geometry().right() < row->parentWidget()->width(), qPrintable(row->label()->text()));
            }
        };
        unclipped(QStringLiteral("column"));
        // Start, End and Duration share the column in thirds, and the kind
        // line runs to the column's edge (under ⋯), so neither is cut short
        // at 300 points (text widths differ by platform; the room does not).
        QCOMPARE(inspector->m_timingFields.size(), 3);
        for (ScrubField *field : inspector->m_timingFields)
            QVERIFY2(field->width() >= (inspector->width() - 2 * 16 - 2 * 8) / 3 - 1, qPrintable(QString::number(field->width())));
        QVERIFY(inspector->m_kind->x() + inspector->m_kind->width() >= inspector->width() - 16 - 1);
        // Everything of the video, folded, is in view.
        QScrollBar *scroll = inspector->scrollArea()->verticalScrollBar();
        QVERIFY2(scroll->maximum() == 0, qPrintable(QStringLiteral("scrolls %1 in %2").arg(scroll->maximum()).arg(inspector->height())));
        const QRect viewport(inspector->scrollArea()->viewport()->mapTo(inspector, QPoint()), inspector->scrollArea()->viewport()->size());
        for (const char *section : {"crop", "anchor", "linkedAudio", "masks", "effects", "source"}) {
            InspectorSection *found = inspector->section(QString::fromLatin1(section));
            QVERIFY2(found, section);
            const QRect header(found->header()->mapTo(inspector, QPoint()), found->header()->size());
            QVERIFY2(viewport.contains(header), section);
        }
        // The same in a 16:9 project (the laptop): one stage row for every
        // shape, so the column is as tall and nothing scrolls either.
        {
            QJsonObject wide = everyKindSnapshot(2);
            QJsonObject timeline = wide.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1920);
            timeline.insert(QStringLiteral("height"), 1080);
            wide.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(wide);
            QCoreApplication::processEvents();
            QCOMPARE(window.m_viewer->device(), ViewerWidget::Device::Laptop);
            QVERIFY2(scroll->maximum() == 0, qPrintable(QStringLiteral("16:9 scrolls %1 in %2").arg(scroll->maximum()).arg(inspector->height())));
            InspectorSection *source = inspector->section(QStringLiteral("source"));
            QVERIFY(source);
            QVERIFY(viewport.contains(QRect(source->header()->mapTo(inspector, QPoint()), source->header()->size())));
            unclipped(QStringLiteral("16:9 column"));
        }
        // In a 1440 × 900 window the open Transform and Composite modules
        // still show whole, unscrolled.
        {
            window.resize(1440, 900);
            QCoreApplication::processEvents();
            const QRect small(inspector->scrollArea()->viewport()->mapTo(inspector, QPoint()), inspector->scrollArea()->viewport()->size());
            QCOMPARE(scroll->value(), 0);
            for (const char *id : {"transform", "compositing"}) {
                InspectorSection *module = inspector->section(QString::fromLatin1(id));
                QVERIFY2(module && module->isOpen(), id);
                const QRect plate(module->mapTo(inspector, QPoint()), module->size() - QSize(0, InspectorSection::kGap));
                QVERIFY2(small.contains(plate), qPrintable(QStringLiteral("%1 at %2 in %3").arg(QString::fromLatin1(id)).arg(plate.bottom()).arg(small.bottom())));
            }
            window.resize(1600, 1000);
            QCoreApplication::processEvents();
        }
        // Every row lines its controls up after one label column, and ends
        // at one edge: the reset lane's, 24 points inside the column's.
        int controlsRight = -1;
        for (InspectorRow *row : inspector->findChildren<InspectorRow *>()) {
            QCOMPARE(row->label()->width(), InspectorRow::kLabelWidth);
            QCOMPARE(row->height(), InspectorRow::kHeight);
            if (!row->isVisibleTo(inspector)) continue;
            int right = -1;
            for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly))
                if (child != row->label() && child->isVisibleTo(row) && child->width() > InspectorRow::kResetLane &&
                    child->sizePolicy().horizontalPolicy() != QSizePolicy::Fixed)
                    right = std::max(right, child->geometry().right());
            // One value field is the column's one width; two (X and Y, L and
            // R) share the row equally, 78 + 8 + 78 at a 300-point column.
            QList<ScrubField *> fields;
            for (ScrubField *field : row->findChildren<ScrubField *>(QString(), Qt::FindDirectChildrenOnly))
                if (field->isVisibleTo(row)) fields << field;
            if (fields.size() == 2) {
                // Within a point: the 303-point column splits unevenly.
                QVERIFY(std::abs(fields.at(0)->width() - fields.at(1)->width()) <= 1);
                QVERIFY(fields.at(0)->width() >= InspectorRow::kValueField);
            } else {
                for (ScrubField *field : fields) QCOMPARE(field->width(), InspectorRow::kValueField);
            }
            if (right < 0) continue;
            if (controlsRight < 0) controlsRight = right;
            QVERIFY2(right <= controlsRight, qPrintable(row->label()->text()));
        }
        QVERIFY(controlsRight > 0);
        QVERIFY(controlsRight <= inspector->width() - 16 - 24 + 1);
        // At the narrower 300-point column, still nothing is cut off.
        auto *alone = new InspectorPanel;
        alone->setEffectCatalog(QJsonArray{});
        alone->setMaskable(true);
        alone->setMaskCatalog(QJsonArray{QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0}, {"min", 0}, {"max", 10000}, {"unit", "pixels"}}},
                              {QStringLiteral("add")});
        QJsonObject timeline = everyKindSnapshot(1).value("timeline").toObject();
        alone->setState(timeline, {QStringLiteral("v")});
        alone->resize(300, 560);
        alone->show();
        QVERIFY(QTest::qWaitForWindowExposed(alone));
        for (InspectorSection *section : alone->findChildren<InspectorSection *>()) section->setOpen(true);
        QCoreApplication::processEvents();
        const auto check = [alone](const QString &what) {
            for (InspectorRow *row : alone->findChildren<InspectorRow *>()) {
                if (!row->isVisibleTo(alone)) continue;
                for (QWidget *child : row->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly)) {
                    if (!child->isVisibleTo(row)) continue;
                    QVERIFY2(child->geometry().right() < row->width(), qPrintable(QStringLiteral("%1: %2").arg(what, row->label()->text())));
                }
                QVERIFY2(row->mapTo(alone, QPoint(row->width(), 0)).x() <= alone->width() - 16,
                         qPrintable(QStringLiteral("%1 %2: right %3, min %4").arg(what, row->label()->text())
                                        .arg(row->mapTo(alone, QPoint(row->width(), 0)).x()).arg(row->minimumSizeHint().width())));
            }
        };
        check(QStringLiteral("300"));
        // Nothing selected: the frame size reads whole in its pop-up, clear
        // of its ⌃⌄.
        alone->setState(timeline, {});
        QCoreApplication::processEvents();
        {
            auto *size = alone->findChild<QComboBox *>(QStringLiteral("frameSize"));
            QVERIFY(size && size->isVisible());
            QVERIFY2(QFontMetrics(size->font()).horizontalAdvance(size->currentText()) + 28 <= size->width(),
                     qPrintable(QStringLiteral("%1 in %2").arg(size->currentText()).arg(size->width())));
        }
        for (const char *id : {"t", "music", "adj"}) {
            alone->setState(timeline, {QString::fromLatin1(id)});
            for (InspectorSection *section : alone->findChildren<InspectorSection *>()) section->setOpen(true);
            QCoreApplication::processEvents();
            check(QString::fromLatin1(id));
        }
        delete alone;
    }

    // The structure of the review design (mockup-spec 0.1) at 1600 x 1000:
    // the toolbar 40; Program 1296 wide, its strip 28 over a 564 stage; the
    // Inspector column 304; the band 344 of the bin 272, the timeline 1024
    // and the browser 304; the status bar 24; each region counting its 1 pt
    // seam, all to within a point. At 1440 x 900 and 1920 x 1080 the columns
    // and the band keep their sizes and the Program monitor and the
    // timeline take the rest. Then the title, the format bank, the
    // timeline's header and the selection on the status bar.
    void editorTakesTheChosenStructure() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const auto clip = [](const char *id, const char *source, double start, double duration) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", source}, {"timelineStart", start}, {"duration", duration}};
        };
        QJsonArray tracks;
        for (int index = 1; index <= 4; ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("V%1").arg(index)}, {"type", "video"},
                                      {"clips", index == 1 ? QJsonArray{clip("v1a", "wide", 0.0, 6.0), clip("v1b", "close", 6.0, 5.0)} : QJsonArray{}}});
        tracks.append(QJsonObject{{"id", "S1"}, {"type", "subtitle"}, {"name", "Captions"}, {"clips", QJsonArray{}}});
        for (int index = 1; index <= 4; ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("A%1").arg(index)}, {"type", "audio"}, {"clips", QJsonArray{}}});
        const QJsonArray sources{
            QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}},
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"width", 1080}, {"height", 1920}, {"duration", 20}},
            QJsonObject{{"id", "drone"}, {"path", "media/drone-pass.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 20}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}}};
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 18}, {"timeline", QJsonObject{
            {"name", "Harbor Teaser"}, {"fps", 24}, {"width", 1920}, {"height", 1080}, {"sources", sources}, {"tracks", tracks}}}});
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
        fakeArtwork(window.m_artwork);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        QCoreApplication::processEvents();
        namespace layout = tokens::layout;
        Workspace *workspace = window.m_workspace;
        const auto frame = [&window](const QWidget *widget) { return QRect(widget->mapTo(&window, QPoint()), widget->size()); };
        const auto area = [&](const char *id) { return frame(workspace->panel(QString::fromLatin1(id))->dockAreaWidget()); };
        // A region with its seam: the 1 pt splitter line before it counts as its own.
        const auto near = [](int actual, int expected) { return std::abs(actual - expected) <= 1; };
        const auto check = [&](const char *what) {
            QCoreApplication::processEvents();
            const int width = window.width();
            const int height = window.height();
            const QRect toolbar = frame(window.m_toolbar);
            const QRect status = frame(window.statusBar());
            const QRect program = area("program");
            const QRect inspector = area("inspector");
            const QRect bin = area("project");
            const QRect timeline = area("timeline");
            const QRect browser = area("browser");
            const QString got = QStringLiteral("%1: toolbar %2 status %3 program %4,%5 %6x%7 inspector %8 %9x%10 bin %11,%12 %13x%14 timeline %15 %16 browser %17 %18")
                                    .arg(QString::fromLatin1(what)).arg(toolbar.height()).arg(status.height())
                                    .arg(program.x()).arg(program.y()).arg(program.width()).arg(program.height())
                                    .arg(inspector.x()).arg(inspector.width()).arg(inspector.height())
                                    .arg(bin.x()).arg(bin.y()).arg(bin.width()).arg(bin.height())
                                    .arg(timeline.x()).arg(timeline.width()).arg(browser.x()).arg(browser.width());
            QVERIFY2(toolbar.top() == 0 && toolbar.height() == layout::toolbar, qPrintable(got));
            QVERIFY2(status.bottom() == height - 1 && status.height() == layout::status, qPrintable(got));
            // The top row: Program, then the Inspector column to the edge.
            QVERIFY2(program.left() == 0 && program.top() == layout::toolbar, qPrintable(got));
            QVERIFY2(near(program.width(), width - layout::inspector), qPrintable(got));
            QVERIFY2(near(inspector.left() - 1, width - layout::inspector) && inspector.right() == width - 1, qPrintable(got));
            QVERIFY2(near(inspector.width() + 1, layout::inspector) && inspector.top() == program.top(), qPrintable(got));
            QVERIFY2(near(program.height(), height - layout::toolbar - layout::status - layout::band), qPrintable(got));
            QVERIFY2(inspector.height() == program.height(), qPrintable(got));
            // The Program monitor's strip and stage.
            const QRect strip = frame(window.m_programStrip);
            const QRect stage = frame(window.m_viewer);
            QVERIFY2(strip.top() == program.top() && strip.height() == layout::strip && strip.width() == program.width(), qPrintable(got));
            QVERIFY2(stage.top() == strip.bottom() + 1 && stage.bottom() == program.bottom(), qPrintable(got));
            // The band: the bin, the timeline, the browser, under the row.
            QVERIFY2(near(bin.top() - 1, program.bottom() + 1) && bin.bottom() == status.top() - 1, qPrintable(got));
            QVERIFY2(near(bin.height() + 1, layout::band), qPrintable(got));
            QVERIFY2(bin.left() == 0 && near(bin.width() + 1, layout::bin), qPrintable(got));
            QVERIFY2(near(timeline.left(), layout::bin) && near(timeline.width(), width - layout::bin - layout::browser), qPrintable(got));
            QVERIFY2(near(browser.left() - 1, width - layout::browser) && browser.right() == width - 1, qPrintable(got));
            QVERIFY2(timeline.top() == bin.top() && browser.top() == bin.top() && timeline.height() == bin.height(), qPrintable(got));
            // No gaps: each seam is the one point between.
            QVERIFY2(inspector.left() - program.right() == 2 && timeline.left() - bin.right() == 2 && browser.left() - timeline.right() == 2,
                     qPrintable(got));
            QVERIFY2(bin.top() - program.bottom() == 2, qPrintable(got));
        };
        check("1600x1000");
        QCOMPARE(window.m_viewer->height(), 564);
        QCOMPARE(area("program").width(), 1296);
        QCOMPARE(area("timeline").width(), 1024);
        // The regions that draw their own strip show no tab row over it.
        for (const char *id : {"program", "inspector", "browser"})
            QVERIFY2(workspace->panel(QString::fromLatin1(id))->dockAreaWidget()->titleBar()->isHidden(), id);
        window.resize(1440, 900);
        check("1440x900");
        window.resize(1920, 1080);
        check("1920x1080");
        window.resize(1600, 1000);
        check("back to 1600x1000");

        // The title, and under it the project's state, centred together.
        QCOMPARE(window.m_projectTitle->text(), QStringLiteral("Harbor Teaser"));
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Saved · 16:9 · 24 fps"));
        QVERIFY(window.m_projectSubtitle->geometry().top() >= window.m_projectTitle->geometry().bottom());
        QCOMPARE(window.m_projectSubtitle->geometry().center().x(), window.m_projectTitle->geometry().center().x());
        QJsonObject edited = sharedStatus();
        edited.insert(QStringLiteral("changed"), true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", edited}});
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · 16:9 · 24 fps"));
        // The toolbar's format bank names the format as the subtitle does.
        {
            const QString times = QStringLiteral(" × ");
            QCOMPARE(window.m_formatBank->toolTip(), QStringLiteral("16:9 · 1920") + times + QStringLiteral("1080"));
            window.syncCanvasPreset(QSize(1080, 1920));
            QCOMPARE(window.m_formatBank->toolTip(), QStringLiteral("9:16 · 1080") + times + QStringLiteral("1920"));
            QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · 9:16 · 24 fps"));
            window.syncCanvasPreset(QSize(1920, 1080));
            QCOMPARE(window.m_formatBank->currentIndex(), 0);
        }
        // The Program strip: its name, the mask tools and the viewer's
        // switches as the registry's actions (the ones still to come say so).
        ProgramStrip *strip = window.m_programStrip;
        QVERIFY(strip->buttonFor(window.m_maskTools.at(0)));
        QAction *safe = window.m_commands->action(QStringLiteral("view.safeZones"));
        QVERIFY(strip->buttonFor(safe));
        QCOMPARE(safe->shortcut(), QKeySequence(Qt::Key_Apostrophe));
        QAction *dim = window.m_commands->action(QStringLiteral("view.dim"));
        QVERIFY(strip->buttonFor(dim) && !dim->isEnabled());
        QVERIFY(dim->toolTip().contains(QStringLiteral("Coming in this release")));
        QVERIFY(strip->buttonFor(window.m_commands->action(QStringLiteral("viewer.zoom"))));
        QVERIFY(!strip->previewShown());
        // The timeline's header: after its name, how many tracks and how
        // long; on the right, the zoom beside the track heights.
        QCOMPARE(window.m_timelineDetail->text(), QStringLiteral("9 tracks  ·  00:00:18:00"));
        QVERIFY(window.m_timelineDetail->isVisible() && window.m_timelineZoom->isVisible());
        ads::CDockAreaTitleBar *header = workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->titleBar();
        QCOMPARE(window.m_timelineDetail->parentWidget(), header);
        QCOMPARE(header->indexOf(window.m_timelineDetail), header->indexOf(header->tabBar()) + 1);
        QVERIFY(window.m_timelineZoom->x() > header->width() / 2);
        // The selection on the status bar's left: a click, then ⌘-click adds.
        QVERIFY(window.m_selectionStatus->text().isEmpty());
        window.clickSelectClip(QStringLiteral("v1a"));
        QCOMPARE(window.m_selectionStatus->text(), QStringLiteral("1 clip selected  ·  00:00:06:00"));
        window.m_timeline->fitToWidth(window.m_timelineScroll->viewport()->width());
        QCoreApplication::processEvents();
        const QRect other = window.m_timeline->clipBounds(QStringLiteral("v1b"));
        QVERIFY(!other.isEmpty());
        QTest::mouseClick(window.m_timeline, Qt::LeftButton, Qt::ControlModifier, other.center());
        QCOMPARE(window.m_selectedClips, (QStringList{QStringLiteral("v1a"), QStringLiteral("v1b")}));
        QCOMPARE(window.m_bus->selection(), window.m_selectedClips);
        QCOMPARE(window.m_selectionStatus->text(), QStringLiteral("2 clips selected  ·  00:00:11:00"));
        QCOMPARE(window.m_inspector->selection(), window.m_selectedClips);
        QTest::mouseClick(window.m_timeline, Qt::LeftButton, Qt::ControlModifier, other.center());
        QCOMPARE(window.m_selectedClips, QStringList{QStringLiteral("v1a")});
        window.clickSelectClip(QString());
        QVERIFY(window.m_selectionStatus->text().isEmpty());
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::WorkspaceTest)

#include "workspace_test.moc"
