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

    void workspacesDockPanelsAndRememberLayouts() {
        QSettings().remove(QStringLiteral("workspace"));
        {
            MainWindow window{QString(), QString()};
            window.show();
            Workspace *workspace = window.m_workspace;
            ads::CDockWidget *timeline = workspace->panel(QStringLiteral("timeline"));
            ads::CDockWidget *project = workspace->panel(QStringLiteral("project"));
            QAction *toggle = window.m_commands->action(QStringLiteral("window.timeline"));
            QVERIFY(toggle->isChecked());
            QVERIFY(!timeline->isClosed());
            toggle->trigger();
            QVERIFY(timeline->isClosed());
            // Reopening a panel from its own controls keeps the menu in step.
            timeline->toggleView(true);
            QVERIFY(toggle->isChecked());

            QCOMPARE(workspace->names(), QStringList{QStringLiteral("Editing")});
            // Kerf's conversation starts open, in the Program monitor's stage.
            QVERIFY(window.chatOpen());
            QVERIFY(workspace->saveAs(QStringLiteral("Review")));
            QVERIFY(!workspace->saveAs(QStringLiteral("review")));
            QCOMPARE(workspace->current(), QStringLiteral("Review"));
            project->toggleView(false);
            workspace->activate(QStringLiteral("Editing"));
            QVERIFY(!project->isClosed());
            // Each workspace keeps its changes until it is reset.
            workspace->activate(QStringLiteral("Review"));
            QVERIFY(project->isClosed());
            QVERIFY(!window.m_commands->action(QStringLiteral("window.project"))->isChecked());
            workspace->resetCurrent();
            QVERIFY(!project->isClosed());

            window.m_commands->action(QStringLiteral("app.preferences"))->trigger();
            QVERIFY(window.m_preferences->isVisible());
        }
        QSettings().remove(QStringLiteral("workspace"));
    }

    // The Editing workspace reads as a Short does on YouTube's desktop page:
    // the Program monitor is the page, the widest panel of its row, and in
    // its stage the picture card, the action rail and Kerf's conversation
    // form one group, centred, the conversation as tall as the card and
    // level with it. The browsers and Effect Controls sit on its left, the
    // Source monitor is a tab behind it, and the Timeline runs the window's
    // full width under them.
    void editingShowsTheVideoWithItsConversationInItsStage() {
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
        QCOMPARE(area("source"), area("program"));
        QVERIFY(panel("program")->isCurrentTab());
        for (const char *id : {"inspector", "effects", "presets"}) QCOMPARE(area(id), area("project"));
        QVERIFY(panel("project")->isCurrentTab());
        // Their column shows the four tabs' names in full.
        for (const char *id : {"project", "inspector", "effects", "presets"}) {
            auto *label = panel(id)->tabWidget()->findChild<ads::CElidingLabel *>();
            QVERIFY2(label && label->isVisible() && !label->isElided(), id);
        }
        // Every panel is open, and Window lists each one.
        for (const char *id : {"project", "source", "program", "timeline", "inspector", "effects", "presets"}) {
            QVERIFY2(!panel(id)->isClosed(), id);
            QAction *item = window.m_commands->action(QStringLiteral("window.%1").arg(QString::fromLatin1(id)));
            QVERIFY2(item && item->isCheckable() && item->isChecked(), id);
        }

        const QRect browser = frame(area("project"));
        const QRect viewer = frame(area("program"));
        const QRect timeline = frame(area("timeline"));
        // One row: the browsers, then the viewer to the window's edge.
        QVERIFY(browser.right() < viewer.left());
        QCOMPARE(viewer.right(), window.width() - 1);
        QCOMPARE(browser.top(), viewer.top());
        QVERIFY(viewer.width() > 2 * browser.width());
        QCOMPARE(timeline.left(), browser.left());
        QCOMPARE(timeline.right(), viewer.right());
        QVERIFY(timeline.top() > viewer.bottom());
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
        QCOMPARE(frame(area("project")).width(), browser.width());
        QCOMPARE(frame(area("program")).width(), viewer.width() + 240);
        QVERIFY(window.m_chat->width() > chatWidth && window.m_chat->width() <= ViewerWidget::kDefaultChatWidth);
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
            const double cardArea = stage.card.width() * stage.card.height();
            const double panelArea = double(stage.panel.width()) * stage.panel.height();
            QVERIFY2(cardArea >= panelArea, qPrintable(QStringLiteral("%1 < %2").arg(cardArea).arg(panelArea)));
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
        QCOMPARE(frame(area("project")).width(), browser.width());
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
        // With the Source monitor in front, opening the conversation brings
        // the Program monitor, whose stage holds it, forward.
        panel("source")->setAsCurrentTab();
        QCoreApplication::processEvents();
        QVERIFY(!window.m_viewer->isVisible());
        toggle->trigger();
        QCoreApplication::processEvents();
        QVERIFY(panel("program")->isCurrentTab() && window.m_chat->isVisible());
        // Open but out of sight behind the Source monitor, it reads as
        // closed: the timeline's bar stands in for it with Kerf's proposal,
        // and the first press of ⇧⌘J brings it back into sight.
        window.setConnection(QStringLiteral("ready"));
        window.updateProposal(QJsonObject{{"status", "pending"}, {"proposalId", "p1"}, {"script", "remove v1c\n"},
                                          {"documentDiff", "-    clip v1c\n"}});
        panel("source")->setAsCurrentTab();
        QCoreApplication::processEvents();
        QVERIFY(window.m_viewer->chatOpen() && !window.chatOpen());
        QVERIFY(!toggle->isChecked() && !window.m_chatToggle->isChecked());
        QVERIFY(window.m_aiBar->isVisible());
        QVERIFY(window.m_aiBarText->text().startsWith(QStringLiteral("Proposal")));
        QTest::keyClick(&window, Qt::Key_J, Qt::ControlModifier | Qt::ShiftModifier);
        QCoreApplication::processEvents();
        QVERIFY(panel("program")->isCurrentTab() && window.chatOpen() && window.m_chat->isVisible());
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

    // The new Editing arrangement takes the place of the Editing layout
    // saved before it, while a workspace of one's own saved then keeps its
    // arrangement, even one saved with the conversation's old dock (which
    // restoring leaves out); one from before today's panels gives way to
    // Editing's.
    void editingLayoutReplacesOldDefaultsAndKeepsOwnWorkspaces() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray before;
        QByteArray older;
        QByteArray docked;
        {
            MainWindow window{QString(), QString()};
            window.m_workspace->panel(QStringLiteral("presets"))->toggleView(false);
            before = window.m_workspace->manager()->saveState(5);
            older = window.m_workspace->manager()->saveState(3);
            // A layout of version 5 with the conversation docked as its own
            // panel (in place of the Inspector, in the browsers' column).
            docked = window.m_workspace->manager()->saveState(5);
            if (!docked.startsWith("<?xml")) docked = qUncompress(docked);
            QVERIFY(docked.contains("Name=\"inspector\""));
            docked.replace("Name=\"inspector\"", "Name=\"assistant\"");
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Review"), QStringLiteral("Old"), QStringLiteral("Docked")});
            settings.setValue(QStringLiteral("workspace/saved/Editing"), before);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), before);
            settings.setValue(QStringLiteral("workspace/saved/Review"), before);
            settings.setValue(QStringLiteral("workspace/saved/Old"), older);
            settings.setValue(QStringLiteral("workspace/saved/Docked"), docked);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        ads::CDockWidget *presets = workspace->panel(QStringLiteral("presets"));
        QVERIFY(!presets->isClosed());
        workspace->activate(QStringLiteral("Review"));
        QCOMPARE(workspace->current(), QStringLiteral("Review"));
        QVERIFY(presets->isClosed());
        QVERIFY(!window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        workspace->activate(QStringLiteral("Old"));
        QVERIFY(!presets->isClosed());
        workspace->activate(QStringLiteral("Docked"));
        QCOMPARE(workspace->current(), QStringLiteral("Docked"));
        QVERIFY(presets->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("program"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("assistant")));
        // The conversation is the stage's, whatever the workspace.
        QVERIFY(window.chatOpen());
    }

    // Add effect raises the Effects browser with its search ready; adding
    // one brings the Inspector back with it open. Dropping an effect on the
    // Inspector inserts it where it lands.
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
        QVERIFY(workspace->panel(QStringLiteral("inspector"))->isCurrentTab());
        InspectorPanel *inspector = window.m_inspector;
        inspector->section(QStringLiteral("effects"))->setOpen(true);
        QCoreApplication::processEvents();
        auto *add = inspector->findChild<QPushButton *>(QStringLiteral("addEffect"));
        QVERIFY(add && add->isVisible());
        QCOMPARE(add->text(), QStringLiteral("Add effect"));
        add->click();
        QVERIFY(workspace->panel(QStringLiteral("effects"))->isCurrentTab());
        QVERIFY(window.m_effects->m_search->hasFocus() || QApplication::focusWidget() == window.m_effects->m_search);
        // Adding: one fx-add, and the Inspector is back.
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        const QJsonObject added = takeOps(window).first().toObject();
        QCOMPARE(added.value("op").toString(), QStringLiteral("fx-add"));
        QCOMPARE(added.value("fx").toString(), QStringLiteral("blur.gaussian"));
        QVERIFY(workspace->panel(QStringLiteral("inspector"))->isCurrentTab());
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
        workspace->panel(QStringLiteral("inspector"))->setAsCurrentTab();
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

    // Clicking a clip brings the Inspector to the front of its column and
    // remembers the tab it came from; clearing the selection goes back to
    // it, unless the Inspector was chosen by hand. A clip selected by an
    // edit raises nothing, and a closed column stays closed. The
    // conversation is never touched, and keyboard focus stays put.
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
        ads::CDockWidget *inspector = workspace->panel(QStringLiteral("inspector"));
        ads::CDockWidget *project = workspace->panel(QStringLiteral("project"));
        QVERIFY(project->isCurrentTab());
        const QRect chat = window.m_chat->geometry();
        QVERIFY(window.m_chat->isVisible());
        window.m_timeline->setFocus(Qt::OtherFocusReason);
        QCoreApplication::processEvents();
        QWidget *focus = QApplication::focusWidget();
        // A click on a clip: the Inspector, showing it; focus stays.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        QCOMPARE(window.m_inspector->nameText(), QStringLiteral("fisher-close"));
        QCOMPARE(QApplication::focusWidget(), focus);
        // Another clip: only the values change.
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        // Deselected (an empty lane, Esc): back to Project.
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        QVERIFY(window.m_selectedClip.isEmpty());
        // A selection by an edit raises nothing.
        window.selectClip(QStringLiteral("v1b"));
        QVERIFY(project->isCurrentTab());
        window.selectClip(QString());
        // Project chosen while a clip is selected stays until a new clip is clicked.
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isCurrentTab());
        project->setAsCurrentTab();
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QVERIFY(project->isCurrentTab());
        // ⌘4: the Inspector by hand, which stays when the selection clears
        // and shows the sequence; ⌘4 again goes back.
        QAction *show = window.m_commands->action(QStringLiteral("window.inspector"));
        show->trigger();
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        QVERIFY(show->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        emit window.m_timeline->deselectRequested();
        QVERIFY(inspector->isCurrentTab());
        QCOMPARE(window.m_inspector->m_mode, InspectorPanel::Mode::Sequence);
        show->trigger();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        QVERIFY(!inspector->isClosed() && show->isChecked());
        // Following off: clicks leave the column alone, and it is remembered.
        QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
        QVERIFY(follows->isCheckable() && follows->isChecked());
        follows->trigger();
        QVERIFY(!QSettings().value(QStringLiteral("editor/inspectorFollowsSelection")).toBool());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(project->isCurrentTab());
        follows->trigger();
        QVERIFY(Workspace::inspectorFollowsSelection());
        emit window.m_timeline->deselectRequested();
        // A closed Inspector is not reopened by a click.
        inspector->toggleView(false);
        QVERIFY(!window.m_commands->action(QStringLiteral("window.inspector"))->isChecked());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isClosed());
        // ⌘4 opens it again.
        show->trigger();
        QVERIFY(!inspector->isClosed() && inspector->isCurrentTab());
        // Effects or Presets chosen by hand stay in front when another clip
        // is clicked (looks applied clip by clip), and the browser adds to
        // the clip just clicked.
        ads::CDockWidget *effects = workspace->panel(QStringLiteral("effects"));
        ads::CDockWidget *presets = workspace->panel(QStringLiteral("presets"));
        QVERIFY(effects && presets);
        QCOMPARE(effects->dockAreaWidget(), inspector->dockAreaWidget());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(inspector->isCurrentTab());
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        QCOMPARE(window.m_inspector->selection(), QStringList{QStringLiteral("v1a")});
        takeOps(window);
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        {
            const QJsonArray added = takeOps(window);
            QCOMPARE(added.size(), 1);
            QCOMPARE(added.at(0).toObject().value("op").toString(), QStringLiteral("fx-add"));
            QCOMPARE(added.at(0).toObject().value("clip").toString(), QStringLiteral("v1a"));
        }
        QVERIFY(effects->isCurrentTab());
        presets->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QVERIFY(presets->isCurrentTab());
        // From Project a click still raises it; the Inspector's own Add
        // effect brings Effects up for it, and the next click brings the
        // Inspector back.
        project->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(inspector->isCurrentTab());
        emit window.m_inspector->addEffectRequested(false);
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // Add effect, an effect added (the Inspector comes back), then a
        // deselect: back to Project, not Effects; the next click raises
        // the Inspector again.
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        emit window.m_inspector->addEffectRequested(false);
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_effects->addRequested(QStringLiteral("blur.gaussian"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // It still knows the clip it shows: Project chosen by hand stays
        // when that same clip is clicked again (to drag or trim it); a new
        // clip brings the Inspector back.
        project->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(project->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // Effects chosen by hand stays through a deselect and the next
        // click; Project then gives the click back to the Inspector.
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->deselectRequested();
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        emit window.m_timeline->clipSelected(QStringLiteral("v1b"));
        QCoreApplication::processEvents();
        QVERIFY(effects->isCurrentTab());
        project->setAsCurrentTab();
        emit window.m_timeline->clipSelected(QStringLiteral("v1a"));
        QCoreApplication::processEvents();
        QVERIFY(inspector->isCurrentTab());
        // ⌘4 still shows it over a browser chosen by hand.
        effects->setAsCurrentTab();
        QCoreApplication::processEvents();
        show->trigger();
        QVERIFY(inspector->isCurrentTab());
        // The conversation never moved.
        QVERIFY(window.m_chat->isVisible());
        QCOMPARE(window.m_chat->geometry(), chat);
    }

    // ⌘4 is Window ▸ Inspector (Effect Controls' old command still finds
    // it); ⌥⌘4 is Inspector Follows Selection. Workspaces saved with Effect
    // Controls restore with the Inspector where it was.
    void inspectorCommandsAndLayoutMigration() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QByteArray old;
        {
            MainWindow window{QString(), QString()};
            QAction *inspector = window.m_commands->action(QStringLiteral("window.inspector"));
            QVERIFY(inspector && inspector->isCheckable());
            QCOMPARE(inspector->text(), QStringLiteral("Inspector"));
            QCOMPARE(inspector->shortcut(), QKeySequence(Qt::CTRL | Qt::Key_4));
            QCOMPARE(window.m_commands->action(QStringLiteral("window.effectControls")), inspector);
            QCOMPARE(CommandRegistry::resolve(QStringLiteral("window.effectControls")), QStringLiteral("window.inspector"));
            QAction *follows = window.m_commands->action(QStringLiteral("window.inspectorFollows"));
            QVERIFY(follows && follows->isCheckable());
            QCOMPARE(follows->shortcut(), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_4));
            // Its tab is second in the column, named Inspector.
            ads::CDockAreaWidget *area = window.m_workspace->panel(QStringLiteral("project"))->dockAreaWidget();
            QCOMPARE(area->dockWidget(1)->objectName(), QStringLiteral("inspector"));
            QCOMPARE(area->dockWidget(1)->windowTitle(), QStringLiteral("Inspector"));
            // A version-6 layout, with the Inspector by its old id and Presets closed.
            window.m_workspace->panel(QStringLiteral("presets"))->toggleView(false);
            old = window.m_workspace->manager()->saveState(6);
            if (!old.startsWith("<?xml")) old = qUncompress(old);
            QVERIFY(old.contains("Name=\"inspector\""));
            old.replace("Name=\"inspector\"", "Name=\"effectControls\"");
        }
        {
            QSettings settings;
            settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("Grading")});
            settings.setValue(QStringLiteral("workspace/saved/Grading"), old);
            settings.setValue(QStringLiteral("workspace/changes/Editing"), old);
            settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Editing"));
        }
        MainWindow window{QString(), QString()};
        Workspace *workspace = window.m_workspace;
        // Editing's own changes from version 6 give way to the new
        // proportions (version 8: the 300 column over a 290 timeline), with
        // the Inspector open in the column; a workspace of one's own keeps
        // its arrangement, the Inspector renamed.
        QVERIFY(!workspace->panel(QStringLiteral("presets"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("inspector"))->isClosed());
        QCOMPARE(workspace->panel(QStringLiteral("inspector"))->dockAreaWidget(), workspace->panel(QStringLiteral("project"))->dockAreaWidget());
        workspace->resetCurrent();
        workspace->activate(QStringLiteral("Grading"));
        QCOMPARE(workspace->current(), QStringLiteral("Grading"));
        QVERIFY(workspace->panel(QStringLiteral("presets"))->isClosed());
        QVERIFY(!workspace->panel(QStringLiteral("inspector"))->isClosed());
        QCOMPARE(workspace->panel(QStringLiteral("inspector"))->dockAreaWidget(), workspace->panel(QStringLiteral("project"))->dockAreaWidget());
    }

    // At 1440x900 the Inspector fits its column: no row wider than the
    // column, and a video clip's identity, its primary rows and every
    // folded section header in view without scrolling.
    // Dragged all the way up, the timeline's split closes the row over it
    // altogether instead of stopping at a band of cut-off panels, and comes
    // back when dragged down; the timeline itself never closes. Short of
    // that, each dock area stops at its panels' least size, so nothing in
    // it is cut off, and the column of Project, Inspector, Effects and
    // Presets keeps one least width (no shorter than its tabs' names)
    // whichever is chosen.
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
        ads::CDockAreaWidget *browser = workspace->panel(QStringLiteral("project"))->dockAreaWidget();
        ads::CDockAreaWidget *viewer = workspace->panel(QStringLiteral("program"))->dockAreaWidget();
        auto *rows = qobject_cast<QSplitter *>(timeline->parentWidget());
        QVERIFY(rows && rows->orientation() == Qt::Vertical && rows->count() == 2);
        const int timelineIndex = rows->indexOf(timeline);
        const int topIndex = 1 - timelineIndex;
        QVERIFY(rows->isCollapsible(topIndex) && !rows->isCollapsible(timelineIndex));
        // Each area's least size is its panels' largest.
        for (const char *id : {"project", "inspector", "effects", "presets", "program", "source"}) {
            ads::CDockWidget *dock = workspace->panel(QString::fromLatin1(id));
            const QSize least = dock->dockAreaWidget()->minimumSizeHint();
            QVERIFY2(least.width() >= dock->widget()->minimumSizeHint().width() && least.height() >= dock->widget()->minimumSizeHint().height(),
                     id);
        }
        QVERIFY(browser->minimumSizeHint().width() >= browser->titleBar()->tabBar()->sizeHint().width());
        QVERIFY(timeline->minimumSizeHint().height() >= workspace->panel(QStringLiteral("timeline"))->widget()->minimumHeight());
        // The Editing arrangement holds the timeline's least height.
        QVERIFY(timeline->height() >= timeline->minimumSizeHint().height());
        // Choosing another tab leaves the column's width alone.
        const int browserWidth = browser->width();
        for (const char *id : {"inspector", "effects", "presets", "project"}) {
            workspace->panel(QString::fromLatin1(id))->setAsCurrentTab();
            QCoreApplication::processEvents();
            QCOMPARE(browser->width(), browserWidth);
        }
        // Part way up: the row stops at its least height, its panels whole.
        const QList<int> arranged = rows->sizes();
        const int total = arranged.at(0) + arranged.at(1);
        QList<int> partWay(2);
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
        rows = qobject_cast<QSplitter *>(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->parentWidget());
        QVERIFY(rows && rows->childrenCollapsible());
        QVERIFY(!rows->isCollapsible(rows->indexOf(workspace->panel(QStringLiteral("timeline"))->dockAreaWidget())));
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
        window.resize(1440, 900);
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
        // On a 1366 × 768 screen the open Transform and Composite modules
        // still show whole, unscrolled.
        {
            window.resize(1366, 768);
            QCoreApplication::processEvents();
            const QRect small(inspector->scrollArea()->viewport()->mapTo(inspector, QPoint()), inspector->scrollArea()->viewport()->size());
            QCOMPARE(scroll->value(), 0);
            for (const char *id : {"transform", "compositing"}) {
                InspectorSection *module = inspector->section(QString::fromLatin1(id));
                QVERIFY2(module && module->isOpen(), id);
                const QRect plate(module->mapTo(inspector, QPoint()), module->size() - QSize(0, InspectorSection::kGap));
                QVERIFY2(small.contains(plate), qPrintable(QStringLiteral("%1 at %2 in %3").arg(QString::fromLatin1(id)).arg(plate.bottom()).arg(small.bottom())));
            }
            window.resize(1440, 900);
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
                QCOMPARE(fields.at(0)->width(), fields.at(1)->width());
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

    // The structure the owner chose (SPEC-R10 part D): a title with a quiet
    // subtitle of the project's state; text tabs with an underline; a 300
    // column holding "Search media" over a two-column grid of tiles; a stage
    // row over a 290 timeline that shows eight tracks at 1440x900 and six at
    // 1280x800, its header saying how many tracks and how long, with a zoom;
    // and the selection on the status bar's left, ⌘-click adding to it.
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
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.m_errorBanner->hide();
        QCoreApplication::processEvents();
        const EditorTheme &theme = editorTheme();

        // The title, and under it the project's state, centred together.
        QCOMPARE(window.m_projectTitle->text(), QStringLiteral("Harbor Teaser"));
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Saved · 16:9 · 24 fps"));
        QVERIFY(window.m_projectSubtitle->geometry().top() >= window.m_projectTitle->geometry().bottom());
        QCOMPARE(window.m_projectSubtitle->geometry().center().x(), window.m_projectTitle->geometry().center().x());
        QJsonObject edited = sharedStatus();
        edited.insert(QStringLiteral("changed"), true);
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", edited}});
        QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · 16:9 · 24 fps"));
        // The toolbar's format bank names the format as the subtitle and
        // the Inspector's Frame size do: "1:1 · 1080 × 1080", one separator,
        // never "Custom" for a preset, never two spaces in a row; a custom
        // size latches no key.
        {
            const QString times = QStringLiteral("\u2009×\u2009");
            QCOMPARE(window.m_formatBank->toolTip(), QStringLiteral("16:9 · 1920") + times + QStringLiteral("1080"));
            const struct {
                QSize size;
                QString ratio;
                QString text;
            } formats[] = {{QSize(1080, 1080), QStringLiteral("1:1"), QStringLiteral("1:1 · 1080") + times + QStringLiteral("1080")},
                           {QSize(1080, 1350), QStringLiteral("4:5"), QStringLiteral("4:5 · 1080") + times + QStringLiteral("1350")},
                           {QSize(1080, 1920), QStringLiteral("9:16"), QStringLiteral("9:16 · 1080") + times + QStringLiteral("1920")},
                           {QSize(1280, 720), QStringLiteral("16:9"), QStringLiteral("Custom · 1280") + times + QStringLiteral("720")}};
            for (const auto &format : formats) {
                window.syncCanvasPreset(format.size);
                QCOMPARE(window.m_formatBank->toolTip(), format.text);
                QCOMPARE(window.m_projectSubtitle->text(), QStringLiteral("Edited · %1 · 24 fps").arg(format.ratio));
                QCOMPARE(canvasSizeLabel(format.size), format.text);
                const bool custom = format.text.startsWith(QStringLiteral("Custom"));
                QCOMPARE(window.m_formatBank->currentIndex() < 0, custom);
                for (int index = 0; index < window.m_formatBank->count(); ++index)
                    QVERIFY2(!window.m_formatBank->key(index)->toolTip().contains(QStringLiteral("  ")), qPrintable(window.m_formatBank->key(index)->toolTip()));
            }
            window.syncCanvasPreset(QSize(1920, 1080));
            QCOMPARE(window.m_formatBank->toolTip(), QStringLiteral("16:9 · 1920") + times + QStringLiteral("1080"));
            QCOMPARE(window.m_formatBank->currentIndex(), 0);
        }

        // Text tabs: 13pt names on the panel's own surface, the chosen one in
        // the strongest ink over a 2pt underline of it, the others quieter,
        // a hairline under the row; no strip, no boxes.
        for (const char *id : {"project", "inspector", "effects", "presets", "program", "source", "timeline"}) {
            ads::CDockWidget *dock = window.m_workspace->panel(QString::fromLatin1(id));
            ads::CDockWidgetTab *tab = dock->tabWidget();
            QVERIFY2(tab->property("textTab").toBool(), id);
            auto *name = tab->findChild<QLabel *>();
            QVERIFY(name);
            QCOMPARE(name->font().pixelSize(), 13);
            const bool chosen = dock->dockAreaWidget()->currentDockWidget() == dock;
            // (A tab under the pointer takes the body's ink.)
            if (tab->underMouse()) continue;
            QVERIFY2(name->palette().color(QPalette::WindowText).rgb() == (chosen ? theme.textStrong : theme.tabInactiveText).rgb(),
                     qPrintable(QStringLiteral("%1 %2 %3").arg(QString::fromLatin1(id)).arg(chosen).arg(name->palette().color(QPalette::WindowText).name())));
            QCOMPARE(tab->palette().color(QPalette::Window).alpha() == 0 || !tab->autoFillBackground(), true);
            QCOMPARE(dock->dockAreaWidget()->titleBar()->height(), 32);
        }
        QVERIFY(window.m_workspace->manager()->styleSheet().contains(QStringLiteral("border-bottom: 2px solid ") + theme.textStrong.name()));
        // The Program tab row ends with the frame, the rate and the fit.
        QCOMPARE(window.m_viewerDetail->text(), QStringLiteral("1920 × 1080  ·  24 fps  ·  Fit"));

        // The column: about 300 at 1440x900, "Search media" over two columns
        // of 124 by 70 tiles, 16 in from its edge.
        ads::CDockAreaWidget *column = window.m_workspace->panel(QStringLiteral("project"))->dockAreaWidget();
        QVERIFY2(std::abs(column->width() - 300) <= 6, qPrintable(QString::number(column->width())));
        ProjectPanel *bin = window.m_projectPanel;
        QCOMPARE(bin->search()->placeholderText(), QStringLiteral("Search media"));
        QCOMPARE(bin->m_list->count(), 4);
        const QRect first = bin->m_list->visualItemRect(bin->m_list->item(0));
        const QRect second = bin->m_list->visualItemRect(bin->m_list->item(1));
        QCOMPARE(second.top(), first.top());
        QVERIFY(second.left() > first.right() - 1);
        const int tileLeft = bin->m_list->viewport()->mapTo(bin, first.topLeft()).x() + (first.width() - ProjectPanel::kThumbnail.width()) / 2;
        QCOMPARE(tileLeft, ProjectPanel::kSidePadding);
        QCOMPARE(bin->m_list->item(0)->data(Qt::UserRole + 1).toString(), QStringLiteral("16:9"));
        QCOMPARE(bin->m_list->item(1)->data(Qt::UserRole + 1).toString(), QStringLiteral("9:16"));
        QCOMPARE(bin->m_list->item(3)->data(Qt::UserRole + 1).toString(), QStringLiteral("Audio"));
        // Search filters the bin by name.
        bin->search()->setText(QStringLiteral("DRONE"));
        QStringList shown;
        for (int row = 0; row < bin->m_list->count(); ++row)
            if (!bin->m_list->item(row)->isHidden()) shown << bin->m_list->item(row)->data(Qt::UserRole).toString();
        QCOMPARE(shown, QStringList{QStringLiteral("drone")});
        bin->search()->clear();
        QVERIFY(!bin->m_list->item(0)->isHidden());

        // The timeline's header: after its name, how many tracks and how
        // long; on the right, the zoom beside the track heights.
        QCOMPARE(window.m_timelineDetail->text(), QStringLiteral("9 tracks  ·  00:00:18:00"));
        QVERIFY(window.m_timelineDetail->isVisible() && window.m_timelineZoom->isVisible());
        ads::CDockAreaTitleBar *header = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->titleBar();
        QCOMPARE(window.m_timelineDetail->parentWidget(), header);
        QCOMPARE(header->indexOf(window.m_timelineDetail), header->indexOf(header->tabBar()) + 1);
        QVERIFY(window.m_timelineZoom->x() > header->width() / 2);
        auto *heights = header->findChild<QToolButton *>(QStringLiteral("trackHeightMini"));
        QVERIFY(heights && window.m_timelineZoom->geometry().right() < heights->x());
        // At their default heights, filmstrips and waveforms in view, six
        // tracks fit at 1440x900; at Small, one line each, eight.
        const auto tracksShown = [&window] {
            QCoreApplication::processEvents();
            const int bottom = window.m_timelineScroll->viewport()->height() - window.m_timeline->pos().y();
            int fit = 0;
            for (int row = 0; row < window.m_timeline->trackRowCount(); ++row)
                if (window.m_timeline->trackHeaderRect(row).bottom() < bottom) ++fit;
            return fit;
        };
        QVERIFY2(tracksShown() >= 6, qPrintable(QString::number(tracksShown())));
        window.m_timeline->setAllTrackHeights(kTrackHeightPresets[1].height);
        QVERIFY2(tracksShown() >= 8, qPrintable(QString::number(tracksShown())));
        window.m_timeline->resetTrackHeights();
        // The zoom sets the timeline's scale, and follows it.
        const double before = window.m_timeline->scale();
        window.m_timelineZoom->zoomInButton()->click();
        QVERIFY(window.m_timeline->scale() > before * 1.2);
        window.m_timelineZoom->slider()->setValue(0);
        QVERIFY(std::abs(window.m_timeline->scale() - TimelineWidget::kMinScale) < 0.5);
        window.m_timeline->setScale(120.0);
        QCOMPARE(window.m_timelineZoom->slider()->value(), TimelineZoomControl::position(120.0));

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
        QCOMPARE(window.m_timeline->selectedClips(), window.m_selectedClips);
        QCOMPARE(window.m_selectionStatus->text(), QStringLiteral("2 clips selected  ·  00:00:11:00"));
        QCOMPARE(window.m_inspector->selection(), window.m_selectedClips);
        // ⌘-click again takes it out; a plain click selects one.
        QTest::mouseClick(window.m_timeline, Qt::LeftButton, Qt::ControlModifier, other.center());
        QCOMPARE(window.m_selectedClips, QStringList{QStringLiteral("v1a")});
        window.clickSelectClip(QString());
        QVERIFY(window.m_selectionStatus->text().isEmpty());

        // At 1280x800 six tracks still fit.
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        QTest::qWait(20);
        QVERIFY2(tracksShown() >= 6, qPrintable(QString::number(tracksShown())));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::WorkspaceTest)

#include "workspace_test.moc"
