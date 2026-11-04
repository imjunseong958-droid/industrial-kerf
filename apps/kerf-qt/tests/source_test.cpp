// Tests for the Source viewer: the Inspector sheet's Source page and the
// SourceState it shows (Q-SOURCE).

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_common.h"
#include "editor_style.h"
#include "effects_panel.h"
#include "inspector_panel.h"
#include "inspector_widgets.h"
#include "main_window.h"
#include "media_artwork.h"
#include "presets_panel.h"
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

    void clearInAndOut() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        window.openSource(QStringLiteral("m1"));
        SourceState *state = window.m_sourceWiring->state();
        state->setRange(2.0, 5.0);
        QVERIFY(state->isMarked());
        QAbstractButton *clearButton = window.m_sourceMonitor->button(QStringLiteral("source.clearMarks"));
        QVERIFY(qobject_cast<ui::Button *>(clearButton));
        QTRY_VERIFY(clearButton->isEnabled());
        clearButton->click();
        QCOMPARE(state->inPoint(), 0.0);
        QCOMPARE(state->outPoint(), 10.0);
        QVERIFY(!state->isMarked());
        state->setRange(2.0, 5.0);
        QTRY_VERIFY(clearButton->isEnabled());
        clearButton->click();
        QVERIFY(!state->isMarked());
        // ⇧⌘X is the registry's source.clearMarks.
        QAction *clear = window.m_commands->action(QStringLiteral("source.clearMarks"));
        QCOMPARE(clear->shortcut(), QKeySequence(QStringLiteral("Ctrl+Shift+X")));
        QTRY_COMPARE(ui::tipShortcut(clearButton), clear->shortcut().toString(QKeySequence::NativeText));
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
        SourceMonitorPanel pane(&state);
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

    void openInTimelineRevealsTheClip() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = sourceSnapshot();
        window.applySnapshot(snapshot);
        window.openSource(QStringLiteral("m1"));
        // Media that is not in the timeline cannot be shown there.
        QVERIFY(window.m_sourceMonitor->state()->revealClip().isEmpty());
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
        QVERIFY(!window.m_sourceMonitor->state()->revealClip().isEmpty());
        window.m_sourceMonitor->seek(5.0);
        window.m_commands->action(QStringLiteral("source.reveal"))->trigger();
        QCOMPARE(window.m_selectedClip, QStringLiteral("c2"));
        QCOMPARE(window.m_playhead, 7.0);
        // A position no clip shows goes to the first clip's start.
        window.m_sourceMonitor->seek(3.0);
        QVERIFY(window.m_sourceWiring->reveal());
        QCOMPARE(window.m_selectedClip, QStringLiteral("c1"));
        QCOMPARE(window.m_playhead, 1.0);
        // ⇧F and Space are the viewer's own while it has the focus.
        const auto overrides = [&window](QKeyCombination key) {
            QKeyEvent override(QEvent::ShortcutOverride, key.key(), key.keyboardModifiers());
            override.ignore();
            QCoreApplication::sendEvent(window.m_sourceMonitor->body(), &override);
            return override.isAccepted();
        };
        const QKeyCombination shiftF(Qt::ShiftModifier, Qt::Key_F);
        const QKeyCombination space(Qt::Key_Space);
        QVERIFY(overrides(shiftF));
        QVERIFY(overrides(space));
        QVERIFY(!overrides(QKeyCombination(Qt::Key_X)));
        // Media the timeline does not use leaves ⇧F to the window.
        window.openSource(QStringLiteral("m2"));
        QVERIFY(window.m_sourceMonitor->state()->revealClip().isEmpty());
        QVERIFY(!overrides(shiftF));
        QVERIFY(overrides(space));
        // With nothing open, Space is the window's program play.
        window.m_sourceMonitor->close();
        QVERIFY(!overrides(space));
        QVERIFY(!overrides(shiftF));
        // Media with no picture cannot play, so Space is the window's too.
        SourceState sound;
        sound.open(QJsonObject{{"id", "a1"}, {"path", "Sound.wav"}, {"duration", 3.0}, {"rate", "24"}, {"hasAudio", true}},
                   QStringLiteral("/nonexistent/Sound.wav"));
        SourceMonitorPanel viewer(&sound);
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
        SourceMonitorPanel late(state);
        QVERIFY(late.actions().contains(markIn));
        SourceMonitorPanel moved;
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

    // Play and Open in timeline are the registry's source.play and
    // source.reveal: the buttons run them, a key one gives them works while
    // a viewer has the focus, and their tips name it. Space and ⇧F stay the
    // viewer's own.
    void sourcePlayAndRevealAreRegistryCommands() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sourceSnapshot());
        QAction *play = window.m_commands->action(QStringLiteral("source.play"));
        QAction *reveal = window.m_commands->action(QStringLiteral("source.reveal"));
        QVERIFY(play && reveal);
        SourceState *state = window.m_sourceWiring->state();
        QCOMPARE(state->action(QStringLiteral("source.play")), play);
        QCOMPARE(state->action(QStringLiteral("source.reveal")), reveal);
        QVERIFY(window.m_commandHandlers.contains(QStringLiteral("source.play")));
        QVERIFY(window.m_commandHandlers.contains(QStringLiteral("source.reveal")));
        QCOMPARE(play->shortcutContext(), Qt::WidgetWithChildrenShortcut);
        QVERIFY(window.m_sourceMonitor->actions().contains(play));
        QVERIFY(window.m_sourceMonitor->actions().contains(reveal));
        window.openSource(QStringLiteral("m1"));
        QAbstractButton *button = window.m_sourceMonitor->button(QStringLiteral("source.play"));
        QTRY_VERIFY(button->isEnabled());
        QCOMPARE(ui::tipShortcut(button), QStringLiteral("Space"));
        QSignalSpy triggered(play, &QAction::triggered);
        QSignalSpy playing(state, &SourceState::playingChanged);
        button->click();
        QCOMPARE(triggered.size(), 1);
        QVERIFY(playing.size() >= 1);
        state->pause();
        // The command runs the state with no viewer in between.
        QVERIFY(window.m_sourceWiring->togglePlay());
        state->pause();
        // A key of one's own shows on the button.
        play->setShortcut(QKeySequence(Qt::Key_K));
        QTRY_COMPARE(ui::tipShortcut(button), QKeySequence(Qt::Key_K).toString(QKeySequence::NativeText));
        play->setShortcut(QKeySequence());
        QTRY_COMPARE(ui::tipShortcut(button), QStringLiteral("Space"));
        // Sound has no picture: the command does nothing.
        SourceState sound;
        sound.open(QJsonObject{{"id", "a1"}, {"path", "Sound.wav"}, {"duration", 3.0}, {"rate", "24"}, {"hasAudio", true}}, QString());
        QVERIFY(!sound.canPlay());
    }

    // The compact pane fits the Inspector column at its narrowest: the
    // canvas keeps its shape (216 x 122 at 240) and nothing runs past 12 in.
    void compactPaneFitsAt240() {
        SourceState state;
        SourceMonitorPanel pane(&state);
        pane.resize(240, 520);
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        state.open(QJsonObject{{"id", "m1"}, {"path", "missing.mov"}, {"duration", 9.0}, {"rate", "25"}, {"width", 1920}}, QString());
        QCoreApplication::processEvents();
        QCOMPARE(pane.pictureRect(), QRect(12, 12, 216, 122));
        QCOMPARE(pane.barRect(), QRect(12, 12 + 122 + 12, 216, 4));
        for (const char *id : {"source.markIn", "source.markOut", "source.previousFrame", "source.play", "source.nextFrame",
                               "source.insert", "source.overwrite", "source.clearMarks"}) {
            QAbstractButton *button = pane.button(QString::fromLatin1(id));
            QVERIFY2(button->isVisible(), id);
            QVERIFY2(button->geometry().left() >= 12 && button->geometry().right() + 1 <= 240 - 12, id);
        }
        // The transport stays centred.
        QAbstractButton *first = pane.button(QStringLiteral("source.markIn"));
        QAbstractButton *last = pane.button(QStringLiteral("source.nextFrame"));
        QCOMPARE(first->geometry().left() - 12, 240 - 12 - (last->geometry().right() + 1));
        QCOMPARE(pane.keyValueRect(0).width(), 240);
        // At the full column the canvas is the design's 280 x 158 again.
        pane.resize(304, 520);
        QCoreApplication::processEvents();
        QCOMPARE(pane.pictureRect(), QRect(12, 12, 280, 158));
    }

    // KERF_SCREENSHOT_DIR=<dir>: the pane on real media, saved for a look.
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
        state.setRange(2.2, 6.24);
        state.seek(3.72);
        QTRY_VERIFY_WITH_TIMEOUT(!state.frame().isNull(), 10000);
        QTest::qWait(300);
        SourceMonitorPanel pane(&state);
        pane.resize(303, 440);
        pane.show();
        QVERIFY(QTest::qWaitForWindowExposed(&pane));
        QTest::qWait(100);
        QVERIFY(pane.grab().save(QDir(shots).filePath(QStringLiteral("source-compact.png"))));
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
