// Tests for the Source viewer: the stage column's and the Inspector's compact
// pane, sharing one SourceState (Q-SOURCE).

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
#include "source_state.h"
#include "ui/components.h"
#include "wiring/source_wiring.h"
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

class SourceTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void sourceMarksInsertAndOverwriteOnTargetTracks() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "missing.mov"}, {"duration", 10.0},
                                               {"rate", "24"}, {"width", 1920}, {"height", 1080}, {"hasAudio", true}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{}}},
                                  QJsonObject{{"id", "A1"}, {"type", "audio"}, {"locked", true}, {"clips", QJsonArray{}}},
                                  QJsonObject{{"id", "A2"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}}}});
        QAction *insert = window.m_commands->action(QStringLiteral("source.insert"));
        QCOMPARE(window.m_commands->reason(QStringLiteral("source.insert")), QStringLiteral("Open media in the source monitor"));
        window.openSource(QStringLiteral("m1"));
        window.m_sourceMonitor->seek(1.0);
        window.m_commands->action(QStringLiteral("source.markIn"))->trigger();
        window.m_sourceMonitor->seek(2.0);
        window.m_commands->action(QStringLiteral("source.markOut"))->trigger();
        QCOMPARE(window.m_sourceMonitor->inPoint(), 1.0);
        QVERIFY(insert->isEnabled());
        window.queueSeek(0.5);
        insert->trigger();
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("insert"));
        QCOMPARE(op.value("media").toString(), QStringLiteral("m1"));
        QCOMPARE(op.value("sourceIn").toInt(), 24);
        // The out mark includes the frame on screen.
        QCOMPARE(op.value("frames").toInt(), 25);
        QCOMPARE(op.value("at").toInt(), 12);
        // Locked tracks are not targets.
        QCOMPARE(op.value("tracks").toArray(), (QJsonArray{"V1", "A2"}));
        window.m_commands->action(QStringLiteral("source.overwrite"))->trigger();
        QCOMPARE(window.m_lastCommand.value("ops").toArray().first().toObject().value("op").toString(), QStringLiteral("overwrite"));
    }

    void sourceMonitorShowsTheFrameAtItsPosition() {
        QTemporaryDir directory;
        const QString video = directory.filePath(QStringLiteral("bars.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-f"), QStringLiteral("lavfi"),
            QStringLiteral("-i"), QStringLiteral("testsrc2=size=320x180:rate=24"), QStringLiteral("-t"), QStringLiteral("2"),
            QStringLiteral("-pix_fmt"), QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        SourceMonitorPanel monitor;
        monitor.open(QJsonObject{{"id", "bars"}, {"path", "bars.mp4"}, {"duration", 2.0}, {"rate", "24"}, {"width", 320}}, video);
        QTRY_VERIFY_WITH_TIMEOUT(!monitor.frame().isNull(), 10000);
        monitor.step(12);
        QCOMPARE(monitor.position(), 0.5);
        // Positions stay before the last frame.
        monitor.seek(9.0);
        QCOMPARE(monitor.position(), 2.0 - 1.0 / 24);
        monitor.markOut();
        QCOMPARE(monitor.outPoint(), 2.0);
        monitor.close();
        QVERIFY(!monitor.isOpen());
    }

    void bothViewersFollowOneState() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        // One state, two forms: the Inspector's pane and the stage column.
        QVERIFY(window.m_sourceMonitor->state());
        QCOMPARE(window.m_stageSource->state(), window.m_sourceMonitor->state());
        QCOMPARE(window.m_sourceMonitor->mode(), SourceMonitorPanel::Mode::Compact);
        QCOMPARE(window.m_stageSource->mode(), SourceMonitorPanel::Mode::Stage);
        window.openSource(QStringLiteral("m1"));
        QCOMPARE(window.m_stageSource->mediaId(), QStringLiteral("m1"));
        // A mark set from the stage's button shows in the pane, through the
        // registry's command.
        window.m_stageSource->seek(1.0);
        QAbstractButton *markIn = window.m_stageSource->button(QStringLiteral("source.markIn"));
        QTRY_VERIFY(markIn->isEnabled());
        markIn->click();
        QCOMPARE(window.m_sourceMonitor->inPoint(), 1.0);
        window.m_sourceMonitor->seek(3.0);
        QCOMPARE(window.m_stageSource->position(), 3.0);
        window.m_sourceMonitor->button(QStringLiteral("source.markOut"))->click();
        QCOMPARE(window.m_stageSource->outPoint(), 3.0 + 1.0 / 24);
        QCOMPARE(window.m_stageSource->keyValues(), window.m_sourceMonitor->keyValues());
        QCOMPARE(window.m_sourceMonitor->keyValues(),
                 (QStringList{QStringLiteral("Clip"), QStringLiteral("00:00:01:00"), QStringLiteral("00:00:03:01"),
                              QStringLiteral("00:00:02:01")}));
        // The registry's source keys (I, O, ←, →, ⇧⌘X) work in the stage's
        // viewer too: it carries the same actions.
        QAction *markOut = window.m_commands->action(QStringLiteral("source.markOut"));
        QVERIFY(window.m_stageSource->actions().contains(markOut));
        QVERIFY(window.m_sourceMonitor->actions().contains(markOut));
        QVERIFY(!window.m_stageSource->actions().contains(window.m_commands->action(QStringLiteral("source.insert"))));
        // Insert in the stage's footer is the registry's Insert.
        QAbstractButton *insert = window.m_stageSource->button(QStringLiteral("source.insert"));
        QTRY_VERIFY(insert->isEnabled());
        insert->click();
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("insert"));
        QCOMPARE(op.value("sourceIn").toInt(), 24);
        QCOMPARE(op.value("frames").toInt(), 49);
        // Closing the media closes it in both.
        window.m_sourceMonitor->close();
        QVERIFY(!window.m_stageSource->isOpen());
        QTRY_VERIFY(!window.m_stageSource->button(QStringLiteral("source.markIn"))->isEnabled());
        QTRY_VERIFY(ui::tipName(window.m_stageSource->button(QStringLiteral("source.markIn"))).contains(QStringLiteral("Open media")));
    }

    void rangeRowsComeFromTheMediaIndex() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        window.openSource(QStringLiteral("m1"));
        SourceState *state = window.m_sourceWiring->state();
        // No bridge here, so nothing was asked and the viewer says why.
        QCOMPARE(state->indexStatus(), SourceState::IndexStatus::Unavailable);
        QCOMPARE(window.m_stageSource->rangeCount(), 0);
        // kerfd's index for this media, as the bridge sends it.
        window.handleEvent(QJsonObject{{"type", "media_index"}, {"media", "m1"}, {"cuts", QJsonArray{2.0, 6.5}},
                                       {"ranges", QJsonArray{QJsonObject{{"start", 6.5}, {"end", 10.0}},
                                                             QJsonObject{{"start", 0.0}, {"end", 2.0}},
                                                             QJsonObject{{"start", 2.0}, {"end", 6.5}}}},
                                       {"silences", QJsonArray{}}, {"loudest", QJsonArray{}}});
        QCOMPARE(state->indexStatus(), SourceState::IndexStatus::Ready);
        const QVector<SourceState::Range> ranges = state->ranges();
        QCOMPARE(ranges.size(), 3);
        // In order, named by their place: the engine names no shots.
        QCOMPARE(ranges.at(0).name, QStringLiteral("Shot 1"));
        QCOMPARE(ranges.at(0).end, 2.0);
        QCOMPARE(ranges.at(2).name, QStringLiteral("Shot 3"));
        QCOMPARE(ranges.at(2).start, 6.5);
        QCOMPARE(window.m_stageSource->rangeCount(), 3);
        QCOMPARE(window.m_sourceMonitor->rangeCount(), 3);
        // Each row names its last frame: neighbouring shots do not overlap.
        QCOMPARE(window.m_stageSource->rangeSpan(0), QStringLiteral("00:00:00:00–00:00:01:23"));
        QCOMPARE(window.m_stageSource->rangeSpan(1), QStringLiteral("00:00:02:00–00:00:06:11"));
        QCOMPARE(window.m_stageSource->rangeSpan(2), QStringLiteral("00:00:06:12–00:00:09:23"));
        // Another media's index is kept for when it opens.
        window.handleEvent(QJsonObject{{"type", "media_index"}, {"media", "m2"}, {"cuts", QJsonArray{1.0}}});
        QCOMPARE(state->ranges().size(), 3);
        QVERIFY(state->hasIndex(QStringLiteral("m2")));

        // A row sets in and out to its shot and goes to its start.
        SourceMonitorPanel panel(state, SourceMonitorPanel::Mode::Stage);
        panel.resize(505, 548);
        panel.show();
        QVERIFY(QTest::qWaitForWindowExposed(&panel));
        QCOMPARE(panel.rangeCount(), 3);
        const QRect row = panel.rangeRowRect(1);
        QVERIFY(panel.rangesRect().contains(row.center()));
        QTest::mouseClick(panel.body(), Qt::LeftButton, {}, row.center());
        QCOMPARE(state->inPoint(), 2.0);
        QCOMPARE(state->outPoint(), 6.5);
        QCOMPARE(state->position(), 2.0);
        QCOMPARE(window.m_sourceMonitor->inPoint(), 2.0);

        // Cuts alone make the shots between them.
        const QVector<SourceState::Range> fromCuts =
            SourceState::rangesFromIndex(QJsonObject{{"media", "m1"}, {"cuts", QJsonArray{4.0, 2.0, 12.0}}}, 10.0);
        QCOMPARE(fromCuts.size(), 3);
        QCOMPARE(fromCuts.at(0).start, 0.0);
        QCOMPARE(fromCuts.at(0).end, 2.0);
        QCOMPARE(fromCuts.at(2).end, 10.0);
        QVERIFY(SourceState::rangesFromIndex(QJsonObject{{"media", "m1"}}, 10.0).isEmpty());

        // A refusal shows in place of the rows.
        state->forgetIndex(QStringLiteral("m1"));
        window.handleEvent(QJsonObject{{"type", "error"}, {"command", "media_index"}, {"media", "m1"},
                                       {"message", "The media could not be read"}});
        QCOMPARE(state->indexStatus(), SourceState::IndexStatus::Failed);
        QCOMPARE(state->indexMessage(), QStringLiteral("The media could not be read"));
        QCOMPARE(panel.rangeCount(), 0);
        // An index with no cuts says so.
        window.handleEvent(QJsonObject{{"type", "media_index"}, {"media", "m1"}, {"cuts", QJsonArray{}}});
        QCOMPARE(state->indexStatus(), SourceState::IndexStatus::Ready);
        QCOMPARE(state->indexMessage(), QStringLiteral("No cuts in this media"));
    }

    void clearInAndOut() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        window.openSource(QStringLiteral("m1"));
        SourceState *state = window.m_sourceWiring->state();
        state->setRange(2.0, 5.0);
        QVERIFY(state->isMarked());
        // The stage's ghost and the pane's secondary button are the same command.
        QAbstractButton *stageClear = window.m_stageSource->button(QStringLiteral("source.clearMarks"));
        QAbstractButton *paneClear = window.m_sourceMonitor->button(QStringLiteral("source.clearMarks"));
        QVERIFY(qobject_cast<ui::GhostButton *>(stageClear));
        QVERIFY(qobject_cast<ui::Button *>(paneClear));
        QTRY_VERIFY(stageClear->isEnabled());
        stageClear->click();
        QCOMPARE(state->inPoint(), 0.0);
        QCOMPARE(state->outPoint(), 10.0);
        QVERIFY(!state->isMarked());
        state->setRange(2.0, 5.0);
        paneClear->click();
        QVERIFY(!state->isMarked());
        // ⇧⌘X is the registry's source.clearMarks.
        QAction *clear = window.m_commands->action(QStringLiteral("source.clearMarks"));
        QCOMPARE(clear->shortcut(), QKeySequence(QStringLiteral("Ctrl+Shift+X")));
        QTRY_COMPARE(ui::tipShortcut(stageClear), clear->shortcut().toString(QKeySequence::NativeText));
        state->setRange(2.0, 5.0);
        clear->trigger();
        QVERIFY(!state->isMarked());
        // A range is kept in frames and within the media.
        state->setRange(-3.0, 40.0);
        QCOMPARE(state->inPoint(), 0.0);
        QCOMPARE(state->outPoint(), 10.0);
        state->setRange(4.0, 4.0);
        QCOMPARE(state->outPoint(), 4.0 + 1.0 / 24);
    }

    void compactGeometry() {
        SourceState state;
        SourceMonitorPanel pane(&state, SourceMonitorPanel::Mode::Compact);
        pane.resize(304, 600);
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        state.open(QJsonObject{{"id", "m1"}, {"path", "media/Room tone.wav"}, {"duration", 9.0}, {"rate", "25"}},
                   QString());
        // mockup-spec 5.2: pad 12, a 280 x 158 canvas, the bar 12 under it,
        // the timecodes 8 under that, the transport 8 under them.
        QCOMPARE(pane.pictureRect(), QRect(12, 12, 280, 158));
        QCOMPARE(pane.barRect(), QRect(12, 182, 280, 4));
        QCOMPARE(pane.transportRect().top(), 218);
        // The transport is centred: five 24 buttons, 4 apart.
        QAbstractButton *first = pane.button(QStringLiteral("source.markIn"));
        QAbstractButton *last = pane.button(QStringLiteral("source.nextFrame"));
        QCOMPARE(first->geometry().size(), QSize(24, 24));
        QCOMPARE(last->geometry().right() + 1 - first->geometry().left(), 5 * 24 + 4 * 4);
        QCOMPARE(first->geometry().left() - 12, 304 - 12 - (last->geometry().right() + 1));
        QCOMPARE(first->geometry().top(), 218);
        // Insert, Overwrite and Clear in and out, 12 under, 8 apart, on one row.
        QAbstractButton *insert = pane.button(QStringLiteral("source.insert"));
        QAbstractButton *overwrite = pane.button(QStringLiteral("source.overwrite"));
        QAbstractButton *clear = pane.button(QStringLiteral("source.clearMarks"));
        QCOMPARE(insert->geometry().topLeft(), QPoint(12, 254));
        QCOMPARE(overwrite->geometry().left(), insert->geometry().right() + 1 + 8);
        QCOMPARE(clear->geometry().left(), overwrite->geometry().right() + 1 + 8);
        QCOMPARE(clear->geometry().top(), 254);
        QVERIFY(clear->isVisible());
        QVERIFY(!pane.button(QStringLiteral("source.reveal"))->isVisible());
        // "Source clip" (28) 4 under the pad, then 24 pt key-values.
        QCOMPARE(pane.sectionRect(), QRect(0, 294, 304, 28));
        QCOMPARE(pane.keyValueRect(0), QRect(0, 322, 304, 24));
        QCOMPARE(pane.keyValueRect(3), QRect(0, 394, 304, 24));
        QCOMPARE(pane.sizeHint().height(), 418);
        QCOMPARE(pane.keyValues(), (QStringList{QStringLiteral("Room tone"), QStringLiteral("00:00:00:00"),
                                                QStringLiteral("00:00:09:00"), QStringLiteral("00:00:09:00")}));
        // Sound has no picture to play.
        QVERIFY(!pane.button(QStringLiteral("source.play"))->isEnabled());
        QTRY_VERIFY(ui::tipName(pane.button(QStringLiteral("source.play"))).contains(QStringLiteral("no picture")));
        // Without the registry an edit cannot run; the marks still work.
        QVERIFY(!insert->isEnabled());
        state.seek(2.0);
        pane.button(QStringLiteral("source.markIn"))->click();
        QCOMPARE(state.inPoint(), 2.0);
        // The timecodes under the bar are the in and out marks.
        QCOMPARE(pane.compactTimecodes(), (QPair<QString, QString>{QStringLiteral("00:00:02:00"), QStringLiteral("00:00:09:00")}));
        // The canvas is black, on the Inspector's chrome.
        const QImage image = pane.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        QCOMPARE(image.pixelColor(QPoint(150, 60) * ratio).rgb(), tokens::color(tokens::overlay::deviceScreen).rgb());
        QCOMPARE(image.pixelColor(QPoint(4, 100) * ratio).rgb(), tokens::color(tokens::chrome).rgb());
        // Shorter than its content, the pane scrolls.
        pane.resize(304, 200);
        QTRY_VERIFY(pane.m_scroll->verticalScrollBar()->maximum() > 0);
    }

    void stageGeometry() {
        // mockup-spec 3.6: the picture is the column less 32 wide, 16:9 while
        // that fits in colH - 280, else 9 * floor((colH - 280) / 9) tall and
        // pillarboxed, centred.
        SourceMonitorPanel::StagePicture fits = SourceMonitorPanel::stagePicture(QSize(505, 548));
        QCOMPARE(fits.box, QRect(16, 40, 473, 266));
        QCOMPARE(fits.picture, fits.box);
        SourceMonitorPanel::StagePicture shortColumn = SourceMonitorPanel::stagePicture(QSize(505, 500));
        QCOMPARE(shortColumn.box, QRect(16, 40, 473, 216));
        QCOMPARE(shortColumn.picture, QRect(16 + (473 - 384) / 2, 40, 384, 216));
        SourceState state;
        SourceMonitorPanel viewer(&state, SourceMonitorPanel::Mode::Stage);
        viewer.resize(505, 548);
        viewer.show();
        QVERIFY(QTest::qWaitForWindowExposed(&viewer));
        state.open(QJsonObject{{"id", "m1"}, {"path", "missing.mov"}, {"duration", 9.0}, {"rate", "25"}, {"width", 1920}}, QString());
        QCOMPARE(viewer.headerRect(), QRect(0, 0, 505, 40));
        QCOMPARE(viewer.pictureRect(), fits.box);
        // Bar 12 under the picture; transport 4 under the bar (28); the
        // filmstrip 12 under that (40); the rows 8 under it; a 48 footer.
        QCOMPARE(viewer.barRect(), QRect(16, 40 + 266 + 12, 473, 4));
        QCOMPARE(viewer.transportRect(), QRect(0, 40 + 266 + 12 + 4 + 4, 505, 28));
        QCOMPARE(viewer.filmstripRect(), QRect(16, viewer.transportRect().bottom() + 1 + 12, 473, 40));
        QCOMPARE(viewer.rangesRect().top(), viewer.filmstripRect().bottom() + 1 + 8);
        QCOMPARE(viewer.footerRect(), QRect(0, 548 - 48, 505, 48));
        QCOMPARE(viewer.rangesRect().bottom() + 1, viewer.footerRect().top());
        // Open in timeline sits at the header's right (pad 12).
        QAbstractButton *reveal = viewer.button(QStringLiteral("source.reveal"));
        QCOMPARE(reveal->geometry(), QRect(505 - 12 - 24, 8, 24, 24));
        // The footer: Insert, Overwrite at the left, Clear in and out at the right.
        QCOMPARE(viewer.button(QStringLiteral("source.insert"))->geometry().topLeft(), QPoint(16, 500 + 12));
        QCOMPARE(viewer.button(QStringLiteral("source.clearMarks"))->geometry().right() + 1, 505 - 16);
        // The transport's length is 88 wide at the right; the buttons sit between.
        QAbstractButton *next = viewer.button(QStringLiteral("source.nextFrame"));
        QVERIFY(next->geometry().right() < 505 - 16 - 88);
        QCOMPARE(next->geometry().top(), viewer.transportRect().top() + 2);
        // The card and the black picture box, the bar's marked range in track2.
        state.setRange(0.0, 4.5);
        state.seek(6.0);
        const QImage image = viewer.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        QCOMPARE(image.pixelColor(QPoint(200, 20) * ratio).rgb(), tokens::color(tokens::card).rgb());
        QCOMPARE(image.pixelColor(QPoint(200, 100) * ratio).rgb(), tokens::color(tokens::overlay::deviceScreen).rgb());
        const QRect bar = viewer.barRect();
        QCOMPARE(image.pixelColor(QPoint(bar.left() + bar.width() / 4, bar.top() + 2) * ratio).rgb(), tokens::color(tokens::track2).rgb());
        QCOMPARE(image.pixelColor(QPoint(bar.left() + bar.width() * 9 / 10, bar.top() + 2) * ratio).rgb(),
                 tokens::color(tokens::track).rgb());
        // The position's tick stands over the bar in the playhead colour.
        const int tick = bar.left() + int(bar.width() * 6.0 / 9.0);
        QCOMPARE(image.pixelColor(QPoint(tick, bar.top() - 3) * ratio).rgb(), tokens::color(tokens::playhead).rgb());
        // Phone width: the same rule in a 400 column.
        viewer.resize(400, 548);
        QCOMPARE(viewer.pictureRect(), QRect(16, 40, 368, 207));
        QCOMPARE(viewer.sizeHint(), QSize(400, 548));
        QCOMPARE(viewer.minimumSizeHint(), QSize(0, 0));
    }

    void openInTimelineRevealsTheClip() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = sourceSnapshot();
        window.applySnapshot(snapshot);
        window.openSource(QStringLiteral("m1"));
        QAbstractButton *reveal = window.m_stageSource->button(QStringLiteral("source.reveal"));
        // Media that is not in the timeline cannot be shown there.
        QTRY_VERIFY(!reveal->isEnabled());
        QTRY_VERIFY(ui::tipName(reveal).contains(QStringLiteral("not in the timeline")));
        QVERIFY(!window.m_sourceWiring->reveal());
        // Two clips of it: the second shows source 4 s to 7 s from program 6 s.
        QJsonObject timeline = snapshot.value("timeline").toObject();
        timeline.insert("tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 1.0}, {"in", 0.0}, {"duration", 2.0}},
                QJsonObject{{"id", "c2"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 6.0}, {"in", 4.0}, {"duration", 3.0}}}}},
            QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{}}}});
        snapshot.insert("timeline", timeline);
        snapshot.insert("revision", 3);
        snapshot.insert("duration", 12);
        window.applySnapshot(snapshot);
        QTRY_VERIFY(reveal->isEnabled());
        QTRY_COMPARE(ui::tipShortcut(reveal), QStringLiteral("⇧F"));
        window.m_stageSource->seek(5.0);
        reveal->click();
        QCOMPARE(window.m_selectedClip, QStringLiteral("c2"));
        QCOMPARE(window.m_playhead, 7.0);
        // A position no clip shows goes to the first clip's start.
        window.m_stageSource->seek(3.0);
        QVERIFY(window.m_sourceWiring->reveal());
        QCOMPARE(window.m_selectedClip, QStringLiteral("c1"));
        QCOMPARE(window.m_playhead, 1.0);
        // ⇧F and Space are the viewer's own while it has the focus.
        const auto overrides = [&window](QKeyCombination key) {
            QKeyEvent override(QEvent::ShortcutOverride, key.key(), key.keyboardModifiers());
            override.ignore();
            QCoreApplication::sendEvent(window.m_stageSource->body(), &override);
            return override.isAccepted();
        };
        const QKeyCombination shiftF(Qt::ShiftModifier, Qt::Key_F);
        const QKeyCombination space(Qt::Key_Space);
        QVERIFY(overrides(shiftF));
        QVERIFY(overrides(space));
        QVERIFY(!overrides(QKeyCombination(Qt::Key_X)));
        // Media the timeline does not use leaves ⇧F to the window.
        window.openSource(QStringLiteral("m2"));
        QVERIFY(window.m_stageSource->state()->revealClip().isEmpty());
        QVERIFY(!overrides(shiftF));
        QVERIFY(overrides(space));
        // With nothing open, Space is the window's program play.
        window.m_stageSource->close();
        QVERIFY(!overrides(space));
        QVERIFY(!overrides(shiftF));
        // Media with no picture cannot play, so Space is the window's too.
        SourceState sound;
        sound.open(QJsonObject{{"id", "a1"}, {"path", "Sound.wav"}, {"duration", 3.0}, {"rate", "24"}, {"hasAudio", true}},
                   QStringLiteral("/nonexistent/Sound.wav"));
        SourceMonitorPanel viewer(&sound, SourceMonitorPanel::Mode::Stage);
        QVERIFY(sound.isOpen());
        QVERIFY(!sound.canPlay());
        QKeyEvent soundSpace(QEvent::ShortcutOverride, Qt::Key_Space, Qt::NoModifier);
        soundSpace.ignore();
        QCoreApplication::sendEvent(viewer.body(), &soundSpace);
        QVERIFY(!soundSpace.isAccepted());
    }

    // A viewer attached after the registry's actions were set carries them
    // as well; one moved to another state drops the old state's.
    void lateViewerCarriesTheSourceKeys() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        SourceState *state = window.m_sourceWiring->state();
        QAction *markIn = window.m_commands->action(QStringLiteral("source.markIn"));
        QVERIFY(state->actions().contains(markIn));
        SourceMonitorPanel late(state, SourceMonitorPanel::Mode::Stage);
        QVERIFY(late.actions().contains(markIn));
        SourceMonitorPanel moved(nullptr, SourceMonitorPanel::Mode::Stage);
        QVERIFY(!moved.actions().contains(markIn));
        moved.setState(state);
        QVERIFY(moved.actions().contains(markIn));
        SourceState other;
        moved.setState(&other);
        QVERIFY(!moved.actions().contains(markIn));
    }


    void sourcePlaysItsPictureToTheOutMark() {
        QTemporaryDir directory;
        const QString video = directory.filePath(QStringLiteral("bars.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-f"), QStringLiteral("lavfi"),
            QStringLiteral("-i"), QStringLiteral("testsrc2=size=320x180:rate=24"), QStringLiteral("-t"), QStringLiteral("2"),
            QStringLiteral("-pix_fmt"), QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        SourceState state;
        state.open(QJsonObject{{"id", "bars"}, {"path", "bars.mp4"}, {"duration", 2.0}, {"rate", "24"}, {"width", 320}, {"height", 180}},
                   video);
        QCOMPARE(SourceState::streamSize(320, 180), QSize(320, 180));
        QCOMPARE(SourceState::streamSize(1920, 1080), QSize(640, 360));
        QVERIFY(state.canPlay());
        QSignalSpy playing(&state, &SourceState::playingChanged);
        state.play();
        QVERIFY(state.isPlaying());
        QTRY_VERIFY_WITH_TIMEOUT(state.position() >= 0.25, 10000);
        QVERIFY(!state.frame().isNull());
        QCOMPARE(state.frame().size(), QSize(320, 180));
        state.pause();
        QVERIFY(!state.isPlaying());
        const double paused = state.position();
        QTest::qWait(200);
        QCOMPARE(state.position(), paused);
        // Inside a marked range, play stops at its out mark.
        state.setRange(0.5, 1.0);
        state.seek(0.5);
        state.play();
        QTRY_VERIFY_WITH_TIMEOUT(!state.isPlaying(), 10000);
        QVERIFY(state.position() < 1.0);
        QVERIFY(state.position() >= 0.5);
        // The stop leaves a full-size still on screen.
        QTRY_COMPARE_WITH_TIMEOUT(state.frame().size(), QSize(320, 180), 10000);
        // Where play stopped (the range's last frame), Space again starts
        // over from the in mark and plays on past one frame.
        state.play();
        QVERIFY(state.isPlaying());
        QCOMPARE(state.position(), 0.5);
        QTRY_VERIFY_WITH_TIMEOUT(state.position() >= 0.5 + 2.0 / 24 || !state.isPlaying(), 10000);
        QVERIFY(state.position() >= 0.5 + 2.0 / 24);
        QTRY_VERIFY_WITH_TIMEOUT(!state.isPlaying(), 10000);
        // Mark out on a frame leaves the playhead on the range's last frame:
        // play starts over from the in mark there too.
        state.seek(0.75);
        state.markOut();
        QCOMPARE(state.outPoint(), 0.75 + 1.0 / 24);
        state.play();
        QCOMPARE(state.position(), 0.5);
        state.pause();
        // At the out mark itself, the same.
        state.seek(1.0);
        state.setRange(0.5, 1.0);
        state.seek(1.0);
        state.play();
        QCOMPARE(state.position(), 0.5);
        // Seeking stops it.
        state.seek(0.25);
        QVERIFY(!state.isPlaying());
        QVERIFY(playing.count() >= 4);
    }

    // KERF_SCREENSHOT_DIR=<dir>: the stage viewer and the compact pane on
    // real media, its filmstrip and a fixture index, saved for a look.
    void sourceGallery() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("Set KERF_SCREENSHOT_DIR to save the Source viewer's screenshots");
        if (QStandardPaths::findExecutable(QStringLiteral("ffmpeg")).isEmpty()) QSKIP("FFmpeg unavailable");
        QTemporaryDir directory;
        QDir().mkpath(directory.filePath(QStringLiteral("media")));
        const QString video = directory.filePath(QStringLiteral("media/B003_C011.mp4"));
        QCOMPARE(QProcess::execute(QStringLiteral("ffmpeg"), {
            QStringLiteral("-v"), QStringLiteral("error"), QStringLiteral("-f"), QStringLiteral("lavfi"),
            QStringLiteral("-i"), QStringLiteral("testsrc2=size=640x360:rate=25"), QStringLiteral("-t"), QStringLiteral("9.8"),
            QStringLiteral("-pix_fmt"), QStringLiteral("yuv420p"), QStringLiteral("-y"), video}), 0);
        const QJsonObject source{{"id", "m1"}, {"path", "media/B003_C011.mp4"}, {"duration", 9.8}, {"rate", "25"},
                                 {"width", 640}, {"height", 360}};
        MediaArtwork artwork;
        SourceState state;
        state.setArtwork(&artwork);
        artwork.request(QJsonArray{source}, directory.path());
        QTRY_VERIFY_WITH_TIMEOUT(!artwork.strips.value(QStringLiteral("m1")).isEmpty(), 30000);
        state.open(source, video);
        state.applyIndex(QJsonObject{{"media", "m1"}, {"cuts", QJsonArray{2.2, 4.4, 5.64, 6.48, 8.88}}});
        state.setRange(2.2, 6.24);
        state.seek(3.72);
        QTRY_VERIFY_WITH_TIMEOUT(!state.frame().isNull(), 10000);
        QTest::qWait(300);
        QWidget surround;
        surround.setStyleSheet(QStringLiteral("background: #0e0e0e"));
        surround.resize(537, 564);
        SourceMonitorPanel stage(&state, SourceMonitorPanel::Mode::Stage, &surround);
        stage.setGeometry(16, 8, 505, 548);
        SourceMonitorPanel pane(&state, SourceMonitorPanel::Mode::Compact);
        pane.resize(303, 440);
        surround.show();
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&surround));
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        QTest::qWait(100);
        QDir().mkpath(shots);
        QVERIFY(surround.grab().save(QDir(shots).filePath(QStringLiteral("source-stage.png"))));
        QVERIFY(pane.grab().save(QDir(shots).filePath(QStringLiteral("source-compact.png"))));
        surround.resize(537, 470);
        stage.setGeometry(16, 8, 505, 454);
        QTest::qWait(50);
        QVERIFY(surround.grab().save(QDir(shots).filePath(QStringLiteral("source-stage-short.png"))));
    }

private:
    static QJsonObject sourceSnapshot() {
        return QJsonObject{{"revision", 2}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24},
            {"sources", QJsonArray{QJsonObject{{"id", "m1"}, {"path", "Clip.mov"}, {"duration", 10.0},
                                               {"rate", "24"}, {"width", 1920}, {"height", 1080}, {"hasAudio", true}},
                                   QJsonObject{{"id", "m2"}, {"path", "Other.mov"}, {"duration", 4.0},
                                               {"rate", "24"}, {"width", 1920}, {"height", 1080}}}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{}}},
                                  QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}}}};
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::SourceTest)

#include "source_test.moc"
