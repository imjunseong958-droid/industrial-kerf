// Tests for the Program stage: the [OWNER] placement, the parts placed in it,
// the picture overlays, dim, the Program strip, masks and formats.

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
#include "program_strip.h"
#include "scrub_strip.h"
#include "stage_rail.h"
#include "ui/components.h"
#include "ui/editor_bus.h"
#include "ui/wiring/stage_wiring.h"
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
#include <QGraphicsOpacityEffect>
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

// Within `tolerance` of `expected`, or a failure naming the value.
#define KERF_NEAR(actual, expected, tolerance, what)                                                                       \
    QVERIFY2(std::abs(double(actual) - double(expected)) <= (tolerance),                                                   \
             qPrintable(QStringLiteral("%1: %2, expected %3").arg(QLatin1String(what)).arg(double(actual), 0, 'f', 2).arg(double(expected))))

// A stage of the 1600 x 1000 canvas (1296 x 564) with stand-ins for its
// parts: the rail, the panel, the scrub strip, the HUD (hidden, as it is
// until a preview renders) and the Source column.
struct Stage {
    ViewerWidget viewer;
    QWidget *rail = new QWidget;
    QWidget *panel = new QWidget;
    QWidget *scrub = new QWidget;
    QWidget *hud = new QWidget;
    QWidget *source = new QWidget;
    explicit Stage(const QSize &canvas, bool open = true, const QSize &size = QSize(1296, 564)) {
        viewer.resize(size);
        hud->hide();
        viewer.setStage(rail, panel);
        viewer.setStageParts(scrub, hud, source);
        viewer.setChatOpen(open);
        viewer.setProjectInfo(canvas.width(), canvas.height());
    }
};

// A snapshot of the showcase in `canvas`'s shape.
QJsonObject shaped(const QSize &canvas, int revision = 3) {
    QJsonObject snapshot = showcaseSnapshot(revision, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("width"), canvas.width());
    timeline.insert(QStringLiteral("height"), canvas.height());
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.remove(QStringLiteral("pendingProposal"));
    snapshot.remove(QStringLiteral("proposalTimeline"));
    return snapshot;
}

QColor pixelOf(const QImage &image, const QPointF &point) {
    const qreal ratio = image.devicePixelRatio();
    return image.pixelColor(QPoint(int(std::floor(point.x() * ratio)), int(std::floor(point.y() * ratio))));
}

// The comments the stage's overlays read: a QC thread at 17:08 and Tom's
// reply with a drawing on it (mockup-spec 4.9 seed, as fixture data only).
QJsonObject seedComments() {
    const QJsonArray drawing{QJsonObject{{"tool", "pen"},
                                         {"points", QJsonArray{QJsonArray{0.30, 0.30}, QJsonArray{0.50, 0.25}, QJsonArray{0.60, 0.45}}},
                                         {"color", "#ededed"}}};
    return QJsonObject{
        {"type", "comments"},
        {"reviewRevision", 22},
        {"items", QJsonArray{
             QJsonObject{{"id", "c-1708"}, {"parent", QJsonValue()}, {"author", QJsonObject{{"kind", "qc"}, {"name", "QC"}, {"role", "captions"}}},
                         {"at", 17.32}, {"duration", 0.0}, {"body", "Caption overlaps the lower third by 8 px"}, {"tags", QJsonArray{"captions"}},
                         {"blocking", true}, {"resolved", false}, {"qcKey", "qc:text.overlap:t4"}, {"created", "2026-11-04T10:12:00Z"}, {"revision", 22}},
             QJsonObject{{"id", "c-1708-r1"}, {"parent", "c-1708"}, {"author", QJsonObject{{"kind", "person"}, {"name", "Tom Lind"}, {"id", "tom@example.com"}}},
                         {"body", "Here, the flare"}, {"drawing", drawing}, {"resolved", false}, {"created", "2026-11-04T10:13:00Z"}, {"revision", 22}},
             QJsonObject{{"id", "c-0903"}, {"parent", QJsonValue()}, {"author", QJsonObject{{"kind", "person"}, {"name", "Mina Park"}, {"id", "mina@example.com"}}},
                         {"at", 9.12}, {"duration", 0.0}, {"body", "The sea reads magenta here"}, {"resolved", false}, {"created", "2026-11-04T10:00:00Z"}, {"revision", 22}}}}};
}

// A finished check with the caption overlap found at 17:08 (fixture data).
QJsonObject overlapCheck() {
    return QJsonObject{
        {"type", "check"}, {"status", "done"}, {"valid", false},
        {"findings", QJsonArray{QJsonObject{{"severity", "warning"}, {"code", "text.overlap"}, {"entity", "t4"},
                                            {"message", "Caption overlaps Atlas Coffee by 8 px from 00:00:17:00 to 00:00:17:20"},
                                            {"at", 17.0}, {"duration", 0.84}, {"rect", QJsonObject{{"x", 0.30}, {"y", 0.78}, {"w", 0.20}, {"h", 0.04}}},
                                            {"value", 8}, {"limit", 0}, {"unit", "px"}, {"commentId", "c-1708"}}}},
        {"summary", QJsonObject{{"passed", 7}, {"total", 8}, {"blocking", 1}}}};
}

} // namespace

class StageTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the stage's preferences as a new user has them.
    void init() {
        QSettings().remove(QStringLiteral("viewer"));
        QSettings().remove(QStringLiteral("workspace"));
        ViewerWidget::setMotionOverride(-1);
    }
    void cleanup() { ViewerWidget::setMotionOverride(-1); }

    // The registry's stage commands run through their QActions: Dim and
    // Annotations switch the stage, I and O set the scrub strip's marks,
    // ⌘L its loop (and the strip's own loop button checks the command).
    void registryCommandsDriveTheStage() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        CommandRegistry *commands = window.m_commands;
        StageWiring *stage = window.m_stageWiring.get();
        for (const char *id : {"view.dim", "view.annotations", "viewer.zoom", "playback.loop", "mark.in", "mark.out"})
            QVERIFY2(commands->action(QString::fromLatin1(id))->isEnabled(), id);
        const bool dim = stage->dim();
        commands->action(QStringLiteral("view.dim"))->trigger();
        QCOMPARE(stage->dim(), !dim);
        const bool annotations = stage->annotations();
        commands->action(QStringLiteral("view.annotations"))->trigger();
        QCOMPARE(stage->annotations(), !annotations);
        commands->action(QStringLiteral("view.annotations"))->trigger();
        ScrubStrip *scrub = stage->scrub();
        window.m_bus->setPlayhead(2.0);
        commands->action(QStringLiteral("mark.in"))->trigger();
        QVERIFY(scrub->hasIn());
        QCOMPARE(scrub->inPoint(), 2.0);
        window.m_bus->setPlayhead(4.0);
        commands->action(QStringLiteral("mark.out"))->trigger();
        QVERIFY(scrub->hasOut());
        QAction *loop = commands->action(QStringLiteral("playback.loop"));
        QVERIFY(!scrub->loop() && !loop->isChecked());
        loop->trigger();
        QVERIFY(scrub->loop() && loop->isChecked());
        scrub->loopButton()->click();
        QVERIFY(!scrub->loop() && !loop->isChecked());
        // The strip's buttons show the commands' keys now they run.
        QCOMPARE(ui::tipShortcut(scrub->markInButton()), commands->action(QStringLiteral("mark.in"))->shortcut().toString(QKeySequence::NativeText));
    }

    // The 1296 x 564 stage of the 1600 x 1000 canvas, exactly as the
    // [OWNER] table of mockup-spec 3.1 places it (±0.5): the device, the
    // rail 12 right of it, the panel 12 from the stage's right edge, the
    // scrub strip under the device and the Source column on the left.
    void stageFollowsTheOwnerPlacement() {
        {
            // 16:9, the panel open: the laptop and its rail centred in 16 ... 904.
            Stage stage(QSize(1920, 1080));
            const ViewerWidget::StageLayout layout = stage.viewer.stageLayout();
            QCOMPARE(layout.kind, ViewerWidget::Device::Laptop);
            KERF_NEAR(layout.body.left(), 85, 0.5, "16:9 open x0");
            KERF_NEAR(layout.body.right(), 779.5, 0.5, "16:9 open lid right");
            KERF_NEAR(layout.body.top(), 8, 1e-9, "TOP");
            KERF_NEAR(layout.body.width(), 694.5, 0.05, "laptop width");
            KERF_NEAR(layout.body.height(), 483, 1e-9, "laptop height");
            KERF_NEAR(layout.railRect.left(), 791.5, 0.5, "16:9 open rail");
            KERF_NEAR(layout.rail.left(), 791.5, 0.5, "16:9 open rail widget");
            QCOMPARE(layout.rail.width(), 44);
            QCOMPARE(layout.rail.height(), 548);
            QCOMPARE(layout.panel, QRect(916, 8, 368, 548));
            KERF_NEAR(layout.scrub.left(), 96.3, 0.5, "16:9 scrub x");
            KERF_NEAR(layout.scrub.width(), 671.8, 0.05, "16:9 scrub width");
            QCOMPARE(layout.scrub.top(), 500.0);
            QCOMPARE(layout.scrub.height(), 56.0);
            QVERIFY(!layout.narrowScrub);
            KERF_NEAR(layout.card.width(), 671.8, 0.05, "16:9 picture width");
            KERF_NEAR(layout.card.height(), 377.9, 0.05, "16:9 picture height");
            QVERIFY(layout.source.isEmpty());
            // The parts stand where the layout puts them.
            QCOMPARE(stage.panel->geometry(), layout.panel);
            QVERIFY(stage.panel->isVisibleTo(&stage.viewer));
            QCOMPARE(stage.rail->geometry(), layout.rail);
            QCOMPARE(stage.scrub->geometry(), layout.scrub.toRect());
            QVERIFY(!stage.source->isVisibleTo(&stage.viewer));
            // The HUD's card in title safe: 288 x 72 at (w - tx - 8 - hw, ty + 8).
            const double tx = std::round(layout.card.width() * 0.05), ty = std::round(layout.card.height() * 0.05);
            KERF_NEAR(layout.hud.width(), 288, 1e-9, "HUD width");
            KERF_NEAR(layout.hud.height(), 72, 1e-9, "HUD height");
            KERF_NEAR(layout.hud.right(), layout.card.right() - tx - 8, 1e-6, "HUD right");
            KERF_NEAR(layout.hud.top(), layout.card.top() + ty + 8, 1e-6, "HUD top");
            QVERIFY(!layout.hudStrip);
        }
        {
            // 16:9, the panel closed: the rail ends 12 from the edge, the
            // laptop 12 left of it, the Source column on the left.
            Stage stage(QSize(1920, 1080), false);
            const ViewerWidget::StageLayout layout = stage.viewer.stageLayout();
            KERF_NEAR(layout.body.left(), 533.5, 0.5, "16:9 closed x0");
            KERF_NEAR(layout.body.right(), 1228, 0.5, "16:9 closed lid right");
            QCOMPARE(layout.rail.left(), 1240);
            QVERIFY(layout.panel.isEmpty());
            QVERIFY(!stage.panel->isVisibleTo(&stage.viewer));
            KERF_NEAR(layout.scrub.left(), 544.8, 0.5, "16:9 closed scrub x");
            KERF_NEAR(layout.scrub.width(), 671.8, 0.05, "16:9 closed scrub width");
            QCOMPARE(layout.source.topLeft(), QPoint(16, 8));
            KERF_NEAR(layout.source.width(), 505.5, 0.5, "Source width");
            QCOMPARE(layout.source.height(), 548);
            QCOMPARE(stage.source->geometry(), layout.source);
            QVERIFY(stage.source->isVisibleTo(&stage.viewer));
        }
        {
            // 9:16, the panel open (480): the phone 12 left of the rail at 748.
            Stage stage(QSize(1080, 1920));
            const ViewerWidget::StageLayout layout = stage.viewer.stageLayout();
            QCOMPARE(layout.kind, ViewerWidget::Device::Phone);
            KERF_NEAR(layout.body.left(), 506.6, 0.5, "phone open lx");
            KERF_NEAR(layout.body.width(), 229.4, 0.05, "phone width");
            KERF_NEAR(layout.body.height(), 480, 1e-9, "phone height");
            QCOMPARE(layout.rail.left(), 748);
            QCOMPARE(layout.panel, QRect(804, 8, 480, 548));
            QCOMPARE(layout.scrub, QRectF(432, 500, 360, 56));
            QVERIFY(layout.narrowScrub);
            QCOMPARE(layout.source, QRect(16, 8, 400, 548));
            QVERIFY(stage.source->isVisibleTo(&stage.viewer));
            KERF_NEAR(layout.card.height(), 379.8, 0.05, "9:16 picture height");
            // The HUD docks as a 32 strip over the Source column; while it
            // shows, the column moves 40 down.
            QVERIFY(layout.hudStrip);
            QCOMPARE(layout.hud, QRectF(16, 8, 400, 32));
            stage.hud->show();
            QCOMPARE(stage.viewer.stageLayout().source, QRect(16, 48, 400, 508));
            QCOMPARE(stage.source->geometry(), QRect(16, 48, 400, 508));
            QCOMPARE(stage.hud->geometry(), QRect(16, 8, 400, 32));
            stage.hud->hide();
            QCOMPARE(stage.source->geometry(), QRect(16, 8, 400, 548));
        }
        {
            // 9:16, the panel closed: the rail at 1240, the Source column to the scrub - 32.
            Stage stage(QSize(1080, 1920), false);
            const ViewerWidget::StageLayout layout = stage.viewer.stageLayout();
            KERF_NEAR(layout.body.left(), 998.6, 0.5, "phone closed lx");
            QCOMPARE(layout.rail.left(), 1240);
            QCOMPARE(layout.scrub, QRectF(924, 500, 360, 56));
            QCOMPARE(layout.source, QRect(16, 8, 892, 548));
        }
        // 1:1 and 4:5 go in the same phone, beside the 480 panel.
        for (const QSize &canvas : {QSize(1080, 1080), QSize(1080, 1350)}) {
            Stage stage(canvas);
            const ViewerWidget::StageLayout layout = stage.viewer.stageLayout();
            QCOMPARE(layout.kind, ViewerWidget::Device::Phone);
            KERF_NEAR(layout.body.left(), 506.6, 0.5, "feed lx");
            QCOMPARE(layout.panel.width(), 480);
            KERF_NEAR(layout.card.height(), canvas == QSize(1080, 1080) ? 213.7 : 267.1, 0.05, "feed picture height");
        }
    }

    // The anchors hold at every window: the panel 12 from the stage's
    // right edge, the rail 12 right of the device, the laptop's box 64 + 9
    // over the stage's foot, the scrub strip under the picture. At 1600 x
    // 1000 the window's own parts stand exactly where the table puts them.
    void stageKeepsItsAnchorsAtEveryWindow() {
        for (const QSize &size : {QSize(1600, 1000), QSize(1440, 900), QSize(1920, 1080)}) {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(shaped(QSize(1920, 1080)));
            window.resize(size);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            window.m_errorBanner->hide();
            QCoreApplication::processEvents();
            ViewerWidget *viewer = window.m_viewer;
            const QString what = QStringLiteral("%1x%2").arg(size.width()).arg(size.height());
            const ViewerWidget::StageLayout layout = viewer->stageLayout();
            if (size == QSize(1600, 1000)) QCOMPARE(viewer->size(), QSize(1296, 564));
            QVERIFY2(!layout.overlay, qPrintable(what));
            QCOMPARE(window.m_chat->geometry(), layout.panel);
            QCOMPARE(layout.panel.right() + 1, viewer->width() - 12);
            QCOMPARE(layout.panel.top(), 8);
            QCOMPARE(layout.panel.height(), viewer->height() - 16);
            QCOMPARE(layout.panel.width(), 368);
            QCOMPARE(window.m_rail->geometry(), layout.rail);
            KERF_NEAR(layout.railRect.left() - layout.body.right(), 12, 1e-6, qPrintable(what));
            KERF_NEAR(layout.body.height(), viewer->height() - 64 - 9 - 8, 1e-6, qPrintable(what));
            // Centred in 16 ... panel - 12.
            KERF_NEAR(layout.body.left() - 16, (layout.panel.left() - 12) - layout.railRect.right(), 1.0, qPrintable(what));
            QWidget *scrub = window.m_stageWiring->scrub();
            QCOMPARE(scrub->parentWidget(), viewer);
            QCOMPARE(scrub->geometry(), layout.scrub.toRect());
            QCOMPARE(layout.scrub.top(), viewer->height() - 64.0);
            QCOMPARE(window.m_stageSource->parentWidget(), viewer);
            QVERIFY(!window.m_stageSource->isVisible());
            // Closed (as Edit has it): the Source column fills the left.
            window.setChatOpen(false);
            QCoreApplication::processEvents();
            const ViewerWidget::StageLayout closed = viewer->stageLayout();
            QCOMPARE(closed.rail.right() + 1, viewer->width() - 12);
            QVERIFY(window.m_stageSource->isVisible());
            QCOMPARE(window.m_stageSource->geometry(), closed.source);
            KERF_NEAR(closed.source.right() + 1, closed.body.left() - 12, 0.5, qPrintable(what));
            window.setChatOpen(true);
            window.close();
        }
    }

    // The panel is dragged between 300 and 480 and keeps a width for 16:9
    // and one for the phone formats, each remembered on this Mac.
    void panelResizesBetween300And480PerFormat() {
        Stage stage(QSize(1920, 1080));
        QCOMPARE(stage.viewer.chatWidth(), 368);
        QSignalSpy widths(&stage.viewer, &ViewerWidget::chatWidthChanged);
        stage.viewer.setChatWidth(250);
        QCOMPARE(stage.viewer.chatWidth(), 300);
        stage.viewer.setChatWidth(600);
        QCOMPARE(stage.viewer.chatWidth(), 480);
        QCOMPARE(widths.size(), 2);
        // Its right edge stays put.
        QCOMPARE(stage.viewer.stageLayout().panel, QRect(1284 - 480, 8, 480, 548));
        QCOMPARE(stage.panel->geometry(), stage.viewer.stageLayout().panel);
        stage.viewer.setChatWidth(400);
        // The phone's is its own.
        stage.viewer.setProjectInfo(1080, 1920);
        QCOMPARE(stage.viewer.chatWidth(), 480);
        stage.viewer.setChatWidth(320);
        QCOMPARE(stage.viewer.stageLayout().panel.width(), 320);
        stage.viewer.setProjectInfo(1920, 1080);
        QCOMPARE(stage.viewer.chatWidth(), 400);
        QCOMPARE(stage.viewer.chatWidth(true), 320);
        // The window remembers each under its own key when a drag ends.
        {
            MainWindow window{QString(), QString()};
            window.m_viewer->setChatWidth(330);
            emit window.m_chat->widthDragFinished();
            QCOMPARE(QSettings().value(QStringLiteral("viewer/chatWidth")).toInt(), 330);
            window.applySnapshot(shaped(QSize(1080, 1920)));
            window.m_viewer->setChatWidth(450);
            emit window.m_chat->widthDragFinished();
            QCOMPARE(QSettings().value(QStringLiteral("viewer/chatWidthPhone")).toInt(), 450);
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->chatWidth(false), 330);
        QCOMPARE(window.m_viewer->chatWidth(true), 450);
    }

    // The panel's open and close move the stage over 220 ms: the panel
    // slides 24 to the right as it fades out of the stage, the device goes
    // to its closed place. The first layout has no motion.
    void panelOpensAndClosesWithMotion() {
        Stage stage(QSize(1920, 1080));
        stage.viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&stage.viewer));
        QVERIFY(!stage.viewer.moving());
        ViewerWidget::setMotionOverride(1);
        const QRect open = stage.viewer.stageLayout().panel;
        const double from = stage.viewer.stageLayout().body.left();
        stage.viewer.setChatOpen(false);
        QVERIFY(stage.viewer.moving());
        QVERIFY(stage.panel->isVisible());
        QTest::qWait(80);
        const ViewerWidget::StageLayout midway = stage.viewer.shownLayout();
        QVERIFY2(midway.body.left() > from && midway.body.left() < stage.viewer.stageLayout().body.left(),
                 qPrintable(QString::number(midway.body.left())));
        QVERIFY(midway.panel.left() > open.left() && midway.panel.left() <= open.left() + 24);
        // It fades as it slides, and paints as itself once the move ends.
        const auto fade = [&stage] { return qobject_cast<QGraphicsOpacityEffect *>(stage.panel->graphicsEffect()); };
        QVERIFY(fade());
        QVERIFY2(fade()->opacity() > 0.0 && fade()->opacity() < 1.0, qPrintable(QString::number(fade()->opacity())));
        QTRY_VERIFY(!stage.viewer.moving());
        QVERIFY(!stage.panel->isVisible());
        QVERIFY(!stage.panel->graphicsEffect());
        QCOMPARE(stage.viewer.shownLayout().body, stage.viewer.stageLayout().body);
        stage.viewer.setChatOpen(true);
        QVERIFY(stage.panel->isVisible());
        QVERIFY(fade() && fade()->opacity() < 0.5);
        QTRY_VERIFY(!stage.viewer.moving());
        QVERIFY(!stage.panel->graphicsEffect());
        QCOMPARE(stage.panel->geometry(), open);
        // Where nothing shows the motion (the offscreen tests), none runs.
        ViewerWidget::setMotionOverride(-1);
        if (QGuiApplication::platformName() == QLatin1String("offscreen")) {
            stage.viewer.setChatOpen(false);
            QVERIFY(!stage.viewer.moving());
            QVERIFY(!stage.panel->isVisible());
        }
    }

    // Dim surround: the stage's backdrop goes from #0e0e0e to #050505, from
    // the view.dim switch and from a workspace's stage (Finish dims).
    void dimFollowsTheSwitchAndTheFinishWorkspace() {
        {
            Stage stage(QSize(1920, 1080));
            QCOMPARE(stage.viewer.surroundColor().rgb(), qRgb(0x0e, 0x0e, 0x0e));
            QCOMPARE(pixelOf(stage.viewer.grab().toImage(), QPointF(4, 4)).rgb(), qRgb(0x0e, 0x0e, 0x0e));
            stage.viewer.setDim(true);
            QVERIFY(stage.viewer.dimmed());
            QCOMPARE(stage.viewer.surroundColor().rgb(), qRgb(0x05, 0x05, 0x05));
            QCOMPARE(pixelOf(stage.viewer.grab().toImage(), QPointF(4, 4)).rgb(), qRgb(0x05, 0x05, 0x05));
            // With motion, it fades over 220 ms.
            ViewerWidget::setMotionOverride(1);
            stage.viewer.setDim(false);
            QTest::qWait(60);
            const QColor midway = stage.viewer.surroundColor();
            QVERIFY2(midway.red() > 0x05 && midway.red() < 0x0e, qPrintable(midway.name()));
            QTRY_COMPARE(stage.viewer.surroundColor().rgb(), qRgb(0x0e, 0x0e, 0x0e));
            ViewerWidget::setMotionOverride(-1);
        }
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        QVERIFY(!window.m_viewer->dimmed());
        // The switch (view.dim, ⇧L) and the strip's button.
        QAction *dim = window.m_commands->action(QStringLiteral("view.dim"));
        QVERIFY(dim && dim->isCheckable());
        QCOMPARE(dim->shortcut(), QKeySequence(Qt::SHIFT | Qt::Key_L));
        emit window.m_commands->toggled(QStringLiteral("view.dim"), true);
        QVERIFY(window.m_viewer->dimmed() && dim->isChecked());
        QVERIFY(window.m_programStrip->buttonFor(dim)->isChecked());
        QVERIFY(window.m_workspace->stageState().dim);
        emit window.m_commands->toggled(QStringLiteral("view.dim"), false);
        QVERIFY(!window.m_viewer->dimmed() && !dim->isChecked());
        // Finish dims; Review does not.
        window.m_workspace->activate(QStringLiteral("Finish"));
        QVERIFY(window.m_viewer->dimmed() && dim->isChecked());
        window.m_workspace->activate(QStringLiteral("Review"));
        QVERIFY(!window.m_viewer->dimmed() && !dim->isChecked());
        window.m_workspace->activate(QStringLiteral("Finish"));
        QVERIFY(window.m_viewer->dimmed());
        window.m_workspace->activate(QStringLiteral("Review"));
    }

    // Safe zones over the picture in every format: the action-safe line
    // 3.5% in (93%) and title safe's corner ticks 5% in (90%) with the
    // cross at the centre. Off, the picture is untouched.
    void safeZonesOutlineActionAndTitleSafe() {
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920), QSize(1080, 1080)}) {
            Stage stage(canvas);
            QImage black(canvas / 8, QImage::Format_RGB32);
            black.fill(Qt::black);
            stage.viewer.setFrame(black, 0.0);
            QVERIFY(stage.viewer.safeZonesShown());
            const QRectF picture = stage.viewer.pictureRect();
            const SafeZones zones = safeZones(picture);
            KERF_NEAR(zones.action.width(), picture.width() * 0.93, 1e-6, "action safe");
            KERF_NEAR(zones.title.width(), picture.width() * 0.90, 1e-6, "title safe");
            KERF_NEAR(zones.action.left() - picture.left(), picture.width() * 0.035, 1e-6, "action inset");
            QCOMPARE(zones.cross.center(), picture.center());
            const QImage on = stage.viewer.grab().toImage();
            // The ticks at title safe's corners, the line along action safe,
            // the cross; the picture's black between.
            const QPointF tick = zones.title.topLeft() + QPointF(8, 0.5);
            const QPointF line = QPointF(zones.action.center().x(), zones.action.top() + 0.5);
            const QPointF cross = picture.center() + QPointF(-3, 0.5);
            for (const QPointF &point : {tick, line, cross})
                QVERIFY2(pixelOf(on, point).lightness() > 40, qPrintable(QStringLiteral("%1,%2").arg(point.x()).arg(point.y())));
            QVERIFY(pixelOf(on, zones.title.topLeft() + QPointF(8, 8)).lightness() < 8);
            // The ticks are brighter than the action line (.45 over .22).
            QVERIFY(pixelOf(on, tick).lightness() > pixelOf(on, line).lightness());
            stage.viewer.setSafeZones(false);
            const QImage off = stage.viewer.grab().toImage();
            for (const QPointF &point : {tick, line, cross}) QVERIFY(pixelOf(off, point).lightness() < 8);
        }
    }

    // The engine's caption sits centred with its foot on title safe's (UI
    // 15/20 white on rgba(0,0,0,.72), padding 2 8): one line on 16:9, cut
    // at title safe; wrapped and balanced on a phone.
    void captionSitsOnTitleSafe() {
        const QString text = QStringLiteral("We open before the city does, on the road out to the cliffs at dawn");
        {
            Stage stage(QSize(1920, 1080));
            QVERIFY(stage.viewer.captionRect().isEmpty());
            stage.viewer.setCaption(QStringLiteral("We open before the city does"));
            const QRectF picture = stage.viewer.pictureRect();
            const QRectF caption = stage.viewer.captionRect();
            KERF_NEAR(caption.bottom(), picture.bottom() - std::round(picture.height() * 0.05), 1e-6, "caption foot");
            KERF_NEAR(caption.center().x(), picture.center().x(), 0.5, "caption centre");
            QCOMPARE(caption.height(), 24.0);
            stage.viewer.setCaption(text + text);
            const QRectF long16 = stage.viewer.captionRect();
            QCOMPARE(long16.height(), 24.0);
            QVERIFY(long16.width() <= picture.width() - 2 * std::round(picture.width() * 0.05) + 0.5);
            // Drawn over the picture: its box is the caption's dark.
            QImage white(QSize(480, 270), QImage::Format_RGB32);
            white.fill(Qt::white);
            stage.viewer.setFrame(white, 0.0);
            stage.viewer.setSafeZones(false);
            const QColor under = pixelOf(stage.viewer.grab().toImage(), long16.topLeft() + QPointF(3, 1));
            QVERIFY2(under.lightness() < 90, qPrintable(under.name()));
        }
        {
            Stage stage(QSize(1080, 1920));
            stage.viewer.setCaption(text);
            const QRectF picture = stage.viewer.pictureRect();
            const QRectF caption = stage.viewer.captionRect();
            const double tx = std::round(picture.width() * 0.05);
            QVERIFY(caption.width() <= picture.width() - 2 * tx - 16 + 0.5);
            QVERIFY2(caption.height() >= 44.0, qPrintable(QString::number(caption.height())));
            QCOMPARE(int(caption.height() - 4) % 20, 0);
            KERF_NEAR(caption.bottom(), picture.bottom() - std::round(picture.height() * 0.05), 1e-6, "phone caption foot");
            // Balanced: the box is no wider than the widest balanced line needs,
            // well under the most it may take.
            QString lines;
            (void)stage.viewer.captionBox(&lines);
            const QStringList parts = lines.split(QLatin1Char('\n'));
            QVERIFY(parts.size() >= 2);
            const QFontMetricsF metrics(tokens::font(tokens::caption));
            QVERIFY2(metrics.horizontalAdvance(parts.constLast()) > 0.4 * metrics.horizontalAdvance(parts.constFirst()), qPrintable(lines));
        }
    }

    // The QC overlap box: from the last finished check, the text.overlap
    // finding whose comment is the current thread, while the playhead is in
    // it: the finding's rect on the picture (fill rgba(236,93,97,.32), a
    // neg line), its "8 px" tag and the caption's neg outline.
    void overlapBoxFollowsTheCurrentQcThread() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        viewer->setSafeZones(false);
        window.handleEvent(seedComments());
        window.handleEvent(overlapCheck());
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 17.32}, {"duration", 18.0}});
        QVERIFY(!viewer->overlapShown());
        emit window.bus()->currentThreadChanged(QStringLiteral("c-1708"));
        QVERIFY(viewer->overlapShown());
        QCOMPARE(viewer->overlap(), QRectF(0.30, 0.78, 0.20, 0.04));
        QCOMPARE(viewer->overlapTag(), QStringLiteral("8 px"));
        const QRectF picture = viewer->pictureRect();
        const QRectF box = viewer->overlapBox();
        KERF_NEAR(box.left(), picture.left() + 0.30 * picture.width(), 1e-6, "box x");
        KERF_NEAR(box.height(), 0.04 * picture.height(), 1e-6, "box h");
        QImage black(QSize(480, 270), QImage::Format_RGB32);
        black.fill(Qt::black);
        viewer->setFrame(black, 17.32);
        const QImage image = viewer->grab().toImage();
        const QColor inside = pixelOf(image, box.center());
        QVERIFY2(inside.red() > inside.green() + 30 && inside.red() > 50, qPrintable(inside.name()));
        // Its tag, without a caption over the box's right, 4 over it.
        const QRectF tag = viewer->overlapTagRect();
        KERF_NEAR(tag.right(), box.right(), 1e-6, "tag right");
        KERF_NEAR(tag.bottom(), box.top() - 4, 1e-6, "tag foot");
        QCOMPARE(tag.height(), 16.0);
        // With a caption, the tag stands over the caption's right.
        viewer->setCaption(QStringLiteral("We open before the city does"));
        KERF_NEAR(viewer->overlapTagRect().right(), viewer->captionRect().right(), 1e-6, "tag on the caption");
        // Another thread, or the playhead out of the finding: no box.
        emit window.bus()->currentThreadChanged(QStringLiteral("c-0903"));
        QVERIFY(!viewer->overlapShown());
        // Hovered in the panel, it shows while another thread is current.
        window.m_stageWiring->hoverThread(QStringLiteral("c-1708"));
        QVERIFY(viewer->overlapShown());
        window.m_stageWiring->hoverThread(QString());
        QVERIFY(!viewer->overlapShown());
        emit window.bus()->currentThreadChanged(QStringLiteral("c-1708"));
        QVERIFY(viewer->overlapShown());
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 9.0}, {"duration", 18.0}});
        QVERIFY(!viewer->overlapShown());
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 17.5}, {"duration", 18.0}});
        QVERIFY(viewer->overlapShown());
        // A check running keeps the last one's box; a new one without it clears it.
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.1}});
        QVERIFY(viewer->overlapShown());
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", true}, {"findings", QJsonArray{}}});
        QVERIFY(!viewer->overlapShown());
        window.close();
    }

    // The composer's Draw: the stage takes the pointer for strokes on the
    // frame; Return gives them back on the bus, normalised to the frame;
    // Esc (or the composer) cancels.
    void drawingOnTheFrameGoesBackOnTheBus() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        QSignalSpy finished(window.bus(), &EditorBus::drawingFinished);
        QSignalSpy cancelled(window.bus(), &EditorBus::drawingCancelled);
        QSignalSpy scrubs(viewer, &ViewerWidget::scrubbed);
        emit window.bus()->drawingRequested();
        QVERIFY(viewer->capturingDrawing());
        const QRectF picture = viewer->pictureRect();
        const auto at = [&picture](double x, double y) { return QPointF(picture.left() + x * picture.width(), picture.top() + y * picture.height()); };
        mouse(viewer, QEvent::MouseButtonPress, at(0.25, 0.25));
        for (int step = 1; step <= 10; ++step) {
            QMouseEvent move(QEvent::MouseMove, at(0.25 + 0.02 * step, 0.25 + 0.01 * step), viewer->mapToGlobal(at(0.25 + 0.02 * step, 0.25 + 0.01 * step)),
                             Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
            QCoreApplication::sendEvent(viewer, &move);
        }
        mouse(viewer, QEvent::MouseButtonRelease, at(0.45, 0.35));
        // The picture neither skims nor plays while it is drawn on.
        QVERIFY(scrubs.isEmpty());
        QCOMPARE(viewer->capturedStrokes().size(), 1);
        key(viewer, Qt::Key_Return);
        QVERIFY(!viewer->capturingDrawing());
        QCOMPARE(finished.size(), 1);
        const QJsonArray strokes = finished.constFirst().at(0).toJsonArray();
        QCOMPARE(strokes.size(), 1);
        const QJsonObject stroke = strokes.first().toObject();
        QCOMPARE(stroke.value("tool").toString(), QStringLiteral("pen"));
        const QJsonArray points = stroke.value("points").toArray();
        QVERIFY(points.size() >= 5);
        QVERIFY(near(points.first(), {0.25, 0.25}));
        for (const QJsonValue &point : points) {
            const QJsonArray xy = point.toArray();
            QVERIFY(xy.at(0).toDouble() >= 0.0 && xy.at(0).toDouble() <= 1.0 && xy.at(1).toDouble() >= 0.0 && xy.at(1).toDouble() <= 1.0);
        }
        QVERIFY(cancelled.isEmpty());
        // Esc cancels; nothing drawn is no drawing.
        emit window.bus()->drawingRequested();
        mouse(viewer, QEvent::MouseButtonPress, at(0.5, 0.5));
        mouse(viewer, QEvent::MouseButtonRelease, at(0.5, 0.5));
        key(viewer, Qt::Key_Escape);
        QVERIFY(!viewer->capturingDrawing());
        QCOMPARE(cancelled.size(), 1);
        emit window.bus()->drawingRequested();
        key(viewer, Qt::Key_Return);
        QCOMPARE(cancelled.size(), 2);
        QCOMPARE(finished.size(), 1);
        // The composer's own Cancel ends it once, without an echo.
        emit window.bus()->drawingRequested();
        QVERIFY(viewer->capturingDrawing());
        emit window.bus()->drawingCancelled();
        QVERIFY(!viewer->capturingDrawing());
        QCOMPARE(cancelled.size(), 3);
        window.close();
    }

    // The current thread's drawings show over the picture while the
    // playhead is at their comment (a reply at its thread's time), with the
    // author's 16 pt avatar at the start; view.annotations (⇧A) hides them.
    void annotationsShowTheCurrentThreadsDrawings() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        viewer->setSafeZones(false);
        window.handleEvent(seedComments());
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 17.32}, {"duration", 18.0}});
        QVERIFY(viewer->annotations().isEmpty());
        emit window.bus()->currentThreadChanged(QStringLiteral("c-1708"));
        QCOMPARE(viewer->annotations().size(), 1);
        QCOMPARE(viewer->annotations().first().comment, QStringLiteral("c-1708-r1"));
        QCOMPARE(viewer->annotations().first().initials, QStringLiteral("TL"));
        QVERIFY(viewer->annotationsShown());
        QImage black(QSize(480, 270), QImage::Format_RGB32);
        black.fill(Qt::black);
        viewer->setFrame(black, 17.32);
        const QRectF picture = viewer->pictureRect();
        const QPointF onStroke(picture.left() + 0.40 * picture.width(), picture.top() + 0.275 * picture.height());
        QVERIFY2(pixelOf(viewer->grab().toImage(), onStroke).lightness() > 150, qPrintable(pixelOf(viewer->grab().toImage(), onStroke).name()));
        // The avatar at the stroke's start: the on grey.
        const QPointF start(picture.left() + 0.30 * picture.width(), picture.top() + 0.30 * picture.height() - 6.5);
        QVERIFY(nearColor(pixelOf(viewer->grab().toImage(), start).rgb(), tokens::on, 6));
        // The switch hides them, and is remembered.
        emit window.m_commands->toggled(QStringLiteral("view.annotations"), false);
        QVERIFY(!viewer->annotationsShown());
        QVERIFY(!window.m_commands->action(QStringLiteral("view.annotations"))->isChecked());
        QVERIFY(pixelOf(viewer->grab().toImage(), onStroke).lightness() < 10);
        QCOMPARE(QSettings().value(QStringLiteral("viewer/annotations")).toBool(), false);
        emit window.m_commands->toggled(QStringLiteral("view.annotations"), true);
        // The reply chosen in the panel shows its drawing with another
        // thread current, at its thread's time.
        emit window.bus()->currentThreadChanged(QStringLiteral("c-0903"));
        QVERIFY(viewer->annotations().isEmpty());
        window.m_stageWiring->selectComment(QStringLiteral("c-1708-r1"));
        QCOMPARE(viewer->annotations().size(), 1);
        window.m_stageWiring->selectComment(QString());
        QVERIFY(viewer->annotations().isEmpty());
        emit window.bus()->currentThreadChanged(QStringLiteral("c-1708"));
        // Away from the comment's time, none.
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 12.0}, {"duration", 18.0}});
        QVERIFY(viewer->annotations().isEmpty());
        window.close();
    }

    // The Program strip (mockup-spec 2): "Program", the mask tools, then
    // Annotations, Safe zones and Dim, then Fit with its chevron; each a kit
    // button following its action, with the mockup's tips. The Fit menu sets
    // the viewer's zoom.
    void programStripShowsTheViewersSwitches() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        ProgramStrip *strip = window.m_programStrip;
        QCOMPARE(strip->height(), 28);
        const QList<QPair<QString, QString>> icons{{"maskRectangle", "g-mrect"}, {"maskEllipse", "g-mell"}, {"maskPen", "g-mpen"},
                                                   {"view.annotations", "g-annot"}, {"view.safeZones", "g-safe"}, {"view.dim", "g-dim"}};
        for (const auto &[name, icon] : icons) {
            QAction *action = nullptr;
            for (QAction *candidate : strip->actions())
                if (candidate && candidate->objectName() == name) action = candidate;
            QVERIFY2(action, qPrintable(name));
            auto *button = qobject_cast<ui::IconButton *>(strip->buttonFor(action));
            QVERIFY2(button, qPrintable(name));
            QCOMPARE(button->iconName(), icon);
            QCOMPARE(button->isEnabled(), action->isEnabled());
        }
        // Their keys: ⇧R ⇧E ⇧P for the masks.
        QCOMPARE(window.m_maskTools.at(0)->shortcut(), QKeySequence(Qt::SHIFT | Qt::Key_R));
        QCOMPARE(window.m_maskTools.at(1)->shortcut(), QKeySequence(Qt::SHIFT | Qt::Key_E));
        QCOMPARE(window.m_maskTools.at(2)->shortcut(), QKeySequence(Qt::SHIFT | Qt::Key_P));
        QAction *safe = window.m_commands->action(QStringLiteral("view.safeZones"));
        QAbstractButton *safeButton = strip->buttonFor(safe);
        QCOMPARE(ui::tipName(safeButton), QStringLiteral("Safe zones · action 93%, title 90%"));
        QCOMPARE(safeButton->isChecked(), window.m_viewer->safeZonesShown());
        safeButton->click();
        QCOMPARE(window.m_viewer->safeZonesShown(), safe->isChecked());
        QCOMPARE(safeButton->isChecked(), safe->isChecked());
        safeButton->click();
        QVERIFY(window.m_viewer->safeZonesShown());
        // A disabled one says why.
        QVERIFY(!window.m_maskTools.at(0)->isEnabled());
        QVERIFY(ui::tipName(strip->buttonFor(window.m_maskTools.at(0))).startsWith(QStringLiteral("Rectangle mask · ")));
        // Fit: the zoom's ghost button and its menu.
        QAbstractButton *fit = strip->zoomButton();
        QVERIFY(fit);
        QCOMPARE(strip->zoomText(), QStringLiteral("Fit"));
        QMenu *menu = window.m_stageWiring->zoomMenu();
        QStringList items;
        for (QAction *action : menu->actions())
            if (!action->isSeparator()) items << action->text();
        QCOMPARE(items, (QStringList{"Fit", "50%", "100%", "200%", "Sequence settings…"}));
        for (QAction *action : menu->actions())
            if (action->text() == QStringLiteral("100%")) action->trigger();
        QCOMPARE(window.m_viewer->zoom(), 1.0);
        QCOMPARE(strip->zoomText(), QStringLiteral("100%"));
        // At 100% a frame pixel is a device pixel.
        const qreal ratio = std::max(1.0, window.m_viewer->devicePixelRatioF());
        KERF_NEAR(window.m_viewer->pictureRect().width(), 1920.0 / ratio, 1e-6, "100% width");
        KERF_NEAR(window.m_viewer->pictureRect().center().x(), window.m_viewer->videoRect().center().x(), 1e-6, "zoom centre");
        window.m_stageWiring->setZoom(0.0);
        QCOMPARE(window.m_viewer->pictureRect(), window.m_viewer->videoRect());
        QCOMPARE(strip->zoomText(), QStringLiteral("Fit"));
        // The title's tip names the program.
        QVERIFY(ui::tipName(strip->findChild<QLabel *>(QStringLiteral("programStripTitle"))).contains(QStringLiteral("1920 × 1080")));
    }

    // The pointer on the picture skims across the picture (as the scrub's
    // bar maps it), so the frame skimmed is the one under the pointer's x;
    // a click on the picture plays or pauses and never seeks; off the
    // picture nothing happens.
    void pictureSkimsAcrossItselfAndAClickPlays() {
        Stage stage(QSize(1080, 1920));
        ViewerWidget &viewer = stage.viewer;
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
        QTest::mouseMove(&viewer, QPoint(qRound(card.right() + 4), qRound(card.top() - 30)));
        QCOMPARE(scrubs.size(), skimmed);
        QCOMPARE(ends.size(), 1);
        // A click on the picture plays; a click on the stage does nothing.
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, card.center().toPoint());
        QCOMPARE(controls.size(), 1);
        QCOMPARE(controls.constFirst()[0].value<PlayerOverlay::Control>(), PlayerOverlay::Control::Play);
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, QPoint(qRound(card.right() + 4), qRound(card.top() - 30)));
        QCOMPARE(controls.size(), 1);
        QVERIFY(seeks.isEmpty());
    }

    // The transport left the picture for the scrub strip under the device
    // (mockup-spec 3.4): nothing of a player lies over the picture (no
    // shade, no controls, no time), whatever it shows, and the overlay takes
    // no pointer; it still names the transport's commands for the window.
    void transportLeftThePicture() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ViewerWidget *viewer = window.m_viewer;
        viewer->setSafeZones(false);
        QImage white(QSize(480, 270), QImage::Format_RGB32);
        white.fill(Qt::white);
        viewer->setFrame(white, 0.0);
        QTest::mouseMove(viewer, viewer->videoRect().center().toPoint());
        const QImage image = viewer->grab().toImage();
        const QRectF picture = viewer->videoRect();
        for (const QPointF &point : {QPointF(picture.left() + 20, picture.bottom() - 6), QPointF(picture.center().x(), picture.bottom() - 20),
                                     QPointF(picture.left() + 30, picture.bottom() - 22), QPointF(picture.left() + 20, picture.top() + 20)})
            QCOMPARE(pixelOf(image, point).rgb(), qRgb(255, 255, 255));
        QVERIFY(!viewer->overlay()->mousePress(picture.center()));
        QVERIFY(viewer->overlay()->toolTipAt(picture.center()).isEmpty());
        QVERIFY(viewer->overlay()->controlTip(PlayerOverlay::Control::Play).startsWith(QStringLiteral("Play")));
        // The scrub strip is the transport, under the laptop.
        QWidget *scrub = window.m_stageWiring->scrub();
        QVERIFY(scrub->isVisible());
        QVERIFY(scrub->geometry().top() >= qRound(viewer->stageLayout().body.bottom()));
        window.close();
    }

    // A wide picture shows in a laptop by default, this Mac's preference
    // like the phone: Window ▸ Viewer (and the rail) choose Laptop or Off,
    // and the next editor opens with it. Off draws the plain picture where
    // the laptop's is; the rail and the scrub keep their places.
    void laptopPreviewIsRememberedOnThisMac() {
        {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(shaped(QSize(1920, 1080)));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            ViewerWidget *viewer = window.m_viewer;
            RailButton *device = window.m_rail->deviceButton();
            QVERIFY(device->isVisible() && device->isChecked());
            QCOMPARE(device->glyph(), RailButton::Glyph::Laptop);
            const ViewerWidget::StageLayout stage = viewer->stageLayout();
            QCOMPARE(stage.kind, ViewerWidget::Device::Laptop);
            QCOMPARE(window.m_laptopActions.size(), 2);
            QVERIFY(window.m_laptopActions.at(0)->isChecked() && !window.m_laptopActions.at(1)->isChecked());
            window.m_laptopActions.at(1)->trigger();
            QCOMPARE(viewer->laptopPreview(), LaptopPreview::Off);
            QCOMPARE(QSettings().value(QStringLiteral("viewer/laptopPreview")).toString(), QStringLiteral("off"));
            QVERIFY(!device->isChecked());
            const ViewerWidget::StageLayout off = viewer->stageLayout();
            QVERIFY(!off.device);
            QCOMPARE(off.card, stage.card);
            QCOMPARE(off.rail, stage.rail);
            QCOMPARE(off.scrub, stage.scrub);
            QCOMPARE(window.m_chat->geometry(), off.panel);
            // The rail's menu brings it back.
            emit window.m_rail->devicePreviewRequested(true);
            QCOMPARE(viewer->laptopPreview(), LaptopPreview::Laptop);
            QVERIFY(device->isChecked());
            window.m_laptopActions.at(1)->trigger();
            QVERIFY(!window.m_lastCommand.value("type").toString().contains(QStringLiteral("laptop")));
            window.close();
        }
        MainWindow window{QString(), QString()};
        QCOMPARE(window.m_viewer->laptopPreview(), LaptopPreview::Off);
        QVERIFY(window.m_laptopActions.at(1)->isChecked());
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
            // The group's title, flat: UI 500 (spec 0.3 has only 400 and 500).
            QCOMPARE(inspector->section(QStringLiteral("masks"))->findChild<QLabel *>(QStringLiteral("sectionTitle"))->font().weight(), QFont::Medium);
            for (QWidget *box : pair) {
                box->ensurePolished();
                auto *name = box->findChild<QToolButton *>(QStringLiteral("maskName"));
                name->ensurePolished();
                // The flat sheet's UI 500 (spec 0.3 has only 400 and 500).
                QCOMPARE(name->font().weight(), QFont::Medium);
            }
        }
        // A mask whose bottom edge runs within 20 pt of the picture's foot
        // draws its outline there while paused: the stage has no controls
        // on the picture for it to keep clear of.
        {
            const QJsonObject foot{{"name", "k1"}, {"rect", QJsonArray{0.2, 0.3, 0.8, 0.98}}, {"mode", "add"},
                                   {"inverted", false}, {"params", QJsonObject{}}};
            snapshot(10, QJsonArray{foot});
            viewer->selectMask(QString());
            QVERIFY(viewer->m_overlay->controlsShown());
            // A first grab lays the viewer out at its size in the window.
            viewer->grab();
            toWidget = viewer->pictureToWidget();
            const QPointF edge = toWidget.map(QPointF(0.5, 0.98));
            QVERIFY2(toWidget.map(QPointF(0.5, 1.0)).y() - edge.y() < 20.0, qPrintable(QString::number(edge.y())));
            // In the band the player's foot once kept outlines out of.
            QVERIFY(edge.y() > viewer->m_overlay->footTop() - 4.0);
            const QImage shot = viewer->grab().toImage();
            int brightest = 0;
            for (double dy = -2.0; dy <= 2.0; dy += 0.5)
                brightest = std::max(brightest, qGray(pixelOf(shot, edge + QPointF(0, dy)).rgb()));
            // The outline (white at 70%) against the screen just inside it.
            const int inside = qGray(pixelOf(shot, edge - QPointF(0, 10)).rgb());
            QVERIFY2(brightest > 100 && brightest - inside > 80,
                     qPrintable(QStringLiteral("%1 over %2").arg(brightest).arg(inside)));
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

    // The whole editor at the 1600 x 1000 canvas in the Review workspace,
    // its comments panel open in the stage, for each format: 16:9 in the
    // laptop beside the 368 panel, 9:16, 1:1 and 4:5 in the phone beside the
    // 480 panel with the Source column on the left, every part where the
    // [OWNER] table puts it; then the panel closed. With
    // KERF_SCREENSHOT_DIR set, each is saved as editor-<format>-dark.png and
    // editor-<format>-closed-dark.png, the 16:9 one with the laptop off as
    // editor-16x9-off-dark.png, and the 16:9 one dimmed as
    // editor-16x9-dim-dark.png.
    void editorShowsEveryFormatBesideItsConversation() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto picture = [](const QSize &size) { return testing::footageFrame(testing::Shot::HarborWide, size); };
        struct Format {
            const char *name;
            QSize canvas;
        };
        for (const Format &format : {Format{"16x9", QSize(1920, 1080)}, Format{"9x16", QSize(1080, 1920)}, Format{"1x1", QSize(1080, 1080)},
                                     Format{"4x5", QSize(1080, 1350)}}) {
            QSettings().remove(QStringLiteral("workspace"));
            QSettings().remove(QStringLiteral("viewer"));
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(shaped(format.canvas, 12));
            window.handleEvent(QJsonObject{{"type", "collab"}, {"status", sharedStatus()}});
            fakeArtwork(window.m_artwork);
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            window.m_errorBanner->hide();
            window.m_viewer->setFrame(picture(format.canvas / 4), 0.0);
            window.handleEvent(QJsonObject{{"type", "caption"}, {"text", "We open before the city does"}});
            QTest::mouseMove(window.statusBar(), QPoint(window.statusBar()->width() / 2, 4));
            QTest::qWait(30);
            const QString label = QString::fromLatin1(format.name);
            const bool wide = format.canvas.width() > format.canvas.height();
            ViewerWidget *viewer = window.m_viewer;
            QCOMPARE(window.m_canvasSize, format.canvas);
            QCOMPARE(viewer->size(), QSize(1296, 564));
            QVERIFY(viewer->isVisible() && window.m_chat->isVisible() && window.m_rail->isVisible());
            const ViewerWidget::StageLayout stage = viewer->stageLayout();
            QVERIFY2(!stage.overlay, qPrintable(label));
            QCOMPARE(stage.kind, wide ? ViewerWidget::Device::Laptop : ViewerWidget::Device::Phone);
            QCOMPARE(window.m_chat->geometry(), stage.panel);
            QCOMPARE(window.m_rail->geometry(), stage.rail);
            QCOMPARE(window.m_stageWiring->scrub()->geometry(), stage.scrub.toRect());
            if (wide) {
                KERF_NEAR(stage.body.left(), 85, 0.5, "x0");
                QCOMPARE(stage.panel, QRect(916, 8, 368, 548));
                QVERIFY(!window.m_stageSource->isVisible());
            } else {
                KERF_NEAR(stage.body.left(), 506.6, 0.5, "lx");
                QCOMPARE(stage.panel, QRect(804, 8, 480, 548));
                QCOMPARE(stage.rail.left(), 748);
                QVERIFY(window.m_stageSource->isVisible());
                QCOMPARE(window.m_stageSource->geometry(), stage.source);
                QCOMPARE(stage.source.width(), 400);
            }
            QVERIFY2(std::abs(stage.card.width() / stage.card.height() - double(format.canvas.width()) / format.canvas.height()) < 0.01,
                     qPrintable(label));
            // The band under the stage is the design's 344 for every shape.
            const int timelineHeight = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->height();
            QVERIFY2(std::abs(timelineHeight - (tokens::layout::band - 1)) <= 2, qPrintable(QStringLiteral("%1: timeline %2").arg(label).arg(timelineHeight)));
            const QImage image = window.grab().toImage();
            const qreal ratio = image.devicePixelRatio();
            const QPoint origin = viewer->mapTo(&window, QPoint());
            const auto at = [&](const QPointF &point) { return image.pixelColor((QPointF(origin) + point).toPoint() * ratio).rgb(); };
            // The surround round the device, between it and the rail.
            QCOMPARE(at(QPointF(stage.body.left() - 6, stage.body.top() + 8)), tokens::color(tokens::surround).rgb());
            QCOMPARE(at(QPointF(stage.body.right() + 6, stage.body.top() + 30)), tokens::color(tokens::surround).rgb());
            QCOMPARE(at(stage.body.topLeft() + QPointF(1, 1)), tokens::color(tokens::surround).rgb());
            // The screen's black over the picture.
            QCOMPARE(at(QPointF(stage.screen.left() + 40, (stage.screen.top() + stage.card.top()) / 2.0)), tokens::color(tokens::overlay::deviceScreen).rgb());
            QCOMPARE(amberPixels(image), 0);
            if (!shots.isEmpty()) QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("editor-%1-dark.png").arg(label))));
            if (wide) {
                // The laptop off: the plain picture at the same place.
                window.setLaptopPreview(LaptopPreview::Off);
                QCoreApplication::processEvents();
                QCOMPARE(viewer->stageLayout().card, stage.card);
                if (!shots.isEmpty()) QVERIFY(window.grab().toImage().save(QDir(shots).filePath(QStringLiteral("editor-16x9-off-dark.png"))));
                window.setLaptopPreview(LaptopPreview::Laptop);
                // Dimmed (Finish keeps its band; dim alone here).
                window.m_stageWiring->setDim(true);
                QCoreApplication::processEvents();
                const QImage dim = window.grab().toImage();
                QCOMPARE(dim.pixelColor((QPointF(origin) + QPointF(stage.body.left() - 6, stage.body.top() + 8)).toPoint() * ratio).rgb(),
                         tokens::color(tokens::surroundDim).rgb());
                if (!shots.isEmpty()) QVERIFY(dim.save(QDir(shots).filePath(QStringLiteral("editor-16x9-dim-dark.png"))));
                window.m_stageWiring->setDim(false);
            }
            // Closed: the rail at the stage's edge, the Source column on the left.
            window.setChatOpen(false);
            QCoreApplication::processEvents();
            QVERIFY(!window.m_chat->isVisible());
            const ViewerWidget::StageLayout closed = viewer->stageLayout();
            QCOMPARE(closed.rail.left(), 1240);
            KERF_NEAR(closed.body.left(), wide ? 533.5 : 998.6, 0.5, "closed");
            QVERIFY(window.m_stageSource->isVisible());
            QCOMPARE(window.m_stageSource->geometry(), closed.source);
            QCOMPARE(window.m_stageWiring->scrub()->geometry(), closed.scrub.toRect());
            const QImage shut = window.grab().toImage();
            QCOMPARE(amberPixels(shut), 0);
            if (!shots.isEmpty()) QVERIFY(shut.save(QDir(shots).filePath(QStringLiteral("editor-%1-closed-dark.png").arg(label))));
            window.setChatOpen(true);
            QCoreApplication::processEvents();
            QCOMPARE(viewer->videoRect(), stage.card);
            window.close();
        }
    }
};

#undef KERF_NEAR

} // namespace kerf

KERF_TEST_CLASS(kerf::StageTest)

#include "stage_test.moc"
