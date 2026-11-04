// Tests for the device frames the Program monitor draws around its picture.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "stage_rail.h"
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

namespace {

// The pixel that holds `point` (in points) of an image drawn at `ratio`.
QColor pixelAt(const QImage &image, const QPointF &point) {
    const qreal ratio = image.devicePixelRatio();
    return image.pixelColor(QPoint(int(std::floor(point.x() * ratio)), int(std::floor(point.y() * ratio))));
}

bool nearColor(const QColor &color, QRgb expected, int tolerance) {
    return std::abs(color.red() - qRed(expected)) <= tolerance && std::abs(color.green() - qGreen(expected)) <= tolerance &&
           std::abs(color.blue() - qBlue(expected)) <= tolerance;
}

// The frames' graphite: a neutral grey between the darkest and the lightest
// of the band's, lid's and base's tones (never the glass, the screen or the stage).
bool graphite(const QColor &color) {
    const int lo = std::min({color.red(), color.green(), color.blue()});
    const int hi = std::max({color.red(), color.green(), color.blue()});
    return lo >= 0x38 && hi <= 0x64 && hi - lo <= 8;
}

QString colorText(const QColor &color) { return color.name(QColor::HexArgb); }

// A frame drawn alone on a transparent image at a pixel ratio of 2.
QImage drawn(const QSizeF &size, const std::function<void(QPainter &)> &paint) {
    QImage image(QSize(int(std::ceil(size.width() * 2)), int(std::ceil(size.height() * 2))), QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(2);
    image.fill(Qt::transparent);
    QPainter painter(&image);
    paint(painter);
    painter.end();
    return image;
}

} // namespace

namespace {

// The stage at the 1600 x 1000 canvas: 1296 x 564, with the panel and the
// rail given and the panel open (mockup-spec 3.1).
void stageAtTheCanvas(ViewerWidget &viewer, const QSize &canvas, bool open = true) {
    viewer.resize(1296, 564);
    viewer.setStage(new QWidget, new QWidget);
    viewer.setChatOpen(open);
    viewer.setProjectInfo(canvas.width(), canvas.height());
}

} // namespace

class DeviceTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the stage's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // A 9:16 picture shows in a phone at an iPhone 16 Pro's exact
    // proportions: its 1206:2622 screen inside a thin border that is even
    // all round (2.454 mm at the sides, 2.410 at the top and foot), corners
    // concentric with the screen's, the picture across the screen's width
    // and centred on its height. No island, punch-hole or status bar; the
    // old choices are the phone. Off draws the picture as a plain rect where
    // the phone's would be.
    void portraitPictureShowsInAPhone() {
        ViewerWidget viewer;
        stageAtTheCanvas(viewer, QSize(1080, 1920));
        QCOMPARE(viewer.devicePreview(), DevicePreview::Phone);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device && stage.phone);
        QCOMPARE(stage.kind, ViewerWidget::Device::Phone);
        QCOMPARE(viewer.videoRect(), stage.card);
        // The phone's box: TOP 8 to 12 over the scrub strip (SY 500): 480 tall.
        QCOMPARE(stage.body.top(), 8.0);
        QVERIFY(std::abs(stage.body.height() - 480.0) < 1e-9);
        QVERIFY(std::abs(deviceBodyAspect() - 71.5 / 149.6) < 1e-9);
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - deviceBodyAspect()) < 1e-9);
        const DeviceGeometry device = deviceGeometry(stage.body, 9.0 / 16.0);
        const double w = stage.body.width();
        QCOMPARE(device.screen, stage.screen);
        QVERIFY(std::abs(device.bodyRadius - kDeviceBodyRadius * w) < 0.01);
        QVERIFY(std::abs(device.bezel - kDeviceBezel * w) < 0.01);
        QVERIFY(std::abs(device.topBezel - kDeviceTopBezel * w) < 0.01);
        QVERIFY(std::abs(device.screenRadius - kDeviceScreenRadius * stage.screen.width()) < 0.01);
        // The border: the sides' and the top's and foot's within 0.2 pt.
        QVERIFY(std::abs((stage.screen.left() - stage.body.left()) - device.bezel) < 0.01);
        QVERIFY(std::abs((stage.body.right() - stage.screen.right()) - device.bezel) < 0.01);
        QVERIFY(std::abs((stage.screen.top() - stage.body.top()) - (stage.body.bottom() - stage.screen.bottom())) < 0.01);
        QVERIFY(std::abs((stage.screen.top() - stage.body.top()) - device.bezel) < 0.2);
        QVERIFY(std::abs(device.bodyRadius - (device.screenRadius + device.bezel)) < 0.01);
        QVERIFY(device.body.contains(device.glass) && device.glass.contains(device.screen));
        QVERIFY(std::abs(stage.screen.width() / stage.screen.height() - kDeviceScreenAspect) < 0.001);
        // The picture: 9:16, across the screen, centred, black above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 9.0 / 16.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() > stage.screen.top() && stage.card.bottom() < stage.screen.bottom());
        // Drawn: the graphite band, the glass, the screen's black above the
        // picture and below it, and the stage past the body's corner.
        viewer.setSafeZones(false);
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(270, 480)), 0.0);
        const auto render = [&viewer] { return viewer.grab().toImage(); };
        const QImage phone = render();
        const QPointF band(stage.body.center().x(), stage.body.top() + 2.0);
        QVERIFY2(graphite(pixelAt(phone, band)), qPrintable(colorText(pixelAt(phone, band))));
        const QPointF glass(stage.body.center().x(), (device.glass.top() + stage.screen.top()) / 2.0);
        QVERIFY2(nearColor(pixelAt(phone, glass), tokens::overlay::deviceGlass, 2), qPrintable(colorText(pixelAt(phone, glass))));
        const QRgb black = tokens::color(tokens::overlay::deviceScreen).rgb();
        QCOMPARE(pixelAt(phone, QPointF(stage.screen.center().x(), (stage.screen.top() + stage.card.top()) / 2.0)).rgb(), black);
        QCOMPARE(pixelAt(phone, QPointF(stage.screen.left() + device.screenRadius, (stage.screen.bottom() + stage.card.bottom()) / 2.0)).rgb(),
                 black);
        QCOMPARE(pixelAt(phone, stage.body.topLeft() + QPointF(1, 1)).rgb(), tokens::color(tokens::surround).rgb());
        // Nothing hangs into the black over the picture: no island, no dot.
        for (double y = stage.screen.top() + 2; y < stage.card.top() - 1; y += 1)
            for (double x = stage.screen.center().x() - 0.3 * stage.screen.width(); x <= stage.screen.center().x() + 0.3 * stage.screen.width(); x += 1)
                QCOMPARE(pixelAt(phone, QPointF(x, y)).rgb(), black);
        // The old island and punch-hole settings open as the same phone.
        viewer.setDevicePreview(devicePreviewFromKey(QStringLiteral("punch")));
        QCOMPARE(viewer.devicePreview(), DevicePreview::Phone);
        QCOMPARE(viewer.stageLayout().card, stage.card);
        QCOMPARE(render(), phone);
        // Off: no phone, the picture a plain rect at the phone's picture rect.
        viewer.setDevicePreview(DevicePreview::Off);
        const ViewerWidget::StageLayout off = viewer.stageLayout();
        QVERIFY(!off.device && off.screen.isEmpty());
        QCOMPARE(off.kind, ViewerWidget::Device::None);
        QCOMPARE(off.card, stage.card);
        QCOMPARE(off.body, off.card);
        QCOMPARE(off.rail, stage.rail);
        const QImage plain = render();
        QCOMPARE(pixelAt(plain, band).rgb(), tokens::color(tokens::surround).rgb());
        QCOMPARE(pixelAt(plain, QPointF(stage.screen.center().x(), (stage.screen.top() + stage.card.top()) / 2.0)).rgb(),
                 tokens::color(tokens::surround).rgb());
        // A wide picture shows in a laptop, never in a phone.
        viewer.setDevicePreview(DevicePreview::Phone);
        viewer.setProjectInfo(1920, 1080);
        QCOMPARE(viewer.stageLayout().kind, ViewerWidget::Device::Laptop);
        QVERIFY(!viewer.stageLayout().phone);
    }

    // A 16:9 picture shows in a laptop at a MacBook Pro 14"'s exact
    // proportions: its 3024:1964 screen inside a border of 5.1 mm on every
    // side (3.5 of black glass, 1.6 of rim), the top as thin as the sides
    // and no notch; under the lid, a base of the lid's width with its top
    // lip and thumb scoop. The picture spans the screen's width, centred,
    // black screen above and below it. Off is the plain picture.
    void widePictureShowsInALaptop() {
        ViewerWidget viewer;
        stageAtTheCanvas(viewer, QSize(1920, 1080));
        QCOMPARE(viewer.laptopPreview(), LaptopPreview::Laptop);
        ViewerWidget::StageLayout stage = viewer.stageLayout();
        QVERIFY(stage.device && !stage.phone);
        QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
        QCOMPARE(viewer.videoRect(), stage.card);
        // The laptop's box: TOP 8 to 9 over the scrub strip: 483 tall.
        QCOMPARE(stage.body.top(), 8.0);
        QVERIFY(std::abs(stage.body.height() - 483.0) < 1e-9);
        QVERIFY(std::abs(laptopBodyAspect() - 312.6 / 217.4) < 1e-9);
        QVERIFY(std::abs(stage.body.width() / stage.body.height() - laptopBodyAspect()) < 1e-9);
        const LaptopGeometry laptop = laptopGeometry(stage.body, 16.0 / 9.0);
        QCOMPARE(laptop.screen, stage.screen);
        const double w = laptop.lid.width();
        QCOMPARE(laptop.lid.left(), stage.body.left());
        QCOMPARE(laptop.lid.width(), stage.body.width());
        QCOMPARE(laptop.base.left(), laptop.lid.left());
        QCOMPARE(laptop.base.width(), laptop.lid.width());
        QCOMPARE(laptop.base.top(), laptop.lid.bottom());
        QCOMPARE(laptop.body.bottom(), laptop.base.bottom());
        QVERIFY(std::abs(laptop.lid.height() - kLaptopLidHeight * w) < 0.01);
        QVERIFY(std::abs(laptop.base.height() - kLaptopBaseHeight * w) < 0.05);
        // The border, 1.6865% of the screen's width, the same on every side.
        QVERIFY(std::abs(laptop.bezel - kLaptopBezel * w) < 0.01);
        QCOMPARE(laptop.topBezel, laptop.bezel);
        QVERIFY(std::abs(laptop.bezel / laptop.screen.width() - 0.016865) < 0.00005);
        QVERIFY(std::abs((laptop.screen.left() - laptop.lid.left()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.right() - laptop.screen.right()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.screen.top() - laptop.lid.top()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs((laptop.lid.bottom() - laptop.screen.bottom()) - laptop.bezel) < 0.01);
        QVERIFY(std::abs(laptop.rim - kLaptopRim * w) < 0.01);
        QCOMPARE(laptop.glass, laptop.lid.adjusted(laptop.rim, laptop.rim, -laptop.rim, -laptop.rim));
        QVERIFY(std::abs(laptop.screen.width() / laptop.screen.height() - 3024.0 / 1964.0) < 0.001);
        QVERIFY(std::abs(laptop.screenRadius - kLaptopScreenRadius * laptop.screen.width()) < 0.01);
        QVERIFY(std::abs(laptop.lidRadius - (laptop.screenRadius + laptop.bezel)) < 0.01);
        QVERIFY(std::abs(laptop.glassRadius - (laptop.lidRadius - laptop.rim)) < 0.01);
        QCOMPARE(laptop.hinge, QRectF(laptop.lid.left() + laptop.rim, laptop.lid.bottom() - laptop.rim, w - 2 * laptop.rim, laptop.rim));
        QVERIFY(std::abs(laptop.scoop.width() - 0.23 * laptop.base.width()) < 0.01);
        QVERIFY(std::abs(laptop.scoop.center().x() - laptop.base.center().x()) < 0.01);
        QCOMPARE(laptop.scoop.top(), laptop.base.top());
        QVERIFY(laptop.base.contains(laptop.scoop));
        // The picture: 16:9, across the screen, centred, black above and below.
        QVERIFY(stage.screen.adjusted(-0.01, -0.01, 0.01, 0.01).contains(stage.card));
        QVERIFY(std::abs(stage.card.width() / stage.card.height() - 16.0 / 9.0) < 0.001);
        QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
        QVERIFY(std::abs(stage.card.center().y() - stage.screen.center().y()) < 0.5);
        QVERIFY(stage.card.top() > stage.screen.top() && stage.card.bottom() < stage.screen.bottom());
        QVERIFY(std::abs(laptopPictureShare() - stage.card.width() / stage.body.width()) < 0.001);
        // Drawn: the lid's graphite rim round its glass, the screen's black
        // above and below the picture with nothing hanging into it, the
        // base's graphite, and the stage past the lid's round corner.
        viewer.setSafeZones(false);
        viewer.setFrame(testing::footageFrame(testing::Shot::HarborWide, QSize(480, 270)), 0.0);
        const QImage image = viewer.grab().toImage();
        const QPointF rim(laptop.lid.left() + 2.0, laptop.lid.center().y());
        QVERIFY2(graphite(pixelAt(image, rim)), qPrintable(colorText(pixelAt(image, rim))));
        const QPointF glass((laptop.glass.left() + laptop.screen.left() - kLaptopGlassEdge) / 2.0, laptop.lid.center().y());
        QVERIFY2(nearColor(pixelAt(image, glass), tokens::overlay::deviceGlass, 2), qPrintable(colorText(pixelAt(image, glass))));
        const QRgb black = tokens::color(tokens::overlay::deviceScreen).rgb();
        QCOMPARE(pixelAt(image, QPointF(stage.screen.left() + 20, (stage.screen.top() + stage.card.top()) / 2.0)).rgb(), black);
        QCOMPARE(pixelAt(image, QPointF(stage.screen.left() + 20, (stage.screen.bottom() + stage.card.bottom()) / 2.0)).rgb(), black);
        for (double y = stage.screen.top() + 1; y < stage.card.top() - 1; y += 1)
            for (double x = stage.screen.center().x() - 0.2 * stage.screen.width(); x <= stage.screen.center().x() + 0.2 * stage.screen.width(); x += 1)
                QCOMPARE(pixelAt(image, QPointF(x, y)).rgb(), black);
        const QPointF face(laptop.base.left() + 0.12 * laptop.base.width(), laptop.base.center().y());
        QVERIFY2(graphite(pixelAt(image, face)), qPrintable(colorText(pixelAt(image, face))));
        QCOMPARE(pixelAt(image, laptop.lid.topLeft() + QPointF(1, 1)).rgb(), tokens::color(tokens::surround).rgb());
        // Off: the plain picture at the laptop's picture rect.
        viewer.setLaptopPreview(LaptopPreview::Off);
        const ViewerWidget::StageLayout off = viewer.stageLayout();
        QVERIFY(!off.device && off.screen.isEmpty());
        QCOMPARE(off.card, stage.card);
        QCOMPARE(off.body, off.card);
        QCOMPARE(off.rail, stage.rail);
        QCOMPARE(pixelAt(viewer.grab().toImage(), rim).rgb(), tokens::color(tokens::surround).rgb());
        // A portrait picture keeps its phone whatever the laptop's choice.
        viewer.setProjectInfo(1080, 1920);
        QCOMPARE(viewer.stageLayout().kind, ViewerWidget::Device::Phone);
    }

    // The sizes at the 1600 x 1000 stage (mockup-spec 3.2): a device box
    // 483 tall for the laptop and a phone body 480 tall, every part to the
    // spec's tenth of a point.
    void exactSizesAtTheStage() {
        // Within `tolerance` of the spec's figure, or a failure naming the part.
#define KERF_CLOSE(actual, expected, tolerance, what)                                                                       \
    QVERIFY2(std::abs((actual) - (expected)) <= (tolerance),                                                               \
             qPrintable(QStringLiteral("%1: %2, expected %3").arg(QLatin1String(what)).arg((actual), 0, 'f', 3).arg(expected)))
        // The viewer gives the devices these bodies at the 1296 x 564 stage.
        {
            ViewerWidget viewer;
            stageAtTheCanvas(viewer, QSize(1920, 1080));
            KERF_CLOSE(viewer.stageLayout().body.width(), 694.5, 0.05, "stage laptop width");
            KERF_CLOSE(viewer.stageLayout().body.height(), 483.0, 1e-9, "stage laptop height");
            viewer.setProjectInfo(1080, 1920);
            KERF_CLOSE(viewer.stageLayout().body.width(), 229.4, 0.05, "stage phone width");
            KERF_CLOSE(viewer.stageLayout().body.height(), 480.0, 1e-9, "stage phone height");
            KERF_CLOSE(viewer.stageLayout().card.height(), 379.8, 0.05, "stage 9:16 height");
        }
        // The laptop, height-bound at 483: 2.22171 pt a millimetre.
        const QRectF laptopBody(85, 8, 483 * laptopBodyAspect(), 483);
        const LaptopGeometry laptop = laptopGeometry(laptopBody, 16.0 / 9.0);
        KERF_CLOSE(laptop.lid.width() / 312.6, 483 / 217.4, 1e-9, "scale");
        KERF_CLOSE(laptop.lid.width(), 694.5, 0.05, "lid width");
        KERF_CLOSE(laptop.lid.height(), 459.0, 0.05, "lid height");
        KERF_CLOSE(laptop.rim, 3.55, 0.01, "rim");
        KERF_CLOSE(laptop.bezel - laptop.rim, 7.78, 0.01, "black glass");
        KERF_CLOSE(laptop.bezel, 11.33, 0.01, "border");
        KERF_CLOSE(laptop.screen.width(), 671.8, 0.05, "screen width");
        KERF_CLOSE(laptop.screen.height(), 436.3, 0.05, "screen height");
        KERF_CLOSE(laptop.picture.width(), 671.8, 0.05, "picture width");
        KERF_CLOSE(laptop.picture.height(), 377.9, 0.05, "picture height");
        KERF_CLOSE(laptop.picture.top() - laptop.screen.top(), 29.2, 0.05, "black above");
        KERF_CLOSE(laptop.screen.bottom() - laptop.picture.bottom(), 29.2, 0.05, "black below");
        KERF_CLOSE(laptop.base.width(), 694.5, 0.05, "base width");
        KERF_CLOSE(laptop.base.height(), 24.0, 0.05, "base height");
        KERF_CLOSE(laptop.body.height(), 483.0, 1e-9, "body height");
        KERF_CLOSE(laptop.screenRadius, 6.7, 0.05, "screen corner");
        KERF_CLOSE(laptop.lidRadius, 18.05, 0.05, "lid corner");
        KERF_CLOSE(laptop.lidFootRadius, 3.1, 0.05, "lid foot");
        KERF_CLOSE(laptop.baseRadius, 4.9, 0.05, "base corner");
        KERF_CLOSE(laptop.scoop.width(), 159.7, 0.1, "scoop width");
        KERF_CLOSE(laptop.scoop.height(), 5.55, 0.01, "scoop depth");
        // The phone, 480 tall: 3.20856 pt a millimetre.
        const QRectF phoneBody(506.6, 8, 480 * deviceBodyAspect(), 480);
        const DeviceGeometry phone = deviceGeometry(phoneBody, 9.0 / 16.0);
        KERF_CLOSE(phone.body.width(), 229.4, 0.05, "body width");
        KERF_CLOSE(phone.bezel, 7.87, 0.01, "side border");
        KERF_CLOSE(phone.topBezel, 7.73, 0.01, "top border");
        KERF_CLOSE(phone.screen.top() - phone.body.top(), 7.73, 0.01, "border over the screen");
        KERF_CLOSE(phone.glass.left() - phone.body.left(), 7.87 - 3.85, 0.01, "band");
        KERF_CLOSE(phone.screen.width(), 213.7, 0.05, "screen width");
        KERF_CLOSE(phone.screen.height(), 464.5, 0.05, "screen height");
        KERF_CLOSE(phone.screenRadius, 33.0, 0.05, "screen corner");
        KERF_CLOSE(phone.bodyRadius, 40.8, 0.05, "body corner");
        KERF_CLOSE(phone.picture.width(), 213.7, 0.05, "9:16 width");
        KERF_CLOSE(phone.picture.height(), 379.8, 0.05, "9:16 height");
        KERF_CLOSE(phone.picture.top() - phone.screen.top(), 42.3, 0.05, "9:16 black");
        // The picture never reaches the screen's corner arcs, so needs no clip.
        QVERIFY(phone.picture.top() - phone.screen.top() > phone.screenRadius);
        const DeviceGeometry square = deviceGeometry(phoneBody, 1.0);
        KERF_CLOSE(square.picture.height(), 213.7, 0.05, "1:1 height");
        KERF_CLOSE(square.picture.top() - square.screen.top(), 125.4, 0.05, "1:1 black");
        const DeviceGeometry portrait = deviceGeometry(phoneBody, 4.0 / 5.0);
        KERF_CLOSE(portrait.picture.height(), 267.1, 0.05, "4:5 height");
        KERF_CLOSE(portrait.picture.top() - portrait.screen.top(), 98.7, 0.05, "4:5 black");
        // The side buttons: 2 wide, 1.3 proud, at the spec's shares.
        const QList<QRectF> buttons = deviceButtons(phone);
        QCOMPARE(buttons.size(), 5);
        for (int index = 0; index < buttons.size(); ++index) {
            const QRectF button = buttons.at(index);
            const DeviceButtonSpan span = kDeviceButtons[index];
            QCOMPARE(button.width(), 2.0);
            KERF_CLOSE(span.left ? phone.body.left() - button.left() : button.right() - phone.body.right(), 1.3, 1e-9, "proud");
            KERF_CLOSE(button.top(), phone.body.top() + span.from * 480, 1e-9, "button top");
            KERF_CLOSE(button.bottom(), phone.body.top() + span.to * 480, 1e-9, "button foot");
        }
        QCOMPARE(int(std::count_if(std::begin(kDeviceButtons), std::end(kDeviceButtons), [](const DeviceButtonSpan &s) { return s.left; })), 3);
        // The body's corner is continuous: through a circular corner's
        // 45-degree point, but leaving the edges later than a circle would.
        const QPainterPath body = deviceBodyPath(phone);
        const double r = phone.bodyRadius;
        const QPointF diagonal = phone.body.topLeft() + QPointF(r, r) * (1.0 - std::sqrt(0.5));
        QVERIFY(body.contains(diagonal + QPointF(0.3, 0.3)) && !body.contains(diagonal - QPointF(0.3, 0.3)));
        QVERIFY(!body.contains(phone.body.topLeft() + QPointF(0.2, r)));
        QVERIFY(body.contains(phone.body.topLeft() + QPointF(0.2, 1.6 * r)));
        QVERIFY(body.contains(phone.body.center()));
        // The feed's skeleton round a 4:5 picture, at the mockup's offsets
        // from the picture's corner, on the screen's black.
        const DeviceFeed feed = deviceFeed(portrait);
        const QPointF origin = portrait.picture.topLeft();
        QCOMPARE(feed.avatar, QRectF(origin + QPointF(12, -36), QSizeF(24, 24)));
        QCOMPARE(feed.name, QRectF(origin + QPointF(44, -32), QSizeF(72, 6)));
        QCOMPARE(feed.handle, QRectF(origin + QPointF(44, -20), QSizeF(44, 6)));
        for (int index = 0; index < 3; ++index)
            QCOMPARE(feed.actions[index], QRectF(QPointF(origin.x() + 12 + 28 * index, portrait.picture.bottom() + 12), QSizeF(16, 16)));
        QCOMPARE(feed.caption, QRectF(QPointF(origin.x() + 12, portrait.picture.bottom() + 40), QSizeF(160, 6)));
        QCOMPARE(feed.captionMore, QRectF(QPointF(origin.x() + 12, portrait.picture.bottom() + 52), QSizeF(112, 6)));
        for (const DeviceGeometry &shape : {portrait, square}) {
            const DeviceFeed parts = deviceFeed(shape);
            for (const QRectF &part : {parts.avatar, parts.name, parts.handle, parts.actions[0], parts.actions[2], parts.caption, parts.captionMore}) {
                QVERIFY(shape.screen.contains(part));
                QVERIFY(!shape.picture.intersects(part));
            }
        }
        // Feed for square and 4:5 pictures only, and named so.
        QVERIFY(deviceShowsFeed(1.0) && deviceShowsFeed(0.8));
        QVERIFY(!deviceShowsFeed(9.0 / 16.0) && !deviceShowsFeed(16.0 / 9.0));
        QCOMPARE(devicePreviewLabel(DevicePreview::Phone, 1.0), QStringLiteral("Phone · Feed"));
        QCOMPARE(devicePreviewLabel(DevicePreview::Phone, 0.8), QStringLiteral("Phone · Feed"));
        QCOMPARE(devicePreviewLabel(DevicePreview::Phone, 9.0 / 16.0), QStringLiteral("Phone"));
        QCOMPARE(devicePreviewLabel(DevicePreview::Off, 1.0), QStringLiteral("Off"));
    }

#undef KERF_CLOSE

    // Drawn, the border is as thick at the top as at the sides and the foot
    // (within half a point at a pixel ratio of 2), and nothing of a notch,
    // island or camera shows on either screen.
    void bezelIsUniform() {
        // Walks from the screen's centre toward `direction` in pixels: the
        // screen's black, then the border (the glass, the rim or band), to
        // the edge of the lid or body. Returns the border's thickness.
        const auto border = [](const QImage &image, const QRectF &screen, QPoint direction, QRgb foot) {
            const qreal ratio = image.devicePixelRatio();
            QPoint at((screen.center() * ratio).toPoint());
            const auto inside = [&image](const QPoint &p) { return image.rect().contains(p); };
            const auto black = [](const QColor &c) { return c.alpha() > 127 && std::max({c.red(), c.green(), c.blue()}) <= 4; };
            while (inside(at) && black(image.pixelColor(at))) at += direction;
            const QPoint inner = at;
            // The edge: the stage (transparent), the 1 pt black outline, or
            // (under the laptop's lid) the end of the hinge's tone.
            while (inside(at)) {
                const QColor c = image.pixelColor(at);
                if (c.alpha() < 128 || black(c)) break;
                if (foot && !nearColor(c, foot, 6) && !nearColor(c, tokens::overlay::deviceGlass, 6) && (at - inner).manhattanLength() > 4) break;
                at += direction;
            }
            // The outline is part of the border where it is drawn.
            int length = (at - inner).manhattanLength();
            if (inside(at) && black(image.pixelColor(at))) length += int(std::lround(ratio));
            return length / ratio;
        };
        const QRgb black = tokens::color(tokens::overlay::deviceScreen).rgb();
        // Pure black over the top of the screen, where a notch or island would be.
        const auto noNotch = [black](const QImage &image, const QRectF &screen) {
            const qreal ratio = image.devicePixelRatio();
            for (double y = screen.top() + 0.5; y < screen.top() + 0.15 * screen.width(); y += 0.5)
                for (double x = screen.left() + 0.25 * screen.width(); x < screen.right() - 0.25 * screen.width(); x += 0.5)
                    if (image.pixelColor(QPointF(x * ratio, y * ratio).toPoint()).rgb() != black) return false;
            return true;
        };
        {
            const QRectF body(6, 6, 483 * laptopBodyAspect(), 483);
            const LaptopGeometry laptop = laptopGeometry(body, 16.0 / 9.0);
            const QImage image = drawn(QSizeF(body.right() + 6, body.bottom() + 6), [&](QPainter &p) { paintLaptopBody(p, laptop, editorTheme()); });
            const double top = border(image, laptop.screen, QPoint(0, -1), 0);
            const double left = border(image, laptop.screen, QPoint(-1, 0), 0);
            const double right = border(image, laptop.screen, QPoint(1, 0), 0);
            const double bottom = border(image, laptop.screen, QPoint(0, 1), device_material::hinge);
            const QString text = QStringLiteral("laptop: top %1, left %2, right %3, foot %4").arg(top).arg(left).arg(right).arg(bottom);
            QVERIFY2(std::max({top, left, right, bottom}) - std::min({top, left, right, bottom}) <= 0.5, qPrintable(text));
            // The border less the glass-edge line, which is the screen's black.
            QVERIFY2(std::abs(top - (laptop.bezel - kLaptopGlassEdge)) <= 0.5, qPrintable(text));
            QVERIFY(noNotch(image, laptop.screen));
            // The hinge's tone in the lid's foot, the base's lip lighter than its face.
            QVERIFY(nearColor(pixelAt(image, laptop.hinge.center()), device_material::hinge, 1));
            const QColor lip = pixelAt(image, QPointF(laptop.base.left() + 0.1 * laptop.base.width(), laptop.base.top() + 0.25));
            const QColor face = pixelAt(image, QPointF(laptop.base.left() + 0.1 * laptop.base.width(), laptop.base.top() + 2.25));
            QVERIFY2(lip.lightness() > face.lightness(), qPrintable(colorText(lip) + QLatin1Char(' ') + colorText(face)));
            const QColor scoop = pixelAt(image, QPointF(laptop.scoop.center().x(), laptop.base.top() + 0.25));
            QVERIFY2(scoop.lightness() < face.lightness(), qPrintable(colorText(scoop)));
            // No halo: the stage round the lid is untouched.
            QCOMPARE(pixelAt(image, QPointF(laptop.lid.left() - 1, laptop.lid.center().y())).alpha(), 0);
            QCOMPARE(pixelAt(image, QPointF(laptop.lid.center().x(), laptop.lid.top() - 1)).alpha(), 0);
        }
        {
            const QRectF body(6, 6, 480 * deviceBodyAspect(), 480);
            const DeviceGeometry phone = deviceGeometry(body, 9.0 / 16.0);
            const QImage image = drawn(QSizeF(body.right() + 6, body.bottom() + 6), [&](QPainter &p) { paintDeviceBody(p, phone, editorTheme()); });
            const double top = border(image, phone.screen, QPoint(0, -1), 0);
            const double left = border(image, phone.screen, QPoint(-1, 0), 0);
            const double right = border(image, phone.screen, QPoint(1, 0), 0);
            const double bottom = border(image, phone.screen, QPoint(0, 1), 0);
            const QString text = QStringLiteral("phone: top %1, left %2, right %3, foot %4").arg(top).arg(left).arg(right).arg(bottom);
            QVERIFY2(std::max({top, left, right, bottom}) - std::min({top, left, right, bottom}) <= 0.5, qPrintable(text));
            QVERIFY2(std::abs(left - phone.bezel) <= 0.5 && std::abs(top - phone.topBezel) <= 0.5, qPrintable(text));
            QVERIFY(noNotch(image, phone.screen));
            // The side buttons stand proud, a little darker than the band.
            for (const QRectF &button : deviceButtons(phone)) {
                const bool left = button.left() < body.left();
                const QColor proud = pixelAt(image, QPointF(left ? body.left() - 0.75 : body.right() + 0.25, button.center().y()));
                QVERIFY2(proud.alpha() > 127 && graphite(proud), qPrintable(colorText(proud)));
                QCOMPARE(pixelAt(image, QPointF(left ? body.left() - 1.75 : body.right() + 1.75, button.center().y())).alpha(), 0);
            }
            // Between the buttons, nothing proud of the band.
            QCOMPARE(pixelAt(image, QPointF(body.left() - 0.75, body.top() + 0.22 * body.height())).alpha(), 0);
            QCOMPARE(pixelAt(image, QPointF(body.right() + 0.25, body.top() + 0.5 * body.height())).alpha(), 0);
        }
    }

    // The laptop's (and the phone's) frame is drawn once for its size,
    // offset within a pixel, picture shape and pixel ratio, then only
    // blitted: repainting the same size, as playback does for every frame,
    // never draws it again.
    void deviceFrameIsDrawnOnceForItsSize() {
        ViewerWidget viewer;
        stageAtTheCanvas(viewer, QSize(1920, 1080));
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
        viewer.resize(1296, 600);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 2);
        // The phone's frame is cached the same way.
        viewer.setProjectInfo(1080, 1920);
        (void)viewer.grab();
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 3);
        // The old settings are the same phone: nothing to draw again.
        viewer.setDevicePreview(devicePreviewFromKey(QStringLiteral("island")));
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 3);
        // The plain card has no frame to draw.
        viewer.setDevicePreview(DevicePreview::Off);
        (void)viewer.grab();
        QCOMPARE(viewer.deviceFrameRenders(), 3);
        viewer.setDevicePreview(DevicePreview::Phone);

        // The cache itself: moved by whole points, blitted; by half a point,
        // a new pixel ratio or a feed picture's shape, drawn again.
        DeviceFrameCache cache;
        QImage target(QSize(800, 600), QImage::Format_ARGB32_Premultiplied);
        target.fill(Qt::transparent);
        QPainter painter(&target);
        const QRectF body(20, 10, 240 * deviceBodyAspect(), 240);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 9.0 / 16.0, 1.0);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body.translated(30, 4), 9.0 / 16.0, 1.0);
        QCOMPARE(cache.renders(), 1);
        // A portrait picture of another shape has no feed: the same frame.
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 2.0 / 3.0, 1.0);
        QCOMPARE(cache.renders(), 1);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body.translated(0.5, 0), 9.0 / 16.0, 1.0);
        QCOMPARE(cache.renders(), 2);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 9.0 / 16.0, 2.0);
        QCOMPARE(cache.renders(), 3);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 4.0 / 5.0, 2.0);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 4.0 / 5.0, 2.0);
        QCOMPARE(cache.renders(), 4);
        cache.paint(painter, DeviceFrameCache::Kind::Phone, body, 1.0, 2.0);
        QCOMPARE(cache.renders(), 5);
        // A laptop's frame does not depend on the picture's shape.
        const QRectF lid(20, 10, 300 * laptopBodyAspect(), 300);
        cache.paint(painter, DeviceFrameCache::Kind::Laptop, lid, 16.0 / 9.0, 1.0);
        cache.paint(painter, DeviceFrameCache::Kind::Laptop, lid, 2.39, 1.0);
        cache.paint(painter, DeviceFrameCache::Kind::Laptop, lid, editorTheme(), 1.0);
        QCOMPARE(cache.renders(), 6);
        painter.end();
    }

    // A 1:1 or 4:5 picture in the phone shows as a feed post: the skeleton
    // of its interface in the raised grey on the screen's black, drawn with
    // the frame; a 9:16 picture has none.
    void feedPhoneDrawsItsSkeleton() {
        const QRectF body(4, 4, 480 * deviceBodyAspect(), 480);
        for (const double aspect : {1.0, 0.8, 9.0 / 16.0}) {
            const DeviceGeometry phone = deviceGeometry(body, aspect);
            const DeviceFeed feed = deviceFeed(phone);
            DeviceFrameCache cache;
            const QImage image = drawn(QSizeF(body.right() + 4, body.bottom() + 4), [&](QPainter &p) {
                cache.paint(p, DeviceFrameCache::Kind::Phone, body, aspect, 2.0);
            });
            const QRgb raised = tokens::color(tokens::raised).rgb();
            const QRgb black = tokens::color(tokens::overlay::deviceScreen).rgb();
            const bool shown = deviceShowsFeed(aspect);
            for (const QPointF &point : {feed.avatar.center(), feed.name.center(), feed.actions[1].center(), feed.caption.center()}) {
                const QColor colour = pixelAt(image, point);
                if (shown)
                    QVERIFY2(colour.rgb() == raised, qPrintable(colorText(colour)));
                else if (phone.screen.contains(point) && !phone.picture.contains(point))
                    QCOMPARE(colour.rgb(), black);
            }
            // Between the parts, the screen's black.
            if (shown) QCOMPARE(pixelAt(image, QPointF(feed.name.right() + 8, feed.name.center().y())).rgb(), black);
        }
        // In the stage: 1:1 and 4:5 go in the phone, as a feed post, with the
        // skeleton round the picture; the device's label says Feed.
        for (const QSize &canvas : {QSize(1080, 1080), QSize(1080, 1350)}) {
            ViewerWidget viewer;
            stageAtTheCanvas(viewer, canvas);
            viewer.setSafeZones(false);
            const ViewerWidget::StageLayout stage = viewer.stageLayout();
            QCOMPARE(stage.kind, ViewerWidget::Device::Phone);
            QVERIFY(stage.phone);
            QVERIFY(std::abs(stage.card.width() - stage.screen.width()) < 0.01);
            QCOMPARE(stage.panel.width(), ViewerWidget::kPhoneChatWidth);
            const DeviceFeed feed = deviceFeed(deviceGeometry(stage.body, double(canvas.width()) / canvas.height()));
            const QImage image = viewer.grab().toImage();
            QCOMPARE(pixelAt(image, feed.avatar.center()).rgb(), tokens::color(tokens::raised).rgb());
            QCOMPARE(pixelAt(image, feed.caption.center()).rgb(), tokens::color(tokens::raised).rgb());
            QCOMPARE(devicePreviewLabel(viewer.devicePreview(), viewer.pictureAspect()), QStringLiteral("Phone · Feed"));
        }
    }

    // The phone is this Mac's preference, never the project's: the rail's
    // Device menu and Window ▸ Viewer set it, for a phone format only, and
    // the next editor opens with it. The choices are Phone and Off; the old
    // island and punch-hole settings open as Phone. Safe zones moved to the
    // Program strip: the rail's old button is retired.
    void devicePreviewIsRememberedOnThisMac() {
        QCOMPARE(devicePreviewChoices(), (QList<DevicePreview>{DevicePreview::Phone, DevicePreview::Off}));
        QCOMPARE(devicePreviewKey(DevicePreview::Phone), QStringLiteral("phone"));
        QCOMPARE(devicePreviewFromKey(QStringLiteral("island")), DevicePreview::Phone);
        QCOMPARE(devicePreviewKey(DevicePreview::Off), QStringLiteral("off"));
        QCOMPARE(devicePreviewLabel(DevicePreview::Phone), QStringLiteral("Phone"));
        QCOMPARE(devicePreviewLabel(DevicePreview::Off), QStringLiteral("Off"));
        for (const char *key : {"phone", "island", "punchhole", "punch", "", "anything"})
            QCOMPARE(devicePreviewFromKey(QString::fromLatin1(key)), DevicePreview::Phone);
        QCOMPARE(devicePreviewFromKey(QStringLiteral("off")), DevicePreview::Off);
        // The menu's actions by choice, whatever their order.
        const auto actionFor = [](const MainWindow &window, DevicePreview preview) -> QAction * {
            for (QAction *action : window.m_deviceActions)
                if (static_cast<DevicePreview>(action->data().toInt()) == preview) return action;
            return nullptr;
        };
        {
            MainWindow window{QString(), QString()};
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            // A wide picture: the laptop on the rail.
            QCOMPARE(window.m_rail->deviceButton()->glyph(), RailButton::Glyph::Laptop);
            QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
            QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
            timeline.insert(QStringLiteral("width"), 1080);
            timeline.insert(QStringLiteral("height"), 1920);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(snapshot);
            QCoreApplication::processEvents();
            RailButton *device = window.m_rail->deviceButton();
            QVERIFY(device->isVisible() && device->isChecked());
            QCOMPARE(device->glyph(), RailButton::Glyph::Phone);
            QVERIFY(!window.m_rail->findChild<QWidget *>(QStringLiteral("railSafeZones")));
            QVERIFY(window.m_viewer->stageLayout().device);
            // The panel beside the phone: 480 wide, its right edge 12 from the stage's.
            const ViewerWidget::StageLayout stage = window.m_viewer->stageLayout();
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(stage.panel.width(), ViewerWidget::kPhoneChatWidth);
            QCOMPARE(stage.panel.right() + 1, window.m_viewer->width() - ViewerWidget::kStageEdge);
            // Window ▸ Viewer offers Phone and Off, and nothing else.
            QSet<QString> names;
            for (const QAction *action : window.m_deviceActions) names.insert(action->objectName());
            QCOMPARE(names, (QSet<QString>{QStringLiteral("viewer.device.phone"), QStringLiteral("viewer.device.off")}));
            QAction *phone = actionFor(window, DevicePreview::Phone);
            QAction *off = actionFor(window, DevicePreview::Off);
            QVERIFY(phone && off);
            QCOMPARE(phone->text(), QStringLiteral("Phone"));
            QCOMPARE(off->text(), QStringLiteral("Off"));
            QVERIFY(phone->isChecked() && !off->isChecked());
            off->trigger();
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::Off);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/devicePreview")).toString(), QStringLiteral("off"));
            QVERIFY(!window.m_viewer->stageLayout().device && !device->isChecked());
            // The rail's Device menu sets it too.
            emit window.m_rail->devicePreviewRequested(true);
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::Phone);
            QVERIFY(device->isChecked() && phone->isChecked());
            emit window.m_rail->devicePreviewRequested(false);
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::Off);
            // A square picture in the phone is a feed post, and its menu says so.
            timeline.insert(QStringLiteral("width"), 1080);
            timeline.insert(QStringLiteral("height"), 1080);
            snapshot.insert(QStringLiteral("timeline"), timeline);
            window.applySnapshot(snapshot);
            QCOMPARE(phone->text(), QStringLiteral("Phone · Feed"));
            // Nothing of it reaches the project.
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("device")));
            window.close();
        }
        {
            MainWindow window{QString(), QString()};
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::Off);
            QVERIFY(actionFor(window, DevicePreview::Off)->isChecked());
            actionFor(window, DevicePreview::Phone)->trigger();
            QCOMPARE(QSettings().value(QStringLiteral("viewer/devicePreview")).toString(), QStringLiteral("phone"));
        }
        // The old choices' settings open as the phone.
        for (const char *key : {"island", "punchhole", "punch"}) {
            QSettings().setValue(QStringLiteral("viewer/devicePreview"), QString::fromLatin1(key));
            MainWindow window{QString(), QString()};
            QCOMPARE(window.m_viewer->devicePreview(), DevicePreview::Phone);
            QVERIFY(actionFor(window, DevicePreview::Phone)->isChecked());
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::DeviceTest)

#include "device_test.moc"
