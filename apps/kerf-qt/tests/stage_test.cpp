// Tests for the picture region: its overlays, its switches, the program's marks
// and loop, the preview chip, the agent HUD, masks and formats. Where the region
// stands is stage_shell_test's.

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "editor_style.h"
#include "main_window.h"
#include "media_artwork.h"
#include "ui/components.h"
#include "ui/editor_bus.h"
#include "ui/wiring/stage_wiring.h"
#include "ui/wiring/timeline_wiring.h"
#include "ui/wiring/toolbar_wiring.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
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

// Within `tolerance` of `expected`, or a failure naming the value.
#define KERF_NEAR(actual, expected, tolerance, what)                                                                       \
    QVERIFY2(std::abs(double(actual) - double(expected)) <= (tolerance),                                                   \
             qPrintable(QStringLiteral("%1: %2, expected %3").arg(QLatin1String(what)).arg(double(actual), 0, 'f', 2).arg(double(expected))))

// The picture region at the design's size for its shape, with a stand-in HUD
// (hidden, as it is until a preview renders).
struct Stage {
    ViewerWidget viewer;
    QWidget *hud = new QWidget;
    explicit Stage(const QSize &canvas, const QSize &size = QSize()) {
        const bool wide = stage::isWide(double(canvas.width()) / canvas.height());
        viewer.resize(size.isValid() ? size : wide ? QSize(1200, 675) : QSize(int(std::lround(960.0 * canvas.width() / canvas.height())), 960));
        hud->hide();
        viewer.setOverlays(hud, nullptr);
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

    void registryCommandsDriveTheStage() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        CommandRegistry *commands = window.m_commands;
        StageWiring *stage = window.m_stageWiring.get();
        for (const char *id : {"view.annotations", "viewer.zoom", "playback.loop", "mark.in", "mark.out"})
            QVERIFY2(commands->action(QString::fromLatin1(id))->isEnabled(), id);
        const bool annotations = stage->annotations();
        commands->action(QStringLiteral("view.annotations"))->trigger();
        QCOMPARE(stage->annotations(), !annotations);
        commands->action(QStringLiteral("view.annotations"))->trigger();
        window.m_bus->setDuration(30.0);
        window.m_bus->setPlayhead(2.0);
        window.triggerCommand(QStringLiteral("mark.in"));
        QVERIFY(stage->hasIn());
        QCOMPARE(stage->inPoint(), 2.0);
        window.m_bus->setPlayhead(4.0);
        window.triggerCommand(QStringLiteral("mark.out"));
        QVERIFY(stage->hasOut());
        QCOMPARE(stage->outPoint(), 4.0);
        QCOMPARE(stage->loopStart(), 2.0);
        QCOMPARE(stage->loopEnd(), 4.0);
        // A mark past the other drops it.
        window.m_bus->setPlayhead(6.0);
        window.triggerCommand(QStringLiteral("mark.in"));
        QVERIFY(!stage->hasOut());
        // Clear Marks takes them off the ruler.
        window.triggerCommand(QStringLiteral("mark.clear"));
        QVERIFY(!stage->hasIn());
        QVERIFY(std::isnan(window.m_timeline->m_markIn));
        QAction *loop = commands->action(QStringLiteral("playback.loop"));
        QVERIFY(!stage->loop() && !loop->isChecked());
        loop->trigger();
        QVERIFY(stage->loop() && loop->isChecked());
        loop->trigger();
        QVERIFY(!stage->loop());
    }

    // With the loop on, playback that reaches the out mark goes back to the in
    // mark; playback that ends at the program's end starts again.
    void theLoopGoesBackToTheInMark() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(shaped(QSize(1920, 1080)));
        StageWiring *stage = window.m_stageWiring.get();
        window.m_duration = 30.0;
        window.m_bus->setDuration(30.0);
        window.m_bus->setPlayhead(2.0);
        stage->markIn();
        window.m_bus->setPlayhead(4.0);
        stage->markOut();
        stage->setLoop(true);
        window.m_lastCommand = {};
        window.m_bus->setPlaying(true);
        window.m_bus->setPlayhead(3.0);
        QCOMPARE(window.m_lastCommand, QJsonObject{});
        window.m_bus->setPlayhead(4.0);
        QCOMPARE(window.m_playhead, 2.0);
        // Stopped at the end of the loop: it plays again from the start of it.
        window.m_lastCommand = {};
        stage->clearMarks();
        window.m_duration = 30.0;
        window.m_bus->setPlayhead(30.0);
        window.m_bus->setPlaying(false);
        QCOMPARE(window.m_playhead, 0.0);
    }

    void agentHudCastsThePopShadow() {
        Stage stage(QSize(1920, 1080));
        QImage white(192, 108, QImage::Format_RGB32);
        white.fill(Qt::white);
        stage.viewer.setFrame(white, 0.0);
        stage.viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&stage.viewer));
        QVERIFY(stage.viewer.hudShadowRect().isEmpty());
        stage.hud->setAttribute(Qt::WA_NoSystemBackground);
        stage.hud->show();
        QCoreApplication::processEvents();
        const QRectF hud = QRectF(stage.viewer.hudRect());
        QCOMPARE(QRectF(stage.hud->geometry()), hud);
        // In title safe, at the picture's top right, on every format.
        const QRectF picture = stage.viewer.videoRect();
        KERF_NEAR(hud.right(), picture.right() - std::round(picture.width() * 0.05) - 8, 1.0, "HUD right");
        KERF_NEAR(hud.top(), picture.top() + std::round(picture.height() * 0.05) + 8, 1.0, "HUD top");
        // 0 8 24: 24 round it, 32 under it.
        const QMargins reach = ui::shadowMargins();
        QCOMPARE(stage.viewer.hudShadowRect(), hud.marginsAdded(QMarginsF(reach)));
        QVERIFY(reach.bottom() > reach.top());
        const QImage shot = stage.viewer.grab().toImage();
        const auto at = [&shot](const QPointF &point) { return pixelOf(shot, point).lightness(); };
        const int under = at(QPointF(hud.center().x(), hud.bottom() + 6));
        const int clear = at(QPointF(hud.center().x(), hud.bottom() + 60));
        QVERIFY2(under < clear - 20, qPrintable(QStringLiteral("%1 under, %2 clear").arg(under).arg(clear)));
        // A format change places it again at once, still inside the picture.
        stage.viewer.setProjectInfo(1080, 1920);
        stage.viewer.resize(540, 960);
        QCoreApplication::processEvents();
        QVERIFY(stage.viewer.videoRect().contains(QRectF(stage.hud->geometry())));
    }

    // "Previewing proposal vN", 12 inside the picture's top left, only while a
    // proposal is being previewed; Exit sends preview_exit.
    void thePreviewChipShowsOnlyWhilePreviewing() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = shaped(QSize(1920, 1080));
        snapshot.insert(QStringLiteral("proposals"), QJsonArray{QJsonObject{{"proposalId", "p1"}, {"version", 3}}});
        window.applySnapshot(snapshot);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        PreviewChip *chip = window.m_stageWiring->previewChip();
        QVERIFY(chip);
        QVERIFY(chip->isHidden());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}, {"proposalId", "p1"}});
        QCoreApplication::processEvents();
        QVERIFY(chip->isVisible());
        QCOMPARE(chip->text(), QStringLiteral("Previewing proposal v3"));
        QCOMPARE(chip->height(), 24);
        QCOMPARE(chip->pos(), window.m_viewer->videoRect().topLeft().toPoint() + QPoint(12, 12));
        chip->exitButton()->click();
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("type")).toString(), QStringLiteral("preview_exit"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "exited"}});
        QVERIFY(chip->isHidden());
        // The program's own preview is not a proposal's.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(chip->isHidden());
        // A fresh viewer ends it too.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "live"}, {"proposalId", "p1"}});
        QVERIFY(chip->isVisible());
        window.handleEvent(QJsonObject{{"type", "viewer_reset"}});
        QVERIFY(chip->isHidden());
        window.close();
    }

    void safeZonesOutlineActionAndTitleSafe() {
        // Off by default, on a new user's settings.
        {
            QSettings().remove(QStringLiteral("viewer/safeZones"));
            MainWindow window{QString(), QString()};
            QVERIFY(!window.m_viewer->safeZonesShown());
            QVERIFY(!window.m_commands->action(QStringLiteral("view.safeZones"))->isChecked());
            window.m_commands->action(QStringLiteral("view.safeZones"))->trigger();
            QVERIFY(window.m_viewer->safeZonesShown());
            QVERIFY(QSettings().value(QStringLiteral("viewer/safeZones")).toBool());
            QSettings().remove(QStringLiteral("viewer/safeZones"));
        }
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920), QSize(1080, 1080)}) {
            Stage stage(canvas);
            QImage black(canvas / 8, QImage::Format_RGB32);
            black.fill(Qt::black);
            stage.viewer.setFrame(black, 0.0);
            QVERIFY(!stage.viewer.safeZonesShown());
            stage.viewer.setSafeZones(true);
            const QRectF picture = stage.viewer.pictureRect();
            const QRectF action = picture.adjusted(picture.width() * 0.035, picture.height() * 0.035, -picture.width() * 0.035, -picture.height() * 0.035);
            const QRectF title = picture.adjusted(picture.width() * 0.05, picture.height() * 0.05, -picture.width() * 0.05, -picture.height() * 0.05);
            const QImage on = stage.viewer.grab().toImage();
            // The ticks at title safe's corners, the line along action safe,
            // the cross; the picture's black between.
            const QPointF tick = title.topLeft() + QPointF(8, 0.5);
            const QPointF line = QPointF(action.center().x(), action.top() + 0.5);
            const QPointF cross = picture.center() + QPointF(-3, 0.5);
            for (const QPointF &point : {tick, line, cross})
                QVERIFY2(pixelOf(on, point).lightness() > 40, qPrintable(QStringLiteral("%1,%2").arg(point.x()).arg(point.y())));
            QVERIFY(pixelOf(on, title.topLeft() + QPointF(8, 8)).lightness() < 8);
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

    void pictureSkimsAcrossItselfAndAClickPlays() {
        // Pillarboxed, so there is black beside the picture.
        Stage stage(QSize(1080, 1920), QSize(800, 960));
        ViewerWidget &viewer = stage.viewer;
        viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&viewer));
        const QRectF card = viewer.videoRect();
        QCOMPARE(card, QRectF(136, 12, 527, 936));
        QSignalSpy scrubs(&viewer, &ViewerWidget::scrubbed);
        QSignalSpy ends(&viewer, &ViewerWidget::scrubEnded);
        QSignalSpy plays(&viewer, &ViewerWidget::playRequested);
        QTest::mouseMove(&viewer, card.center().toPoint());
        QVERIFY(!scrubs.isEmpty());
        QVERIFY2(std::abs(scrubs.constLast()[0].toDouble() - 0.5) <= 0.02, qPrintable(QString::number(scrubs.constLast()[0].toDouble())));
        QTest::mouseMove(&viewer, QPoint(qRound(card.left() + card.width() * 0.25), qRound(card.center().y())));
        QVERIFY(std::abs(scrubs.constLast()[0].toDouble() - 0.25) <= 0.02);
        // Off the picture, on the black beside it: the skim ends.
        const qsizetype skimmed = scrubs.size();
        QTest::mouseMove(&viewer, QPoint(qRound(card.left() - 40), qRound(card.center().y())));
        QCOMPARE(scrubs.size(), skimmed);
        QCOMPARE(ends.size(), 1);
        // A click on the picture plays; a click on the black does nothing.
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, card.center().toPoint());
        QCOMPARE(plays.size(), 1);
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, QPoint(qRound(card.left() - 40), qRound(card.center().y())));
        QCOMPARE(plays.size(), 1);
        // While Play cannot run, a click does not ask for it.
        viewer.setPlayEnabled(false);
        QTest::mouseClick(&viewer, Qt::LeftButton, {}, card.center().toPoint());
        QCOMPARE(plays.size(), 1);
        // The click never moved the playhead: no skim came of it.
        QCOMPARE(scrubs.size(), skimmed);
    }

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
        // The transport is in the timeline's strip, its tips the commands'.
        TimelineToolStrip *strip = window.m_timelineWiring->tools();
        QVERIFY(strip->transportButton(TimelineToolStrip::Transport::Play)->isVisible());
        QVERIFY(ui::tipName(strip->transportButton(TimelineToolStrip::Transport::Play)).startsWith(QStringLiteral("Play")));
        window.close();
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
        window.selectClip(QStringLiteral("c1"));
        QVERIFY(rectangle->isEnabled());
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

        // A mask with mode none is a path only, still drawn and edited.
        snapshot(6, QJsonArray{QJsonObject{{"name", "k1"}, {"rect", QJsonArray{0.25, 0.25, 0.75, 0.75}}, {"mode", "none"},
                                           {"inverted", false}, {"params", QJsonObject{{"feather", 12.0}}}}});
        QCOMPARE(viewer->maskAt(toWidget.map(QPointF(0.5, 0.5))), QStringLiteral("k1"));
        // Delete on the picture removes the selected mask.
        viewer->selectMask(QStringLiteral("k1"));
        key(viewer, Qt::Key_Delete);
        QJsonObject removed = op();
        QCOMPARE(removed.value("op").toString(), QStringLiteral("mask-remove"));
        QCOMPARE(removed.value("name").toString(), QStringLiteral("k1"));

        // The pen closes an outline on its first point, and Return leaves
        // it open; names take the next free number.
        snapshot(8, {});
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
        // A mask whose bottom edge runs within 20 pt of the picture's foot
        // draws its outline there while paused: the stage has no controls
        // on the picture for it to keep clear of.
        {
            const QJsonObject foot{{"name", "k1"}, {"rect", QJsonArray{0.2, 0.3, 0.8, 0.98}}, {"mode", "add"},
                                   {"inverted", false}, {"params", QJsonObject{}}};
            snapshot(10, QJsonArray{foot});
            viewer->selectMask(QString());
            // A first grab lays the viewer out at its size in the window.
            viewer->grab();
            toWidget = viewer->pictureToWidget();
            const QPointF edge = toWidget.map(QPointF(0.5, 0.98));
            QVERIFY2(toWidget.map(QPointF(0.5, 1.0)).y() - edge.y() < 20.0, qPrintable(QString::number(edge.y())));
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
        // only a render matches, in the toolbar's subtitle for a moment.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "live"},
                                       {"message", "`a1` uses a crossfade, whose sound only a render plays exactly"}});
        QVERIFY(!window.m_previewRequired);
        QCOMPARE(window.m_previewStatus, QStringLiteral("live"));
        QVERIFY(window.m_toolbarWiring->subtitleText().contains(QStringLiteral("crossfade")));
        QCOMPARE(window.m_commands->reason(QStringLiteral("sequence.renderPreview")), QString());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(!window.m_commands->reason(QStringLiteral("sequence.renderPreview")).isEmpty());
        QCOMPARE(window.m_toolbarWiring->subtitleText(), QStringLiteral("Preview ready"));
    }


    // Accepting a proposal starts the program's playback again: what was showing stands in until its
    // first frame, where a blank "Your viewer" stood for up to two seconds. A window with nothing to
    // show does not keep the frame.
    void aNewPlaybackKeepsThePictureUntilItsFirstFrame() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject clip{{"id", "c1"}, {"kind", "solid"}, {"timelineStart", 0.0}, {"duration", 2.0}};
        const auto snapshot = [&](int revision, const QJsonArray &clips, int width) {
            return QJsonObject{{"revision", revision}, {"duration", clips.isEmpty() ? 0 : 2}, {"timeline", QJsonObject{{"fps", 24}, {"width", width}, {"height", 1080}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", clips}}}}}}};
        };
        window.applySnapshot(snapshot(1, QJsonArray{clip}, 1920));
        window.resize(1389, 868);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const auto blank = [&window] {
            TextRecorder painted;
            window.m_viewer->render(&painted);
            return painted.colorOf(QStringLiteral("Your viewer")).isValid();
        };
        QImage frame(1920, 1080, QImage::Format_RGB32);
        frame.fill(Qt::darkGreen);
        window.m_viewer->setFrame(frame, 0.0);
        QVERIFY(!blank());
        window.handleEvent(QJsonObject{{"type", "viewer_reset"}});
        QVERIFY(!blank());
        // Of another shape it would be stretched, and with nothing to play it would stay for good.
        window.applySnapshot(snapshot(2, QJsonArray{clip}, 1080));
        window.handleEvent(QJsonObject{{"type", "viewer_reset"}});
        QVERIFY(blank());
        window.m_viewer->setFrame(frame, 0.0);
        window.applySnapshot(snapshot(3, QJsonArray{}, 1920));
        window.handleEvent(QJsonObject{{"type", "viewer_reset"}});
        QVERIFY(blank());
        window.close();
    }
};

#undef KERF_NEAR

} // namespace kerf

KERF_TEST_CLASS(kerf::StageTest)

#include "stage_test.moc"
