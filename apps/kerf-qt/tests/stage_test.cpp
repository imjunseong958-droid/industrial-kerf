// Tests for the Program stage: viewer canvas, player overlay, masks and formats.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "design_tokens.h"
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

class StageTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void viewerKeepsLandscapeAndShortsInsideOneCanvasContract() {
        ViewerWidget viewer;
        viewer.resize(800, 600);

        viewer.setProjectInfo(1920, 1080);
        const QRectF landscape = viewer.videoRect();
        QVERIFY(std::abs(landscape.width() / landscape.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(viewer.rect().contains(landscape.toAlignedRect()));
        QVERIFY(landscape.contains(viewer.captionRect()));

        viewer.setProjectInfo(1080, 1920);
        const QRectF shorts = viewer.videoRect();
        QVERIFY(std::abs(shorts.width() / shorts.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(viewer.rect().contains(shorts.toAlignedRect()));
        QVERIFY(shorts.contains(viewer.captionRect()));

        viewer.resize(800, 220);
        viewer.setProjectInfo(1920, 1080);
        // In a short viewer the wide picture's laptop fills its height.
        QVERIFY(viewer.stageLayout().body.top() < 20.0);
        viewer.show();
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QTest::mouseMove(&viewer, QPoint(400, 80));
        QVERIFY(!scrubs.isEmpty());
        QVERIFY(std::abs(scrubs.constLast()[0].toDouble() - 0.5) < 0.005);
        QVERIFY(nearerScrubFrame(1.0, true, 2.0, 3.0, 24.0));
        QVERIFY(!nearerScrubFrame(2.5, true, 0.2, 3.0, 24.0));
        QVERIFY(nearerScrubFrame(0.0, false, 9.0, 1.0, 24.0));
    }

    // A wide picture shows in a laptop by default, this Mac's preference
    // like the phone: the rail's Laptop button and Window ▸ Viewer choose
    // Laptop or Off, and the next editor opens with it. At a laptop's
    // window the conversation is level with the laptop's body (lid top to
    // base bottom) and as tall, the picture keeps at least 90% of the width
    // the plain card has in the same stage, and with the conversation closed
    // the laptop is centred alone.
    void laptopPreviewIsRememberedOnThisMac() {
        QSettings().remove(QStringLiteral("workspace"));
        QSettings().remove(QStringLiteral("viewer/laptopPreview"));
        QSettings().remove(QStringLiteral("viewer/chatOpen"));
        QSettings().remove(QStringLiteral("viewer/chatWidth"));
        const auto restore = qScopeGuard([] {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("viewer/laptopPreview"));
            QSettings().remove(QStringLiteral("viewer/chatOpen"));
        });
        {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            ViewerWidget *viewer = window.m_viewer;
            QCOMPARE(window.m_canvasSize, QSize(1920, 1080));
            RailButton *device = window.m_rail->deviceButton();
            QVERIFY(device->isVisible() && device->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Laptop"));
            QCOMPARE(device->glyph(), RailButton::Glyph::Laptop);
            const ViewerWidget::StageLayout stage = viewer->stageLayout();
            QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
            QVERIFY(!stage.overlay);
            // The conversation: the laptop body's top and height.
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(stage.panel.top(), qRound(stage.body.top()));
            QVERIFY2(std::abs(stage.panel.height() - stage.body.height()) <= 1,
                     qPrintable(QStringLiteral("%1 for %2").arg(stage.panel.height()).arg(stage.body.height())));
            QVERIFY(stage.panel.height() >= ViewerWidget::kMinChatHeight);
            // Laptop, rail, conversation: one group, centred.
            const int leftSpace = qRound(stage.body.left());
            const int rightSpace = viewer->width() - stage.panel.right() - 1;
            QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1 | %2").arg(leftSpace).arg(rightSpace)));
            QCOMPARE(stage.rail.left() - qRound(stage.body.right()), viewer->stageGap());
            // Measured from the laptop's widest part, its base: the stage's
            // margin on the left, the stage's gap before the rail.
            const auto clearOfEdges = [viewer](const ViewerWidget::StageLayout &layout) {
                const QRectF base = laptopGeometry(layout.body, 16.0 / 9.0).base;
                QVERIFY(layout.body.adjusted(-0.01, -0.01, 0.01, 0.01).contains(base));
                QVERIFY2(base.left() >= ViewerWidget::kStageMargin - 0.5, qPrintable(QString::number(base.left())));
                QVERIFY2(layout.rail.left() - base.right() >= viewer->stageGap() - 1.0,
                         qPrintable(QStringLiteral("%1 to %2").arg(base.right()).arg(layout.rail.left())));
            };
            clearOfEdges(stage);
            // The picture keeps 90% of the plain card's width in this stage,
            // unless the stage's height is what bounds the laptop: over the
            // 344 band the laptop is as tall as the stage lets it be.
            const ViewerWidget::StageLayout plain = viewer->stageLayout(ViewerWidget::Device::None);
            const double roomHeight = viewer->height() - 2.0 * viewer->stageInset(ViewerWidget::Device::Laptop);
            const bool heightBound = std::abs(stage.body.height() - roomHeight) <= 1.0;
            QVERIFY2(heightBound || stage.card.width() >= 0.9 * plain.card.width(),
                     qPrintable(QStringLiteral("%1 of %2, body %3 tall in %4").arg(stage.card.width()).arg(plain.card.width())
                                    .arg(stage.body.height()).arg(roomHeight)));
            // The controls inside the screen.
            QVERIFY(stage.screen.contains(viewer->overlay()->controlsBounds()));
            // Window ▸ Viewer has Laptop and Off; Off is the plain card, level
            // with the conversation and as tall too.
            QCOMPARE(window.m_laptopActions.size(), 2);
            QVERIFY(window.m_laptopActions.at(0)->isChecked() && !window.m_laptopActions.at(1)->isChecked());
            window.m_laptopActions.at(1)->trigger();
            QCoreApplication::processEvents();
            QCOMPARE(viewer->laptopPreview(), LaptopPreview::Off);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/laptopPreview")).toString(), QStringLiteral("off"));
            QVERIFY(!device->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Laptop"));
            const ViewerWidget::StageLayout card = viewer->stageLayout();
            QVERIFY(!card.device);
            QCOMPARE(card.card, plain.card);
            QCOMPARE(window.m_chat->geometry(), card.panel);
            QCOMPARE(card.panel.top(), qRound(card.card.top()));
            QVERIFY(std::abs(card.panel.height() - card.card.height()) <= 1);
            window.m_laptopActions.at(0)->trigger();
            QCoreApplication::processEvents();
            QCOMPARE(viewer->stageLayout().card, stage.card);
            // Closed: the laptop centred alone.
            window.m_chatToggle->click();
            QCoreApplication::processEvents();
            const QRectF alone = viewer->stageLayout().body;
            QVERIFY(std::abs(alone.center().x() - viewer->width() / 2.0) <= 1);
            // As much room over it as under it, the card's 16: centred in
            // the stage, as a card is.
            {
                const double over = alone.top(), under = viewer->height() - alone.bottom();
                QVERIFY2(std::abs(over - under) <= 1, qPrintable(QStringLiteral("%1 over, %2 under").arg(over).arg(under)));
                QVERIFY2(std::abs(over - ViewerWidget::kStageMargin) <= 1, qPrintable(QStringLiteral("%1 over").arg(over)));
            }
            QVERIFY(alone.width() >= stage.body.width() - 0.5);
            clearOfEdges(viewer->stageLayout());
            window.m_chatToggle->click();
            window.m_laptopActions.at(1)->trigger();
            // Nothing of it reaches the project.
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("laptop")));
            window.close();
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->laptopPreview(), LaptopPreview::Off);
        QVERIFY(window.m_laptopActions.at(1)->isChecked());
    }

    // The transport lies on the picture as a video player's controls: shown
    // while paused, faded out a moment after the pointer stops while
    // playing, back with a move; the play button and the bar run the
    // editor's own playback and seeking; and the time reads on a white frame.
    void playerControlsLieOnThePicture() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        PlayerOverlay *overlay = viewer->overlay();
        overlay->setFadeDuration(0);
        overlay->setIdleTimeout(40);
        QCOMPARE(overlay->rect(), viewer->videoRect());
        QVERIFY(!overlay->compact());
        // Paused: shown, with the time over the length.
        QVERIFY(overlay->shown());
        QCOMPARE(overlay->opacity(), 1.0);
        QCOMPARE(overlay->timecodeText(), QStringLiteral("00:00:00:00 / 00:00:18:00"));
        QVERIFY(viewer->videoRect().contains(overlay->controlsBounds()));
        QVERIFY(overlay->controlTip(PlayerOverlay::Control::Play).startsWith(QStringLiteral("Play")));
        // The play button asks for playback.
        QTest::mouseClick(viewer, Qt::LeftButton, {}, overlay->controlRect(PlayerOverlay::Control::Play).center().toPoint());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("play"));
        // Playing: hidden a moment after the pointer's last move...
        QTest::mouseMove(viewer, viewer->videoRect().center().toPoint());
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", true}});
        QVERIFY(overlay->playing());
        QTRY_VERIFY(!overlay->shown());
        QCOMPARE(overlay->opacity(), 0.0);
        // ...back on the next move over the picture, and gone again after.
        QTest::mouseMove(viewer, viewer->videoRect().center().toPoint() + QPoint(5, 5));
        QVERIFY(overlay->shown());
        QTRY_VERIFY(!overlay->shown());
        // Paused again, it stays.
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", false}});
        QVERIFY(overlay->shown());
        // A click on the bar seeks to the frame under it, and a drag follows.
        const QRectF bar = overlay->barRect();
        const auto sendMouse = [viewer](QEvent::Type type, const QPointF &at, Qt::MouseButtons buttons) {
            QMouseEvent event(type, at, viewer->mapToGlobal(at), type == QEvent::MouseMove ? Qt::NoButton : Qt::LeftButton, buttons,
                              Qt::NoModifier);
            QCoreApplication::sendEvent(viewer, &event);
        };
        sendMouse(QEvent::MouseButtonPress, QPointF(bar.left() + bar.width() * 0.5, bar.center().y()), Qt::LeftButton);
        QVERIFY(std::abs(window.m_playhead - 9.0) < 1.0 / 24.0);
        sendMouse(QEvent::MouseMove, QPointF(bar.left() + bar.width() * 0.25, bar.center().y()), Qt::LeftButton);
        sendMouse(QEvent::MouseButtonRelease, QPointF(bar.left() + bar.width() * 0.25, bar.center().y()), Qt::NoButton);
        QVERIFY(std::abs(window.m_playhead - 4.5) < 1.0 / 24.0);
        QCOMPARE(overlay->timecodeText(), QStringLiteral("00:00:04:12 / 00:00:18:00"));
        // The step buttons step one frame.
        QTest::mouseClick(viewer, Qt::LeftButton, {}, overlay->controlRect(PlayerOverlay::Control::StepForward).center().toPoint());
        QVERIFY(std::abs(window.m_playhead - (4.5 + 1.0 / 24.0)) < 0.001);
        // No strip of buttons and timecodes under the picture: the stage
        // below the card is empty.
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("viewerFrame"))->findChild<QLabel *>(QStringLiteral("transportTime")));

        // On a portrait card at a laptop's window, a short-video player's
        // arrangement: the cluster at the top left, the bar along the foot,
        // the time small over it, all on the picture.
        QJsonObject snapshot = showcaseSnapshot(4, QStringLiteral("clean"));
        QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
        timeline.insert(QStringLiteral("width"), 1080);
        timeline.insert(QStringLiteral("height"), 1920);
        snapshot.insert(QStringLiteral("timeline"), timeline);
        window.applySnapshot(snapshot);
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        for (const DevicePreview preview : {DevicePreview::Phone, DevicePreview::Off}) {
            window.setDevicePreview(preview);
            QCoreApplication::processEvents();
            const QRectF card = viewer->videoRect();
            QVERIFY(overlay->compact());
            QVERIFY2(card.contains(overlay->controlsBounds()), qPrintable(QStringLiteral("%1 wide").arg(card.width())));
            QVERIFY(overlay->controlRect(PlayerOverlay::Control::Play).top() < card.top() + 8);
            QVERIFY(overlay->barRect().bottom() >= card.bottom() - 0.5);
            QVERIFY(overlay->controlRect(PlayerOverlay::Control::StepForward).right() < card.right());
            // The time fits whole: over the length where there is room, the
            // playhead's alone where there is not.
            const QFontMetricsF metrics(overlay->timeFont(11));
            QVERIFY(metrics.horizontalAdvance(overlay->timecodeText()) <= overlay->timecodeRect().width());
            QVERIFY(overlay->timecodeText().startsWith(QStringLiteral("00:00:04:13")));
        }
        window.setDevicePreview(DevicePreview::Phone);
        window.close();

        // The time reads at 4.5:1 on a white frame, on the gradient's darkest part.
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            ViewerWidget plain;
            plain.resize(900, 520);
            plain.setProjectInfo(canvas.width(), canvas.height());
            QImage white(canvas / 4, QImage::Format_RGB32);
            white.fill(Qt::white);
            plain.setFrame(white, 0.0);
            PlayerOverlay *controls = plain.overlay();
            const QImage image = plain.grab().toImage();
            const QRectF time = controls->timecodeRect();
            const QFontMetricsF metrics(controls->timeFont(controls->compact() ? 11 : 12));
            const QPointF clear(time.left() + metrics.horizontalAdvance(controls->timecodeText()) + 6, time.center().y());
            QVERIFY(controls->rect().contains(clear));
            const QColor under = image.pixelColor((clear * image.devicePixelRatio()).toPoint());
            const QColor ink = laidOn(under, editorTheme().playerInk);
            QVERIFY2(contrastRatio(ink, under) >= 4.5, qPrintable(QStringLiteral("%1 on %2").arg(ink.name(), under.name())));
        }
    }

    // The pointer on the picture works as on a video player: skimming maps
    // across the picture (as the bar does), not across the stage, so the
    // frame skimmed is the one under the bar's tip; a click on the picture
    // plays or pauses and never seeks; off the picture nothing happens.
    void pictureSkimsAcrossItselfAndAClickPlays() {
        ViewerWidget viewer;
        viewer.resize(1063, 528);
        viewer.setStage(new QWidget, new QWidget);
        viewer.setChatOpen(true);
        viewer.setProjectInfo(1080, 1920);
        viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&viewer));
        const QRectF card = viewer.videoRect();
        QVERIFY(viewer.stageLayout().device);
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QSignalSpy ends(&viewer, &ViewerWidget::scrubEnded);
        QSignalSpy seeks(viewer.overlay(), &PlayerOverlay::seekRequested);
        QSignalSpy controls(viewer.overlay(), &PlayerOverlay::controlTriggered);
        QTest::mouseMove(&viewer, card.center().toPoint());
        QVERIFY(!scrubs.isEmpty());
        QVERIFY2(std::abs(scrubs.constLast()[0].toDouble() - 0.5) <= 0.02, qPrintable(QString::number(scrubs.constLast()[0].toDouble())));
        QTest::mouseMove(&viewer, QPoint(qRound(card.left() + card.width() * 0.25), qRound(card.center().y())));
        QVERIFY(std::abs(scrubs.constLast()[0].toDouble() - 0.25) <= 0.02);
        // Off the picture, on the stage beside the phone: the skim ends.
        const qsizetype skimmed = scrubs.size();
        QTest::mouseMove(&viewer, QPoint(8, qRound(card.center().y())));
        QCOMPARE(scrubs.size(), skimmed);
        QCOMPARE(ends.size(), 1);
        // A click on the picture plays; a click on the stage does nothing.
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, card.center().toPoint());
        QCOMPARE(controls.size(), 1);
        QCOMPARE(controls.constFirst()[0].value<PlayerOverlay::Control>(), PlayerOverlay::Control::Play);
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, QPoint(8, 8));
        QCOMPARE(controls.size(), 1);
        QVERIFY(seeks.isEmpty());
    }

    // The caption preview is lifted over the player's controls while they
    // show, as a video site lifts its captions, and drawn over the shade, so
    // the bar never runs through it; with the controls gone it is back at
    // its place near the foot. Safe zones keep the controls (all but the
    // bar) off the picture until the pointer comes over it. A mask's handle
    // under a control is the handle's, and the empty card has one play
    // button only, the player's.
    void captionsAndMasksKeepClearOfThePlayer() {
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(canvas.width(), canvas.height());
            viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
            PlayerOverlay *controls = viewer.overlay();
            controls->setFadeDuration(0);
            controls->setIdleTimeout(10);
            const QRectF video = viewer.videoRect();
            QCOMPARE(controls->opacity(), 1.0);
            const QRectF lifted = viewer.captionRect();
            QVERIFY(video.contains(lifted));
            QVERIFY2(lifted.bottom() <= controls->footTop() - 8.0 + 0.01,
                     qPrintable(QStringLiteral("%1 over %2").arg(lifted.bottom()).arg(controls->footTop())));
            QVERIFY(!lifted.intersects(controls->barReach()) && !lifted.intersects(controls->timecodeRect()));
            for (const PlayerOverlay::Control control : {PlayerOverlay::Control::Play, PlayerOverlay::Control::StepBack,
                                                         PlayerOverlay::Control::StepForward})
                QVERIFY(!lifted.intersects(controls->controlRect(control)));
            QVERIFY(contrastRatio(editorTheme().viewerCaptionText, laidOn(QColor(Qt::white), editorTheme().viewerCaption)) >= 4.5);
            // Playing, the controls fade: the caption goes back to its place.
            controls->setPlaying(true);
            QTRY_COMPARE(controls->opacity(), 0.0);
            const QRectF rest = viewer.captionRect();
            QVERIFY(rest.bottom() >= lifted.bottom());
            if (canvas.width() > canvas.height()) QVERIFY(rest.bottom() > lifted.bottom());
            QVERIFY(std::abs(rest.bottom() - (video.bottom() - std::max(12.0, video.height() * 0.08))) < 0.5);
            controls->setPlaying(false);
        }
        {
            // Safe zones: only the bar until the pointer is over the picture.
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1080, 1920);
            viewer.show();
            QVERIFY(QTest::qWaitForWindowExposed(&viewer));
            PlayerOverlay *controls = viewer.overlay();
            QVERIFY(controls->controlsShown());
            viewer.setSafeZones(true);
            QVERIFY(controls->controlsSuppressed() && !controls->controlsShown());
            QVERIFY(std::abs(controls->footTop() - (viewer.videoRect().bottom() - PlayerOverlay::kBarHoverHeight)) < 0.01);
            QTest::mouseMove(&viewer, viewer.videoRect().center().toPoint());
            QVERIFY(controls->controlsShown());
            QTest::mouseMove(&viewer, QPoint(4, 4));
            QVERIFY(!controls->controlsShown());
            viewer.setSafeZones(false);
            QVERIFY(!controls->controlsSuppressed());
        }
        {
            // A mask whose corner sits on the play button: pressing there
            // takes the corner, not the button.
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1920, 1080);
            viewer.show();
            QVERIFY(QTest::qWaitForWindowExposed(&viewer));
            const QRectF video = viewer.videoRect();
            const QPointF play = viewer.overlay()->controlRect(PlayerOverlay::Control::Play).center();
            const double fx = (play.x() - video.left()) / video.width();
            const double fy = (play.y() - video.top()) / video.height();
            viewer.setMasks(QStringLiteral("v1"),
                            QJsonArray{QJsonObject{{"name", "k1"}, {"mode", "add"}, {"rect", QJsonArray{fx, 0.2, 0.8, fy}}}},
                            QTransform::fromScale(1920, 1080));
            viewer.selectMask(QStringLiteral("k1"));
            QSignalSpy controls(viewer.overlay(), &PlayerOverlay::controlTriggered);
            QSignalSpy edits(&viewer, &ViewerWidget::editRequested);
            QTest::mousePress(&viewer, Qt::LeftButton, {}, play.toPoint());
            QTest::mouseMove(&viewer, (play + QPointF(20, -20)).toPoint());
            QTest::mouseRelease(&viewer, Qt::LeftButton, {}, (play + QPointF(20, -20)).toPoint());
            QVERIFY(controls.isEmpty());
            QCOMPARE(edits.size(), 1);
            QCOMPARE(edits.constFirst()[0].toJsonArray().first().toObject().value("op").toString(), QStringLiteral("mask-set"));
        }
        {
            // The empty card: two quiet lines, and no play disc of its own
            // over them (the player's play button is the only one).
            ViewerWidget viewer;
            viewer.resize(900, 520);
            viewer.setProjectInfo(1920, 1080);
            const qreal ratio = viewer.grab().toImage().devicePixelRatio();
            // In the laptop, the screen's black; on the plain card, the canvas.
            for (const bool laptop : {true, false}) {
                viewer.setLaptopPreview(laptop ? LaptopPreview::Laptop : LaptopPreview::Off);
                const QImage image = viewer.grab().toImage();
                const QRectF video = viewer.videoRect();
                const QColor backdrop = laptop ? tokens::color(tokens::overlay::deviceScreen) : editorTheme().viewerCanvas;
                for (int dy = -40; dy <= -26; dy += 2)
                    for (int dx = -14; dx <= 14; dx += 2)
                        QCOMPARE(image.pixelColor(((video.center() + QPointF(dx, dy)) * ratio).toPoint()).rgb(), backdrop.rgb());
            }
        }
    }

    void viewerCanvasResolutionMatchesProjectThenSourceThenDefault() {
        QCOMPARE(resolvedCanvasSize(QJsonObject{{"width", 1920}, {"height", 1080}},
                                    QJsonArray{QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1920, 1080));
        QCOMPARE(resolvedCanvasSize(QJsonObject{},
                                    QJsonArray{QJsonObject{{"width", 1280}, {"height", 720}},
                                               QJsonObject{{"width", 1080}, {"height", 1920}}}),
                 QSize(1080, 1920));
        QCOMPARE(resolvedCanvasSize(QJsonObject{}, QJsonArray{}), QSize(1920, 1080));
    }

    void maskPicturePlacementFollowsTheCompositor() {
        // A 4:3 picture fitted to a 16:9 canvas is pillarboxed.
        const QSizeF canvas(1920, 1080);
        const QTransform fitted = picturePlacement(QJsonObject{}, QSizeF(640, 480), true, canvas, 0.0);
        QCOMPARE(fitted.map(QPointF(0, 0)), QPointF(240, 0));
        QCOMPARE(fitted.map(QPointF(1, 1)), QPointF(1680, 1080));
        // Scale about the center, then move by frame widths.
        const QTransform moved = picturePlacement(QJsonObject{{"props", QJsonObject{{"scale", 0.5}, {"x", 0.25}}}},
                                                  QSizeF(1920, 1080), true, canvas, 0.0);
        QCOMPARE(moved.map(QPointF(0.5, 0.5)), QPointF(1440, 540));
        QCOMPARE(moved.map(QPointF(0, 0)), QPointF(960, 270));
        // Rotation is clockwise about the anchor.
        const QPointF right = picturePlacement(QJsonObject{{"props", QJsonObject{{"rotation", 90.0}}}},
                                               QSizeF(1920, 1080), true, canvas, 0.0).map(QPointF(1, 0.5));
        QVERIFY(std::abs(right.x() - 960) < 1e-6 && std::abs(right.y() - 1500) < 1e-6);
        // A crop keeps the picture's fractions where they were in the source.
        const QTransform cropped = picturePlacement(QJsonObject{{"props", QJsonObject{{"crop_l", 0.5}}}},
                                                    QSizeF(1920, 1080), false, canvas, 0.0);
        QCOMPARE(cropped.map(QPointF(0.5, 0.5)), QPointF(960 - 480, 540));
        // Animated properties are read at the clip time.
        const QJsonObject keyed{{"x", QJsonArray{QJsonObject{{"at", 0.0}, {"value", 0.0}}, QJsonObject{{"at", 2.0}, {"value", 1.0}}}}};
        QCOMPARE(propertyAt(keyed, "x", 1.0, 0.0), 0.5);
        QCOMPARE(propertyAt(keyed, "x", 3.0, 0.0), 1.0);
        QCOMPARE(propertyAt(keyed, "y", 1.0, 0.25), 0.25);
        QVERIFY(maskPath(QJsonObject{{"points", QJsonArray{QJsonArray{0, 0}, QJsonArray{1, 0}, QJsonArray{1, 1}}}, {"closed", false}})
                    .elementCount() == 3);
    }

    void viewerDrawsAndEditsMasksAsTypedOperations() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonArray maskProperties{
            QJsonObject{{"name", "feather"}, {"type", "number"}, {"default", 0.0}, {"min", 0.0}, {"max", 10000.0}, {"unit", "pixels"}},
            QJsonObject{{"name", "opacity"}, {"type", "number"}, {"default", 1.0}, {"min", 0.0}, {"max", 1.0}}};
        const auto snapshot = [&window, &maskProperties](int revision, const QJsonArray &masks) {
            QJsonObject clip{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0},
                             {"effects", QJsonArray{QJsonObject{{"fx", "generate.audiowaveform"}, {"enabled", true}, {"params", QJsonObject{}}}}}};
            if (!masks.isEmpty()) clip.insert("masks", masks);
            window.applySnapshot(QJsonObject{{"revision", revision}, {"duration", 2}, {"timeline", QJsonObject{
                {"fps", 24}, {"width", 1920}, {"height", 1080}, {"maskProperties", maskProperties},
                {"maskModes", QJsonArray{"add", "subtract", "none"}},
                {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "a.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 2}}}},
                {"tracks", QJsonArray{
                    QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
                    QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{
                        QJsonObject{{"id", "a1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}}}});
        };
        const QJsonObject k1{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "add"},
                             {"inverted", false}, {"params", QJsonObject{}}};
        auto op = [&window] {
            const QJsonObject command = window.m_lastCommand;
            window.m_lastCommand = {};
            return command.value("type").toString() == QStringLiteral("edit")
                ? command.value("ops").toArray().first().toObject() : QJsonObject{};
        };
        snapshot(3, {});
        ViewerWidget *viewer = window.m_viewer;
        viewer->resize(960, 560);
        auto *rectangle = window.findChild<QAction *>(QStringLiteral("maskRectangle"));
        auto *ellipse = window.findChild<QAction *>(QStringLiteral("maskEllipse"));
        auto *pen = window.findChild<QAction *>(QStringLiteral("maskPen"));
        QVERIFY(rectangle && ellipse && pen);
        QVERIFY(!rectangle->isEnabled());
        // A sound clip has no picture to mask.
        window.selectClip(QStringLiteral("a1"));
        QVERIFY(!rectangle->isEnabled());
        QVERIFY(!window.m_inspector->section(QStringLiteral("masks")));
        window.selectClip(QStringLiteral("c1"));
        QVERIFY(rectangle->isEnabled());
        // The Inspector's Masks section, with the tools in its header.
        InspectorSection *masksSection = window.m_inspector->section(QStringLiteral("masks"));
        QVERIFY(masksSection);
        QCOMPARE(masksSection->header()->findChildren<QToolButton *>(QStringLiteral("maskTool")).size(), 3);
        QVERIFY(window.m_inspector->findChild<QLabel *>(QStringLiteral("masksHint")));
        // With no masks and no tool, the viewer still scrubs.
        QSignalSpy scrubs(viewer, &ViewerWidget::scrubbed);
        mouse(viewer, QEvent::MouseMove, QPointF(300, 200));
        QCOMPARE(scrubs.size(), 1);

        // A drag with the rectangle tool adds k1 where it was drawn, then
        // puts the tool down with the new mask selected.
        rectangle->trigger();
        QCOMPARE(viewer->maskTool(), MaskTool::Rectangle);
        QTransform toWidget = viewer->pictureToWidget();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.25, 0.25)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.5, 0.6)));
        QCOMPARE(scrubs.size(), 1);
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.75, 0.75)));
        QJsonObject added = op();
        QCOMPARE(added.value("op").toString(), QStringLiteral("mask-add"));
        QCOMPARE(added.value("clip").toString(), QStringLiteral("c1"));
        QCOMPARE(added.value("name").toString(), QStringLiteral("k1"));
        QVERIFY2(near(added.value("rect"), {0.25, 0.25, 0.75, 0.75}), qPrintable(QJsonDocument(added).toJson()));
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(!rectangle->isChecked());
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        // A snapshot read before the edit landed does not drop the selection.
        snapshot(3, {});
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));

        // The snapshot carries it. A corner drags on its own, and a drag
        // inside moves the whole mask.
        snapshot(4, QJsonArray{k1});
        QCOMPARE(viewer->m_masks.size(), 1);
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        toWidget = viewer->pictureToWidget();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.75, 0.75)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.9, 0.8)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.9, 0.8)));
        QJsonObject set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("name").toString(), QStringLiteral("k1"));
        QVERIFY2(near(set.value("rect"), {0.25, 0.25, 0.9, 0.8}), qPrintable(QJsonDocument(set).toJson()));
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.5, 0.5)));
        mouse(viewer, QEvent::MouseMove, toWidget.map(QPointF(0.6, 0.5)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.6, 0.5)));
        set = op();
        QVERIFY2(near(set.value("rect"), {0.35, 0.25, 1.0, 0.8}), qPrintable(QJsonDocument(set).toJson()));
        // A click without a drag changes nothing; the picture holds still
        // while masks are shown.
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.5, 0.5)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.5, 0.5)));
        QVERIFY(op().isEmpty());
        mouse(viewer, QEvent::MouseMove, QPointF(20, 20));
        QCOMPARE(scrubs.size(), 1);

        // The Inspector's Masks section edits its mode, inversion, and
        // properties (feather in px), and an effect's mask parameter picks it.
        snapshot(5, QJsonArray{k1});
        InspectorPanel *inspector = window.m_inspector;
        const auto boxes = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
        QCOMPARE(boxes.size(), 1);
        QVERIFY(boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->isChecked());
        // The selected mask is open.
        QVERIFY(!boxes.at(0)->findChild<QWidget *>(QStringLiteral("maskBody"))->isHidden());
        QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("1"));
        auto *mode = boxes.at(0)->findChild<QComboBox *>(QStringLiteral("maskMode"));
        auto *inverted = boxes.at(0)->findChild<InspectorSwitch *>(QStringLiteral("maskInverted"));
        auto *feather = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("maskParam_feather"));
        auto *maskOpacity = boxes.at(0)->findChild<ScrubField *>(QStringLiteral("maskParam_opacity"));
        QVERIFY(mode && inverted && feather && maskOpacity);
        QCOMPARE(feather->format().unit, QStringLiteral("px"));
        QCOMPARE(maskOpacity->format().unit, QStringLiteral("%"));
        QCOMPARE(maskOpacity->text(), QStringLiteral("100"));
        QCOMPARE(mode->count(), 3);
        QCOMPARE(mode->currentText(), QStringLiteral("add"));
        mode->setCurrentIndex(2);
        emit mode->activated(2);
        set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("mode").toString(), QStringLiteral("none"));
        inverted->click();
        QCOMPARE(op().value("inverted").toBool(), true);
        feather->commitText(QStringLiteral("12"));
        set = op();
        QCOMPARE(set.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(set.value("name").toString(), QStringLiteral("k1"));
        QCOMPARE(set.value("params").toObject().value("feather").toDouble(), 12.0);
        snapshot(5, QJsonArray{QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "add"},
                                           {"inverted", false}, {"params", QJsonObject{{"feather", 12.0}}}}});
        QCOMPARE(feather->text(), QStringLiteral("12"));
        feather->commitText(QStringLiteral("0"));
        QVERIFY(op().value("params").toObject().value("feather").isNull());
        auto *source = inspector->findChild<QComboBox *>(QStringLiteral("effectParam_path"));
        QVERIFY(source);
        QCOMPARE(source->count(), 2);
        QCOMPARE(source->itemText(1), QStringLiteral("k1"));
        // A mask with mode none is a path only, still drawn and edited.
        snapshot(6, QJsonArray{QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "none"},
                                           {"inverted", false}, {"params", QJsonObject{{"feather", 12.0}}}}});
        QCOMPARE(mode->currentText(), QStringLiteral("none"));
        QCOMPARE(feather->value(), 12.0);
        QCOMPARE(viewer->maskAt(toWidget.map(QPointF(0.5, 0.5))), QStringLiteral("k1"));
        // Choosing it in the panel selects it on the viewer; Delete there
        // removes it, as ⋯ ▸ Remove in the panel does.
        viewer->selectMask(QString());
        boxes.at(0)->findChild<QToolButton *>(QStringLiteral("maskName"))->click();
        QCOMPARE(viewer->selectedMask(), QStringLiteral("k1"));
        key(viewer, Qt::Key_Delete);
        QJsonObject removed = op();
        QCOMPARE(removed.value("op").toString(), QStringLiteral("mask-remove"));
        QCOMPARE(removed.value("name").toString(), QStringLiteral("k1"));
        snapshot(7, QJsonArray{k1});
        inspector->findChild<QAction *>(QStringLiteral("maskRemove"))->trigger();
        QCOMPARE(op().value("op").toString(), QStringLiteral("mask-remove"));
        // Selecting it on the picture opens its row there.
        snapshot(7, QJsonArray{k1});
        QWidget *body = inspector->findChild<QWidget *>(QStringLiteral("maskBody"));
        inspector->m_openMasks.clear();
        body->hide();
        viewer->selectMask(QStringLiteral("k1"));
        emit viewer->maskSelected(QStringLiteral("k1"));
        QVERIFY(!body->isHidden());

        // The pen closes an outline on its first point, and Return leaves
        // it open; names take the next free number.
        snapshot(8, {});
        QVERIFY(inspector->findChildren<QWidget *>(QStringLiteral("maskBox")).isEmpty());
        pen->trigger();
        for (const QPointF &corner : {QPointF(0.1, 0.1), QPointF(0.9, 0.1), QPointF(0.5, 0.9), QPointF(0.1, 0.1)}) {
            mouse(viewer, QEvent::MouseButtonPress, toWidget.map(corner));
            mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(corner));
        }
        added = op();
        QCOMPARE(added.value("op").toString(), QStringLiteral("mask-add"));
        QCOMPARE(added.value("name").toString(), QStringLiteral("k1"));
        QCOMPARE(added.value("closed").toBool(), true);
        const QJsonArray points = added.value("points").toArray();
        QCOMPARE(points.size(), 3);
        QVERIFY(near(points.at(2), {0.5, 0.9}));
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        ellipse->trigger();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.2, 0.2)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.6, 0.8)));
        added = op();
        QCOMPARE(added.value("name").toString(), QStringLiteral("k2"));
        QVERIFY2(near(added.value("ellipse"), {0.4, 0.5, 0.2, 0.3}), qPrintable(QJsonDocument(added).toJson()));
        pen->trigger();
        for (const QPointF &point : {QPointF(0.1, 0.5), QPointF(0.9, 0.5)}) {
            mouse(viewer, QEvent::MouseButtonPress, toWidget.map(point));
            mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(point));
        }
        key(viewer, Qt::Key_Return);
        added = op();
        QCOMPARE(added.value("name").toString(), QStringLiteral("k3"));
        QCOMPARE(added.value("closed").toBool(), false);
        // Escape drops a path being drawn, then puts the tool down.
        pen->trigger();
        mouse(viewer, QEvent::MouseButtonPress, toWidget.map(QPointF(0.3, 0.3)));
        mouse(viewer, QEvent::MouseButtonRelease, toWidget.map(QPointF(0.3, 0.3)));
        key(viewer, Qt::Key_Escape);
        QVERIFY(viewer->m_penPoints.isEmpty());
        QCOMPARE(viewer->maskTool(), MaskTool::Pen);
        key(viewer, Qt::Key_Escape);
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(!pen->isChecked());
        QVERIFY(op().isEmpty());
        // Two masks are two rows of the section, a row's height apart; the
        // section's title and the masks' names are semibold.
        {
            QJsonObject k2 = k1;
            k2.insert("name", "k2");
            snapshot(9, QJsonArray{k1, k2});
            const auto pair = inspector->findChildren<QWidget *>(QStringLiteral("maskBox"));
            QCOMPARE(pair.size(), 2);
            inspector->ensurePolished();
            QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionCount"))->text(), QStringLiteral("2"));
            // The module's title is engraved: bold capitals.
            QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionTitle"))->font().weight(), QFont::Bold);
            for (QWidget *box : pair) {
                box->ensurePolished();
                auto *name = box->findChild<QToolButton *>(QStringLiteral("maskName"));
                name->ensurePolished();
                // The flat sheet's UI 500 (spec 0.3 has only 400 and 500).
                QCOMPARE(name->font().weight(), QFont::Medium);
            }
        }
        // Deselecting the clip hides its masks and stops a tool.
        rectangle->trigger();
        window.selectClip(QString());
        QCOMPARE(viewer->maskTool(), MaskTool::None);
        QVERIFY(viewer->m_maskClip.isEmpty());
        QVERIFY(!rectangle->isEnabled());
    }

    void selectedBoxTracksMotionAndStopsAtSceneCut() {
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QVERIFY(directory.isValid());
        for (int index = 0; index < 8; ++index) {
            QImage frame(160, 90, QImage::Format_RGB888);
            frame.fill(index < 4 ? QColor(30, 30, 30) : QColor(30, 30, 220));
            if (index < 4) {
                for (int row = 0; row < 24; ++row)
                    for (int column = 0; column < 24; ++column)
                        frame.setPixelColor(16 + index * 3 + column, 20 + row,
                                            QColor(60 + column * 7, 40 + row * 7, 100 + (column + row) % 8 * 12));
            }
            QVERIFY(frame.save(directory.filePath(QStringLiteral("frame-%1.png").arg(index, 2, 10, QLatin1Char('0')))));
        }
        const QString video = directory.filePath(QStringLiteral("moving.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-framerate"), QStringLiteral("2"),
            QStringLiteral("-i"), directory.filePath(QStringLiteral("frame-%02d.png")),
            QStringLiteral("-c:v"), QStringLiteral("libx264"), QStringLiteral("-pix_fmt"),
            QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        MainWindow window{directory.path(), QString()};
        window.m_fps = 10;
        window.m_project = QJsonObject{{QStringLiteral("sources"), QJsonArray{
            QJsonObject{{QStringLiteral("id"), QStringLiteral("moving")}, {QStringLiteral("path"), video}}}}};
        const QJsonObject clip{{QStringLiteral("sourceId"), QStringLiteral("moving")},
                               {QStringLiteral("timelineStart"), 0}, {QStringLiteral("in"), 0},
                               {QStringLiteral("out"), 4}, {QStringLiteral("speed"), 1}};
        const QRectF seed(16.0 / 160, 20.0 / 90, 24.0 / 160, 24.0 / 90);
        QVERIFY(!window.sourceFrame(clip, 0).isNull());
        const double cut = window.firstSceneCut(clip, 0, 4);
        QVERIFY2(std::abs(cut - 2.0) < 0.1, qPrintable(QString::number(cut)));
        double reliableEnd = 4;
        const auto boxes = window.trackRegion(clip, 0, 4, seed, &reliableEnd);
        QVERIFY(boxes.size() >= 2);
        QVERIFY(boxes.last().second.x() > seed.x());
        QVERIFY(reliableEnd <= 2.5);
    }

    void livePreviewPlaysAndCanStillRender() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "required"}});
        QVERIFY(window.m_previewRequired);
        // The compositor draws the program, so it plays; its note says what
        // only a render matches.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "live"},
                                       {"message", "`a1` uses a crossfade, whose sound only a render plays exactly"}});
        QVERIFY(!window.m_previewRequired);
        QCOMPARE(window.m_previewStatus, QStringLiteral("live"));
        QVERIFY(!window.m_statusDevice->isHidden());
        QCOMPARE(window.m_statusDevice->text(), QStringLiteral("Live"));
        QVERIFY(window.statusBar()->toolTip().contains(QStringLiteral("crossfade")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("sequence.renderPreview")), QString());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(!window.m_commands->reason(QStringLiteral("sequence.renderPreview")).isEmpty());
        QVERIFY(window.m_statusDevice->isHidden());
    }

    // The whole editor in the Editing workspace with Kerf's conversation
    // open in the stage, as a Short shows with its comments open: a 16:9, a
    // 9:16 and a 1:1 project. The card keeps the
    // project's shape and is as large as the group allows; card, rail and
    // conversation are one group centred in the stage, the conversation as
    // tall as the card and level with it. With KERF_SCREENSHOT_DIR set, each
    // is saved as editor-<format>-dark.png, the 9:16 and 16:9 ones with the
    // conversation closed as editor-<format>-closed-dark.png, and the 16:9
    // one with the laptop off as editor-16x9-off-dark.png.
    void editorShowsEveryFormatBesideItsConversation() {
        QSettings().remove(QStringLiteral("workspace"));
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        // A harbor at dawn in the project's shape (the tests' procedural
        // footage): sky over the water, whose lower part is a dark blue-grey.
        const auto picture = [](const QSize &size) { return testing::footageFrame(testing::Shot::HarborWide, size); };
        struct Format {
            const char *name;
            QSize canvas;
        };
        {
            const EditorTheme &theme = editorTheme();
            for (const Format &format : {Format{"16x9", QSize(1920, 1080)}, Format{"9x16", QSize(1080, 1920)}, Format{"1x1", QSize(1080, 1080)}}) {
                QSettings().remove(QStringLiteral("workspace"));
                QJsonObject snapshot = showcaseSnapshot(12, QStringLiteral("clean"));
                QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
                timeline.insert(QStringLiteral("width"), format.canvas.width());
                timeline.insert(QStringLiteral("height"), format.canvas.height());
                snapshot.insert(QStringLiteral("timeline"), timeline);
                snapshot.remove(QStringLiteral("pendingProposal"));
                snapshot.remove(QStringLiteral("proposalTimeline"));
                MainWindow window{QString(), QString()};
                window.setConnection(QStringLiteral("ready"));
                window.applySnapshot(snapshot);
                window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
                fakeArtwork(window.m_artwork);
                window.resize(1440, 900);
                window.show();
                QVERIFY(QTest::qWaitForWindowExposed(&window));
                window.m_errorBanner->hide();
                window.m_viewer->setFrame(picture(format.canvas / 4), 0.0);
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Tighten the fisher close-up and drop the drone pass."));
                window.appendConversation(QStringLiteral("Kerf"),
                    QStringLiteral("I trimmed the close-up by 1.5 seconds, removed the drone pass, and added a lower third at 12 s."));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Trimmed v1b by 36 frames"));
                window.appendConversationNote(QStringLiteral("Tool"), QStringLiteral("Removed v1c"));
                window.appendConversation(QStringLiteral("You"), QStringLiteral("Now caption the dialogue."));
                window.showNextSteps(QJsonArray{QJsonObject{{"label", "Add captions"}, {"request", "Caption the dialogue"}},
                                                QJsonObject{{"label", "Duck the music"}, {"request", "Lower the music under the dialogue"}},
                                                QJsonObject{{"label", "Match the colour"}, {"request", "Match the close-up to the wide"}}});
                QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
                // Let the conversation settle at its latest comment.
                QTest::qWait(30);
                const QString label = QString::fromLatin1(format.name);
                QCOMPARE(window.m_canvasSize, format.canvas);
                verifyListEnd(window.m_chat);
                QVERIFY(window.m_viewer->isVisible() && window.m_chat->isVisible() && window.m_rail->isVisible());
                QVERIFY(!window.m_sourceMonitor->isVisible());

                // The card keeps the project's shape; card (or the phone a
                // portrait one shows in), rail and conversation are one group
                // centred in the stage.
                const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
                QVERIFY2(!stage.overlay, qPrintable(label));
                const bool tall = format.canvas.width() < format.canvas.height();
                const bool wide = format.canvas.width() > format.canvas.height();
                // A portrait picture in a phone, a wide one in a laptop, a
                // square one on the plain card (in a phone it would be under
                // half the card's width here).
                QCOMPARE(stage.kind, tall ? ViewerWidget::Device::Phone : wide ? ViewerWidget::Device::Laptop : ViewerWidget::Device::None);
                if (!tall && !wide) {
                    // Measured, as R10 asks before putting 1:1 in the phone:
                    // the square picture on the card against the same picture
                    // across a phone's screen, in this stage.
                    const ViewerWidget::StageLayout inPhone = window.m_viewer->stageLayout(ViewerWidget::Device::Phone);
                    qInfo("1:1 at 1440x900: card picture %.0f px wide, in the phone %.0f px (%.0f%%)", stage.card.width(),
                          inPhone.card.width(), 100.0 * inPhone.card.width() / stage.card.width());
                    QVERIFY(inPhone.screen.adjusted(-0.5, -0.5, 0.5, 0.5).contains(inPhone.card));
                    QVERIFY2(inPhone.card.width() < 0.5 * stage.card.width(), "a square picture loses over half its width in the phone");
                }
                const QRectF video = stage.card;
                const QRectF body = stage.body;
                QCOMPARE(window.m_viewer->videoRect(), video);
                QVERIFY2(std::abs(video.width() / video.height() - double(format.canvas.width()) / format.canvas.height()) < 0.01,
                         qPrintable(label));
                const int inset = window.m_viewer->stageInset(stage.kind);
                if (!tall) QCOMPARE(inset, ViewerWidget::kStageMargin);
                const QRectF room = QRectF(window.m_viewer->rect()).adjusted(ViewerWidget::kStageMargin, inset,
                                                                             -ViewerWidget::kStageMargin, -inset);
                const QRect panel = window.m_chat->geometry();
                QCOMPARE(panel, stage.panel);
                QCOMPARE(window.m_rail->geometry(), stage.rail);
                const int leftSpace = qRound(body.left());
                const int rightSpace = window.m_viewer->width() - panel.right() - 1;
                QVERIFY2(std::abs(leftSpace - rightSpace) <= 2, qPrintable(QStringLiteral("%1: %2 | %3").arg(label).arg(leftSpace).arg(rightSpace)));
                QVERIFY2(room.contains(QRectF(body.left(), panel.top(), panel.right() - body.left(), panel.height()).adjusted(0.5, 0.5, -0.5, -0.5)),
                         qPrintable(label));
                // Level with the card (or the device's body) and as tall,
                // in every format, and never shorter than its least.
                QCOMPARE(panel.top(), qRound(body.top()));
                QVERIFY2(std::abs(panel.height() - body.height()) <= 1,
                         qPrintable(QStringLiteral("%1: %2 for %3").arg(label).arg(panel.height()).arg(body.height())));
                QVERIFY2(panel.height() >= ViewerWidget::kMinChatHeight, qPrintable(label));
                // A tall or square card fills the stage's height; a wide one
                // the height, or the width the group leaves it.
                if (!wide)
                    QVERIFY2(std::abs(body.height() - room.height()) < 1, qPrintable(label));
                else
                    QVERIFY2(std::abs(body.height() - room.height()) < 1 ||
                                 std::abs(body.width() - (room.width() - 2 * window.m_viewer->stageGap() - ViewerWidget::kRailWidth - panel.width())) < 1,
                             qPrintable(label));
                // Its own width beside a tall or square picture; beside a wide
                // one it narrows so the picture keeps its share.
                if (!wide)
                    QCOMPARE(panel.width(), ViewerWidget::kDefaultChatWidth);
                else
                    QVERIFY(panel.width() >= ViewerWidget::kMinChatWidth && panel.width() <= ViewerWidget::kDefaultChatWidth);
                // At this window a wide or square picture stays the larger
                // part of the group. Beside a phone the conversation takes its
                // phone width (spec 0.1: 480 at 1600), wider than the phone.
                QVERIFY2(tall || body.width() * body.height() > panel.width() * panel.height() * 0.5,
                         qPrintable(QStringLiteral("%1: body %2x%3, panel %4x%5, viewer %6x%7").arg(label).arg(body.width()).arg(body.height())
                                        .arg(panel.width()).arg(panel.height()).arg(window.m_viewer->width()).arg(window.m_viewer->height())));
                // One stage row for every shape over the bottom band (its
                // 344 less the 1 px seam): the timeline does not jump when
                // the format changes.
                const int timelineHeight = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->height();
                QVERIFY2(std::abs(timelineHeight - (tokens::layout::band - 1)) <= 2,
                         qPrintable(QStringLiteral("%1: timeline %2").arg(label).arg(timelineHeight)));
                if (tall) {
                    // The phone: its 1206:2622 screen inside an even border,
                    // the picture across its width; where the stage is too
                    // short for a 240-point screen at the exact proportions
                    // (a body 539 tall), the stage's least inset gives it all
                    // the height there is. A 9:16 picture in the taller screen is bounded
                    // by the stage's height, so it keeps the share of the
                    // plain card's width the phone's proportions leave it:
                    // the screen is 0.93136 of a body 0.47794 as wide as
                    // tall, so 0.7913 of a 9:16 card as tall as the body.
                    QVERIFY(body.contains(stage.screen));
                    QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kDeviceScreenAspect) < 0.001);
                    QVERIFY(std::abs((stage.screen.left() - body.left()) - kDeviceBezel * body.width()) < 0.01);
                    QVERIFY(std::abs(video.width() - stage.screen.width()) < 0.5);
                    QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(video));
                    const double plain = window.m_viewer->stageLayout(ViewerWidget::Device::None).card.width();
                    QVERIFY2((stage.screen.width() >= ViewerWidget::kLeastPhoneScreen || inset == ViewerWidget::kLeastStageInset) &&
                                 stage.screen.width() >= 0.78 * plain,
                             qPrintable(QStringLiteral("%1: screen %2 of %3").arg(label).arg(stage.screen.width()).arg(plain)));
                }
                if (wide) {
                    // The laptop: the 16:9 picture across its 3024:1964
                    // screen, at least 90% as wide as the plain card here.
                    const auto rectText = [](const QRectF &r) { return QStringLiteral("%1,%2 %3x%4").arg(r.x()).arg(r.y()).arg(r.width()).arg(r.height()); };
                    QVERIFY2(body.contains(stage.screen) && stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(video),
                             qPrintable(QStringLiteral("body %1 screen %2 video %3").arg(rectText(body), rectText(stage.screen), rectText(video))));
                    QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kLaptopScreenAspect) < 0.01);
                    QVERIFY(std::abs(video.width() - stage.screen.width()) < 0.5);
                    const double plain = window.m_viewer->stageLayout(ViewerWidget::Device::None).card.width();
                    QVERIFY2(video.width() >= 0.9 * plain, qPrintable(QStringLiteral("%1 of %2").arg(video.width()).arg(plain)));
                    QVERIFY(stage.screen.contains(window.m_viewer->overlay()->controlsBounds()));
                }
                const QImage image = window.grab().toImage();
                const qreal ratio = image.devicePixelRatio();
                const QPoint viewerOrigin = window.m_viewer->mapTo(&window, QPoint());
                const auto at = [&](const QPointF &viewerPoint) {
                    return image.pixelColor((QPointF(viewerOrigin) + viewerPoint).toPoint() * ratio).rgb();
                };
                // Whether the 7 by 7 area at `viewerPoint` is the water: its
                // average a dark blue-grey (blue above red), as no stage,
                // card or panel is. Averaged, so the footage's grain and
                // ripples do not matter.
                const auto water = [&](const QPointF &viewerPoint) {
                    double red = 0, green = 0, blue = 0;
                    const QPoint centre = (QPointF(viewerOrigin) + viewerPoint).toPoint();
                    for (int dy = -3; dy <= 3; ++dy)
                        for (int dx = -3; dx <= 3; ++dx) {
                            const QColor c = image.pixelColor((centre + QPoint(dx, dy)) * ratio);
                            red += c.red(); green += c.green(); blue += c.blue();
                        }
                    red /= 49; green /= 49; blue /= 49;
                    return blue > red + 12 && (red + green + blue) / 3 < 110;
                };
                // Above the player's gradient, either side of the sun's path.
                QVERIFY2(water(QPointF(video.left() + 14, video.top() + video.height() * 0.7)), qPrintable(label));
                QVERIFY2(water(QPointF(video.right() - 14, video.top() + video.height() * 0.7)), qPrintable(label));
                // Around the group, the stage; between card and rail too.
                QCOMPARE(at(QPointF(body.left() - 6, body.top() + 8)), theme.viewerSurround.rgb());
                QCOMPARE(at(QPointF(body.right() + 6, body.top() + 8)), theme.viewerSurround.rgb());
                // The card's (or phone's) and the panel's corners are round:
                // the stage shows at the very corner.
                QCOMPARE(at(body.topLeft() + QPointF(1, 1)), theme.viewerSurround.rgb());
                // The frames' graphite: a neutral grey of the band's, lid's and base's tones.
                const auto graphite = [&](const QPointF &viewerPoint) {
                    const QColor c = QColor::fromRgb(at(viewerPoint));
                    const int lo = std::min({c.red(), c.green(), c.blue()}), hi = std::max({c.red(), c.green(), c.blue()});
                    return lo >= 0x38 && hi <= 0x64 && hi - lo <= 8;
                };
                const QRgb glass = tokens::color(tokens::overlay::deviceGlass).rgb();
                const QRgb black = tokens::color(tokens::overlay::deviceScreen).rgb();
                if (tall) {
                    // The phone's graphite band, its glass, and its screen's
                    // black above the picture.
                    const DeviceGeometry phone = deviceGeometry(body, 9.0 / 16.0);
                    QVERIFY(graphite(QPointF(body.center().x(), std::floor(body.top()) + 2.0)));
                    QCOMPARE(at(QPointF(body.center().x(), (phone.glass.top() + stage.screen.top()) / 2.0)), glass);
                    QCOMPARE(at(QPointF(stage.screen.left() + 3, (stage.screen.top() + video.top()) / 2.0 + 6)), black);
                }
                if (wide) {
                    // The laptop's graphite rim round its glass, the screen's
                    // black over the picture, and its base.
                    const LaptopGeometry laptop = laptopGeometry(body, 16.0 / 9.0);
                    QVERIFY(graphite(QPointF(std::floor(laptop.lid.left()) + 2.0, laptop.lid.center().y())));
                    QCOMPARE(at(QPointF((laptop.glass.left() + laptop.screen.left() - kLaptopGlassEdge) / 2.0, laptop.lid.center().y())), glass);
                    QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.top() + video.top()) / 2.0)), black);
                    QVERIFY(graphite(QPointF(laptop.base.left() + laptop.base.width() * 0.25, laptop.base.center().y())));
                }
                QCOMPARE(at(QPointF(panel.left() + 1, panel.top() + 1)), theme.viewerSurround.rgb());
                // The panel is its own surface, under its header.
                QCOMPARE(at(QPointF(panel.left() + 4, panel.top() + ChatPanel::kHeaderHeight / 2)), theme.chatSurface.rgb());
                QVERIFY(theme.chatSurface.rgb() != theme.viewerSurround.rgb());
                // Its header: the title, the count, ⋯ and the close button.
                QCOMPARE(window.m_chat->title()->text(), QStringLiteral("Kerf"));
                QCOMPARE(window.m_chat->countLabel()->text(), QStringLiteral("3"));
                QVERIFY(window.m_chat->moreButton()->isVisible() && window.m_chat->closeButton()->isVisible());
                QCOMPARE(window.m_rail->chatButton()->label(), QStringLiteral("3"));
                QVERIFY(window.m_chatScroll->widget()->minimumSizeHint().width() <= window.m_chatScroll->viewport()->width());
                QVERIFY(window.m_chatToggle->isChecked() && window.m_rail->chatButton()->isChecked());
                // Kerf's steps folded under its comment.
                QCOMPARE(window.m_chat->rows().at(1)->expander()->text(), QStringLiteral("2 steps"));
                // Its next steps show once, in it, not again over the timeline.
                QVERIFY(window.m_chatNextSteps->isVisible() && !window.m_aiBar->isVisible());
                QCOMPARE(amberPixels(image), 0);
                const QString schemeName = QStringLiteral("dark");
                if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("editor-%1-%2.png").arg(label, schemeName))));
                if (tall && !shots.isEmpty()) {
                    // Safe zones over the phone, and the plain card.
                    window.setSafeZones(true);
                    QCoreApplication::processEvents();
                    QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-9x16-safezones-%1.png").arg(schemeName))));
                    window.setSafeZones(false);
                    window.setDevicePreview(DevicePreview::Off);
                    QCoreApplication::processEvents();
                    QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-9x16-card-%1.png").arg(schemeName))));
                    window.setDevicePreview(DevicePreview::Phone);
                    QCoreApplication::processEvents();
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                if (wide) {
                    // The laptop Off: the plain card, level with the
                    // conversation and as tall.
                    window.setLaptopPreview(LaptopPreview::Off);
                    QCoreApplication::processEvents();
                    const ViewerWidget::StageLayout card = window.m_viewer->stageLayout();
                    QVERIFY(!card.device);
                    QCOMPARE(window.m_chat->geometry().top(), qRound(card.card.top()));
                    QVERIFY(std::abs(window.m_chat->height() - card.card.height()) <= 1);
                    const QImage off = window.grab().toImage();
                    QCOMPARE(amberPixels(off), 0);
                    if (!shots.isEmpty()) QVERIFY(off.save(QDir(shots).filePath(QStringLiteral("editor-16x9-off-%1.png").arg(schemeName))));
                    window.setLaptopPreview(LaptopPreview::Laptop);
                    QCoreApplication::processEvents();
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                // Closed from the toolbar, the picture (its phone, its
                // laptop) is centred alone, the rail hanging on its right.
                if (tall || wide) {
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(!window.m_chat->isVisible() && !window.m_chatToggle->isChecked());
                    // Closed, the timeline's bar stands in for it with the next steps.
                    QVERIFY(window.m_aiBar->isVisible());
                    const QRectF alone = window.m_viewer->stageLayout().body;
                    QVERIFY(std::abs(alone.center().x() - window.m_viewer->width() / 2.0) <= 1);
                    QVERIFY(std::abs(alone.height() - room.height()) < 1);
                    // The same inset over it as under it, the card's 16 for
                    // the laptop; the phone's only as much less as its
                    // 240-point screen needs here, and never under the least.
                    {
                        const double over = alone.top(), under = window.m_viewer->height() - alone.bottom();
                        QVERIFY2(std::abs(over - under) <= 1, qPrintable(QStringLiteral("%1: %2 over, %3 under").arg(label).arg(over).arg(under)));
                        QVERIFY2(std::abs(over - inset) <= 1, qPrintable(QStringLiteral("%1: %2 over for %3").arg(label).arg(over).arg(inset)));
                        if (tall) {
                            QVERIFY(inset >= ViewerWidget::kLeastStageInset && inset <= ViewerWidget::kStageMargin);
                            QVERIFY(window.m_viewer->stageLayout().screen.width() >= ViewerWidget::kLeastPhoneScreen ||
                                    inset == ViewerWidget::kLeastStageInset);
                            if (inset < ViewerWidget::kStageMargin && inset > ViewerWidget::kLeastStageInset) {
                                // One more point over and under and the screen
                                // would fall under 240.
                                const double body = window.m_viewer->height() - 2.0 * (inset + 1);
                                QVERIFY(body * deviceBodyAspect() * (1.0 - 2.0 * kDeviceBezel) < ViewerWidget::kLeastPhoneScreen);
                            }
                        }
                    }
                    QVERIFY(window.m_rail->isVisible());
                    QCOMPARE(window.m_rail->x() - qRound(alone.right()), window.m_viewer->stageGap());
                    const QImage closed = window.grab().toImage();
                    QCOMPARE(amberPixels(closed), 0);
                    if (!shots.isEmpty()) QVERIFY(closed.save(QDir(shots).filePath(QStringLiteral("editor-%1-closed-%2.png").arg(label, schemeName))));
                    // Opened again, the group is back as it was.
                    window.m_chatToggle->click();
                    QCoreApplication::processEvents();
                    QVERIFY(window.m_chat->isVisible());
                    QCOMPARE(window.m_viewer->videoRect(), video);
                }
                window.close();
            }
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StageTest)

#include "stage_test.moc"
