// Tests for the device frames the Program monitor draws around its picture.

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

class DeviceTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // A portrait picture shows in a phone, as a simulator shows an app: a
    // generic phone drawn in code, its screen 9:18.5 inside an even bezel,
    // the picture across the screen's width and centred on its height, the
    // player's controls inside the picture. A wide or square picture never
    // shows in one; Off is the plain card. Island and punch-hole differ, and
    // safe zones outline where a vertical-video app covers the picture.
    void portraitPictureShowsInAPhone() {
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1080, 1920);
        QCOMPARE(viewer.devicePreview(), DevicePreview::Island);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device);
        QCOMPARE(viewer.videoRect(), stage.card);
        // The phone fills the stage's height, inside the stage's inset (in
        // this short stage, the least: its screen is under 240 points even so).
        const int inset = viewer.stageInset(ViewerWidget::Device::Phone);
        QCOMPARE(inset, ViewerWidget::kLeastStageInset);
        QVERIFY(std::abs(stage.body.height() - (viewer.height() - 2 * inset)) < 1);
        QVERIFY(std::abs(stage.body.top() - inset) < 1);
        // As much room under it as over it.
        QVERIFY(std::abs((viewer.height() - stage.body.bottom()) - stage.body.top()) <= 1);
        // Where the stage is tall enough, the card's 16 over and under it.
        {
            viewer.resize(900, 620);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::Phone), ViewerWidget::kStageMargin);
            const ViewerWidget::StageLayout tall = viewer.stageLayout();
            QVERIFY(std::abs(tall.body.top() - ViewerWidget::kStageMargin) < 1);
            QVERIFY(std::abs((viewer.height() - tall.body.bottom()) - ViewerWidget::kStageMargin) <= 1);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::Laptop), ViewerWidget::kStageMargin);
            QCOMPARE(viewer.stageInset(ViewerWidget::Device::None), ViewerWidget::kStageMargin);
            viewer.resize(900, 460);
        }
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - deviceBodyAspect()) < 0.01);
        const DeviceGeometry device = deviceGeometry(stage.body, 9.0 / 16.0);
        QCOMPARE(device.screen, stage.screen);
        QVERIFY(std::abs(device.bodyRadius - kDeviceBodyRadius * stage.body.width()) < 0.01);
        QVERIFY(std::abs(device.bezel - kDeviceBezel * stage.body.width()) < 0.01);
        QVERIFY(std::abs(device.screenRadius - (device.bodyRadius - device.bezel)) < 0.01);
        QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kDeviceScreenAspect) < 0.01);
        // The picture: 9:16, across the screen, centred, black above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() - stage.screen.top() > deviceStatusBar(device).height());
        // The player's controls, the bar and the time stay on the picture.
        QVERIFY(stage.card.contains(viewer.overlay()->controlsBounds()));
        QVERIFY(viewer.overlay()->compact());
        // The camera: a pill for the island, a dot for the punch-hole, both
        // in the black over the picture.
        const QRectF island = deviceCamera(device, DevicePreview::Island);
        const QRectF hole = deviceCamera(device, DevicePreview::PunchHole);
        QVERIFY(island.width() > 3 * hole.width());
        QVERIFY(std::abs(hole.width() - hole.height()) < 0.01);
        for (const QRectF &camera : {island, hole}) {
            QVERIFY(stage.screen.contains(camera));
            QVERIFY(camera.bottom() < stage.card.top());
        }
        // Drawn: the graphite frame and the screen's black, in either appearance.
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(270, 480)), 0.0);
        const auto render = [&viewer] { return viewer.grab().toImage(); };
        const QImage withIsland = render();
        const qreal ratio = withIsland.devicePixelRatio();
        const auto at = [ratio](const QImage &image, const QPointF &point) { return image.pixelColor((point * ratio).toPoint()).rgb(); };
        QCOMPARE(at(withIsland, QPointF(stage.body.center().x(), stage.body.top() + device.bezel / 2.0 + 1)), editorTheme().deviceFrame.rgb());
        QCOMPARE(at(withIsland, QPointF(stage.screen.left() + device.screenRadius, (stage.screen.bottom() + stage.card.bottom()) / 2.0)),
                 editorTheme().deviceScreen.rgb());
        QCOMPARE(at(withIsland, stage.body.topLeft() + QPointF(1, 1)), editorTheme().viewerSurround.rgb());
        viewer.setDevicePreview(DevicePreview::PunchHole);
        QCOMPARE(viewer.stageLayout().card, stage.card);
        const QImage withHole = render();
        QVERIFY(withIsland != withHole);
        // Safe zones: off until asked for, then a wash over the picture's
        // right-hand column, its bottom band and its top.
        QVERIFY(!viewer.safeZonesShown());
        const SafeZones zones = safeZones(stage.card);
        QVERIFY(stage.card.contains(zones.top) && stage.card.contains(zones.actions) && stage.card.contains(zones.caption));
        QVERIFY(zones.actions.left() > stage.card.center().x() && zones.caption.top() > stage.card.center().y());
        viewer.setSafeZones(true);
        const QImage withZones = render();
        const QPointF inColumn(zones.actions.center().x(), zones.actions.top() + 6);
        const QPointF inBand(zones.caption.right() - 6, zones.caption.center().y());
        for (const QPointF &point : {inColumn, inBand})
            QVERIFY(QColor(at(withZones, point)).lightness() > QColor(at(withHole, point)).lightness());
        viewer.setSafeZones(false);
        QCOMPARE(render(), withHole);
        // Off: the plain card, larger than the phone's picture.
        viewer.setDevicePreview(DevicePreview::Off);
        stage = viewer.stageLayout();
        QVERIFY(!stage.device && stage.screen.isEmpty());
        QCOMPARE(stage.body, stage.card);
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(stage.card.width() > viewer.stageLayout().card.width() - 1);
        // Wide and square pictures never show in a phone: a wide one shows
        // in a laptop, a square one on the plain card.
        viewer.setDevicePreview(DevicePreview::Island);
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1080)}) {
            viewer.setProjectInfo(canvas.width(), canvas.height());
            stage = viewer.stageLayout();
            QVERIFY(stage.kind != ViewerWidget::Device::Phone);
            QCOMPARE(stage.kind, canvas.width() > canvas.height() ? ViewerWidget::Device::Laptop : ViewerWidget::Device::None);
            QVERIFY(!viewer.overlay()->compact());
        }
        QCOMPARE(viewer.stageLayout().body, viewer.stageLayout().card);
    }

    // A wide picture shows in a laptop, the phone's companion: a generic
    // laptop of today drawn in code in a warm stone metal, its lid black
    // glass in a thin rim, its screen 16:10 inside a thin even bezel (a
    // little thicker over it), the 16:9 picture across the screen with thin
    // black bars above and below, the camera's notch in the bar over it,
    // the player's controls inside the screen, and a wider, thin base under
    // the lid. Off is the plain card. The conversation is level with the laptop's body and as
    // tall, and the picture keeps at least 90% of the plain card's width.
    void widePictureShowsInALaptop() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1920, 1080);
        QCOMPARE(viewer.laptopPreview(), LaptopPreview::Laptop);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device);
        QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
        QCOMPARE(viewer.videoRect(), stage.card);
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - laptopBodyAspect()) < 0.01);
        const LaptopGeometry laptop = laptopGeometry(stage.body, 16.0 / 9.0);
        QCOMPARE(laptop.screen, stage.screen);
        // The lid: an even bezel of 1.8% of its width, a little more over
        // the screen; the screen 16:10; the base 108% of the lid wide and thin,
        // and the body the stage lays out exactly as wide as the base.
        QVERIFY(std::abs(laptop.bezel - kLaptopBezel * laptop.lid.width()) < 0.01);
        QVERIFY(kLaptopBaseWidth >= 1.08 && kLaptopBaseWidth <= 1.14);
        QVERIFY(std::abs(laptop.base.left() - laptop.body.left()) < 0.01 && std::abs(laptop.base.right() - laptop.body.right()) < 0.01);
        QVERIFY(laptop.body.adjusted(-0.01, -0.01, 0.01, 0.01).contains(laptop.base));
        QVERIFY(laptop.topBezel > laptop.bezel);
        QVERIFY(std::abs((laptop.screen.left() - laptop.lid.left()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.right() - laptop.screen.right()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.bottom() - laptop.screen.bottom()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs(laptop.screen.width() / laptop.screen.height() - 16.0 / 10.0) < 0.001);
        QVERIFY(std::abs(laptop.base.width() / laptop.lid.width() - kLaptopBaseWidth) < 0.001);
        QVERIFY(laptop.base.height() >= 0.029 * laptop.lid.height() && laptop.base.height() <= 0.04 * laptop.lid.height());
        QCOMPARE(laptop.base.top(), laptop.lid.bottom());
        QCOMPARE(laptop.body.bottom(), laptop.base.bottom());
        QVERIFY(std::abs(laptop.lid.center().x() - stage.body.center().x()) < 0.01);
        // The picture: 16:9, across the screen, centred, thin bars above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() > stage.screen.top() && stage.card.bottom() < stage.screen.bottom());
        // The notch hangs from the bezel into the bar over the picture,
        // centred, never over the picture, the camera in it.
        QCOMPARE(laptop.notch.top(), laptop.screen.top());
        QVERIFY(laptop.notch.bottom() < stage.card.top());
        QVERIFY(std::abs(laptop.notch.center().x() - laptop.lid.center().x()) < 0.01);
        const QRectF camera = laptopCamera(laptop);
        QVERIFY(laptop.notch.contains(camera));
        // The player's controls, in a video player's arrangement, inside the screen.
        QVERIFY(!viewer.overlay()->compact());
        QCOMPARE(viewer.overlay()->rect(), stage.card);
        QVERIFY(stage.screen.contains(viewer.overlay()->controlsBounds()));
        // Drawn: the lid's metal rim round its black glass, the screen's
        // black in the bars, the notch, the base's lit top face, and the
        // stage past the lid's corner.
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(480, 270)), 0.0);
        const QImage image = viewer.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](const QPointF &point) { return image.pixelColor((point * ratio).toPoint()).rgb(); };
        QCOMPARE(at(QPointF(std::ceil(laptop.lid.left()) + 0.25, laptop.lid.center().y())), editorTheme().laptopShell.rgb());
        QCOMPARE(at(QPointF((laptop.lid.left() + laptop.rim + laptop.screen.left()) / 2.0, laptop.lid.center().y())), editorTheme().laptopBezel.rgb());
        QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.top() + stage.card.top()) / 2.0)), editorTheme().deviceScreen.rgb());
        QCOMPARE(at(QPointF(laptop.notch.left() + 2.0, laptop.notch.top() + 1.0)), editorTheme().laptopBezel.rgb());
        QCOMPARE(at(QPointF(stage.screen.left() + 20, (stage.screen.bottom() + stage.card.bottom()) / 2.0)), editorTheme().deviceScreen.rgb());
        QCOMPARE(at(QPointF(laptop.base.left() + laptop.base.width() * 0.25, laptop.base.top() + 3.0)), editorTheme().laptopDeck.rgb());
        // The lid's corners are round: no metal at the very corner.
        QVERIFY(at(laptop.lid.topLeft() + QPointF(1, 1)) != editorTheme().laptopShell.rgb());
        // Off: the plain card.
        viewer.setLaptopPreview(LaptopPreview::Off);
        stage = viewer.stageLayout();
        QVERIFY(!stage.device && stage.screen.isEmpty());
        QCOMPARE(stage.body, stage.card);
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 16.0 / 9.0) < 0.001);
        // Too narrow for the row of controls and the time over the length:
        // the player's compact arrangement, as on a portrait card.
        viewer.setLaptopPreview(LaptopPreview::Laptop);
        viewer.resize(400, 200);
        (void)viewer.grab();
        QVERIFY(viewer.stageLayout().card.width() < ViewerWidget::kCompactPlayerWidth);
        QVERIFY(viewer.overlay()->compact());
        viewer.resize(900, 460);
        (void)viewer.grab();
        QVERIFY(!viewer.overlay()->compact());
        // A portrait picture keeps its phone whatever the laptop's choice.
        viewer.setProjectInfo(1080, 1920);
        QCOMPARE(viewer.stageLayout().kind, ViewerWidget::Device::Phone);
    }

    // The laptop's (and the phone's) frame is drawn once for its size,
    // appearance and pixel ratio, then only blitted: repainting the same
    // size, as playback does for every frame, never draws it again.
    void deviceFrameIsDrawnOnceForItsSize() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        setEditorColorScheme(ColorScheme::Dark);
        ViewerWidget viewer;
        viewer.resize(900, 460);
        viewer.setProjectInfo(1920, 1080);
        QCOMPARE(viewer.deviceFrameRenders(), 0);
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 1);
        // New frames of video, a moved playhead and its controls: blitted.
        for (int index = 0; index < 5; ++index) {
            viewer.setFrame(testing::footageFrame(index % 2 ? testing::Shot::HarborWide : testing::Shot::DronePass, QSize(480, 270)), index / 24.0);
            (void)viewer.grab();
        }
        viewer.setCaption(QStringLiteral("We open on the harbor at dawn."));
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 1);
        // A new size draws it again, once.
        viewer.resize(1000, 520);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 2);
        // So does the other appearance.
        setEditorColorScheme(ColorScheme::Light);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 3);
        // The phone's frame is cached the same way.
        viewer.setProjectInfo(1080, 1920);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 4);
        // The plain card has no frame to draw.
        viewer.setDevicePreview(DevicePreview::Off);
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 4);
        viewer.setDevicePreview(DevicePreview::Island);
    }

    // The phone and safe zones are this Mac's preference, never the
    // project's: the rail and Window ▸ Viewer set them, for a portrait
    // picture only, and the next editor opens with them.
    void devicePreviewIsRememberedOnThisMac() {
        {
            MainWindow window{QString(), QString()};
            window.resize(1440, 900);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // A wide picture: the laptop on the rail, no safe zones.
            QVERIFY(window.m_rail->deviceButton()->isVisible() && !window.m_rail->safeZonesButton()->isVisible());
            QCOMPARE(window.m_rail->deviceButton()->glyph(), RailButton::Glyph::Laptop);
            QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1080);
            timeline.insert(QStringLiteral("height"), 1920);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(snapshot);
            QCoreApplication::processEvents();
            RailButton *device = window.m_rail->deviceButton();
            RailButton *safe = window.m_rail->safeZonesButton();
            QVERIFY(device->isVisible() && safe->isVisible());
            QVERIFY(device->isChecked() && !safe->isChecked());
            QCOMPARE(device->label(), QStringLiteral("Phone"));
            QCOMPARE(device->glyph(), RailButton::Glyph::Phone);
            QVERIFY(window.m_viewer->stageLayout().device);
            // The panel is level with the phone's body and as tall.
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            QCOMPARE(window.m_chat->geometry().top(), qRound(stage.body.top()));
            QVERIFY(std::abs(window.m_chat->height() - stage.body.height()) <= 1);
            // Window ▸ Viewer offers the same choices.
            QCOMPARE(window.m_deviceActions.size(), 3);
            QVERIFY(window.m_deviceActions.constFirst()->isChecked());
            window.m_deviceActions.at(1)->trigger();
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::PunchHole);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/devicePreview")).toString(), QStringLiteral("punchhole"));
            QTest::mouseClick(safe, Qt::LeftButton, {}, safe->discRect().center());
            QVERIFY(window.m_viewer->safeZonesShown() && safe->isChecked() && window.m_safeZonesAction->isChecked());
            QCOMPARE(QSettings().value(QStringLiteral("viewer/safeZones")).toBool(), true);
            window.m_deviceActions.at(2)->trigger();
            QVERIFY(!window.m_viewer->stageLayout().device && !device->isChecked());
            // Off keeps its name (its glyph is slashed), as the menu says
            // "Phone · Off"; no word of its own.
            QCOMPARE(device->label(), QStringLiteral("Phone"));
            window.m_deviceActions.at(1)->trigger();
            // Nothing of it reaches the project.
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("device")));
            window.close();
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::PunchHole);
        QVERIFY(window.m_viewer->safeZonesShown());
        QVERIFY(window.m_deviceActions.at(1)->isChecked() && window.m_safeZonesAction->isChecked());
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::DeviceTest)

#include "device_test.moc"
