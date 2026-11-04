// Tests for the timeline (mockup-spec 7): the bar's tools, zoom and track
// heights; the heads' patch, target, toggles, solo and sync-lock; the ruler's
// ticks, comment marks, proposal bands and markers; flat clips, filmstrips,
// waveforms and the rubber band; QC dots, veils and preview ghosts; and the
// performance contracts (lanes cached, painted by rows in view, scrolled by
// moving what is on screen).

#include "fixtures.h"
#include "test_registry.h"

#include "ask_prompt.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
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
#include "test_footage.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "viewer_widget.h"
#include "ui/stage_shell.h"
#include "wiring/timeline_wiring.h"

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
#include <QFontInfo>
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
#include <QDropEvent>
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

class TimelineTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // ---- edits as typed operations ------------------------------------

    // Esc on the timeline asks to clear the selection (when no proposal
    // takes Esc for Reject); so does a click on an empty lane.
    // An effect card (application/x-kerf-fx) held over a clip rings it; a
    // drop adds the effect to that clip (fx-add), as the Inspector's does.
    // Over an empty lane it is refused.
    void effectCardsDropOnClips() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        TimelineWidget *timeline = window.m_timeline;
        QVERIFY(timeline->acceptDrops());
        timeline->fitToWidth(window.m_timelineScroll->viewport()->width());
        QCoreApplication::processEvents();
        const QRect clip = timeline->clipBounds(QStringLiteral("v1b"));
        QVERIFY(!clip.isEmpty());
        QMimeData mime;
        mime.setData(QStringLiteral("application/x-kerf-fx"), QByteArrayLiteral("blur.gaussian"));
        QDragEnterEvent enter(clip.center(), Qt::CopyAction, &mime, Qt::LeftButton, {});
        QCoreApplication::sendEvent(timeline, &enter);
        QVERIFY(enter.isAccepted());
        // Past the last clip, nothing takes it.
        const QPoint empty(timeline->width() - 4, clip.center().y());
        QVERIFY(timeline->clipAt(empty).id.isEmpty());
        QDragMoveEvent away(empty, Qt::CopyAction, &mime, Qt::LeftButton, {});
        QCoreApplication::sendEvent(timeline, &away);
        QVERIFY(!away.isAccepted());
        QVERIFY(timeline->m_dropClip.isEmpty());
        QDragMoveEvent over(clip.center(), Qt::CopyAction, &mime, Qt::LeftButton, {});
        QCoreApplication::sendEvent(timeline, &over);
        QVERIFY(over.isAccepted());
        QCOMPARE(timeline->m_dropClip, QStringLiteral("v1b"));
        QSignalSpy dropped(timeline, &TimelineWidget::effectDropped);
        window.m_lastCommand = {};
        QDropEvent drop(QPointF(clip.center()), Qt::CopyAction, &mime, Qt::LeftButton, {});
        QCoreApplication::sendEvent(timeline, &drop);
        QCOMPARE(dropped.size(), 1);
        QVERIFY(timeline->m_dropClip.isEmpty());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        QCOMPARE(window.m_lastCommand.value("ops").toArray(),
                 (QJsonArray{QJsonObject{{"op", "fx-add"}, {"clip", "v1b"}, {"fx", "blur.gaussian"}}}));
    }

    // The heads' target toggles choose where the Source's Insert and
    // Overwrite land (MainWindow::sourceTargets); a kind switched off is
    // not edited.
    void targetTogglesChooseWhereSourceEditsLand() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        TimelineWidget *timeline = window.m_timeline;
        QVERIFY(timeline->targetsEditable());
        window.m_sourceMonitor->open(QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"width", 1920}, {"height", 1080},
                                                 {"hasAudio", true}, {"duration", 20.0}},
                                     QString());
        QCOMPARE(window.sourceTargets(), (QStringList{"V1", "A1"}));
        timeline->setTargetTracks({QStringLiteral("V2"), QStringLiteral("A2")});
        QCOMPARE(window.sourceTargets(), (QStringList{"V2", "A2"}));
        timeline->setTargetTracks({QStringLiteral("V2")});
        QCOMPARE(window.sourceTargets(), QStringList{"V2"});
    }

    void timelineAsksToClearTheSelection() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 1.0}, {"name", "Opening"}}}}}}}}, 8, 24);
        QSignalSpy cleared(&timeline, &TimelineWidget::deselectRequested);
        key(&timeline, Qt::Key_Escape);
        QCOMPARE(cleared.size(), 0);
        timeline.setSelectedClip(QStringLiteral("clip"));
        key(&timeline, Qt::Key_Escape);
        QCOMPARE(cleared.size(), 1);
        mouse(&timeline, QEvent::MouseButtonPress, QPointF(kTimelineOrigin + 400, kRulerHeight + kTrackHeight / 2));
        mouse(&timeline, QEvent::MouseButtonRelease, QPointF(kTimelineOrigin + 400, kRulerHeight + kTrackHeight / 2));
        QCOMPARE(cleared.size(), 2);
    }

    void timelineKeepsSeekSelectMoveAndTrimCoordinates() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QSignalSpy selections(&timeline, &TimelineWidget::clipSelected);
        QSignalSpy moves(&timeline, &TimelineWidget::moveRequested);
        QSignalSpy trims(&timeline, &TimelineWidget::trimRequested);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 36, kTrackHeight / 2));
        QCOMPARE(seeks.constLast()[0].toDouble(), 0.5);
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 144, y));
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 216, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(kTimelineOrigin + 216, y));
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("clip"));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        for (const bool leading : {true, false}) {
            const int edge = kTimelineOrigin + (leading ? 75 : 357);
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge, y));
            QTest::mouseMove(&timeline, QPoint(edge + 72, y));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(edge + 72, y));
            QCOMPARE(trims.constLast()[1].toBool(), leading);
            // The edge moves by the drag distance, landing where the preview showed it.
            QCOMPARE(trims.constLast()[2].toDouble(), double((leading ? 24 : 120) + 24));
        }
        QCOMPARE(trims.count(), 2);
        timeline.setSelectedClip(QStringLiteral("clip"));
        timeline.setPlayhead(2.5);
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
    }

    void toolsChooseTheEditAClipDragMakes() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 1.0}, {"startFrame", 0}, {"durationFrames", 24}},
                QJsonObject{{"id", "b"}, {"timelineStart", 1.0}, {"duration", 1.0}, {"startFrame", 24}, {"durationFrames", 24}}}}}}}}}});
        auto op = [&window] { return window.m_lastCommand.value("ops").toArray().first().toObject(); };
        window.m_commands->action(QStringLiteral("tool.slip"))->trigger();
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Slip);
        window.moveClip(QStringLiteral("b"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("slip"));
        QCOMPARE(op().value("frames").toInt(), -12);
        window.m_commands->action(QStringLiteral("tool.slide"))->trigger();
        window.moveClip(QStringLiteral("b"), 1.5);
        QCOMPARE(op().value("op").toString(), QStringLiteral("slide"));
        QCOMPARE(op().value("frames").toInt(), 12);
        window.m_commands->action(QStringLiteral("tool.ripple"))->trigger();
        window.trimClip(QStringLiteral("b"), false, 42);
        QCOMPARE(op().value("op").toString(), QStringLiteral("trim"));
        QVERIFY(op().value("ripple").toBool());
        // Rolling b's in point moves the cut after a.
        window.m_commands->action(QStringLiteral("tool.roll"))->trigger();
        window.trimClip(QStringLiteral("b"), true, 30);
        QCOMPARE(op().value("op").toString(), QStringLiteral("roll"));
        QCOMPARE(op().value("clip").toString(), QStringLiteral("a"));
        QCOMPARE(op().value("frames").toInt(), 6);
        // Only one tool is chosen at a time.
        QVERIFY(!window.m_commands->action(QStringLiteral("tool.slip"))->isChecked());
    }

    void timelineRazorSnapAndTrackToggles() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 2.0}},
                QJsonObject{{"id", "b"}, {"timelineStart", 4.0}, {"duration", 1.0}}}}}}}}, 8, 24);
        timeline.fitToWidth(720);
        timeline.show();
        QSignalSpy razors(&timeline, &TimelineWidget::razorRequested);
        QSignalSpy moves(&timeline, &TimelineWidget::moveRequested);
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        const int y = kRulerHeight + kTrackHeight / 2;
        const auto x = [&timeline](double seconds) {
            return kTimelineOrigin + int(std::lround(seconds * (timeline.width() - kTimelineOrigin) / 8.0));
        };
        timeline.setTool(TimelineTool::Razor);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(1.0), y));
        QCOMPARE(razors.count(), 1);
        QCOMPARE(razors[0][0].toString(), QStringLiteral("a"));
        QVERIFY(std::abs(razors[0][1].toDouble() - 24.0) <= 1.0);
        // Dragging b to just past a's end snaps it onto the cut.
        timeline.setTool(TimelineTool::Selection);
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(4.5), y));
        QTest::mouseMove(&timeline, QPoint(x(4.5) - (x(4.0) - x(2.0)) + 3, y));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(x(4.5) - (x(4.0) - x(2.0)) + 3, y));
        QCOMPARE(moves.count(), 1);
        QCOMPARE(moves[0][1].toDouble(), 2.0);
        // Header toggles: picture output, then lock.
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerToggleRect(0, 0).center());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerToggleRect(0, 1).center());
        QCOMPARE(toggles.count(), 2);
        QCOMPARE(toggles[0][1].toString(), QStringLiteral("enabled"));
        QCOMPARE(toggles[0][2].toBool(), false);
        QCOMPARE(toggles[1][1].toString(), QStringLiteral("locked"));
        QCOMPARE(toggles[1][2].toBool(), true);
    }

    void proposalsShowOnTheTimelineAndAreReviewedInPlace() {
        MainWindow window{QString(), QString()};
        window.show();
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject current{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c1"}, {"timelineStart", 0.0}, {"duration", 2.0}}}}}}}};
        const QJsonObject proposed{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c1"}, {"timelineStart", 0.0}, {"duration", 1.0}}}}}}}};
        QJsonObject proposal{{"status", "pending"}, {"proposalId", "p1"}, {"script", "trim c1 out -24f\n"},
                             {"documentDiff", "-    clip c1 dur=2s\n+    clip c1 dur=1s\n"}};
        window.applySnapshot(QJsonObject{{"revision", 4}, {"duration", 2}, {"timeline", current},
                                         {"pendingProposal", proposal}, {"proposalTimeline", proposed}});
        QVERIFY(window.m_timeline->hasProposal());
        if (const QString shot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT"); !shot.isEmpty()) {
            window.resize(1500, 940);
            QCoreApplication::processEvents();
            QVERIFY(window.grab().save(shot + QStringLiteral(".proposal.png")));
        }
        QAction *accept = window.m_commands->action(QStringLiteral("proposal.accept"));
        QVERIFY(accept->isEnabled());
        accept->trigger();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("accept"));
        QCOMPARE(window.m_lastCommand.value("proposalId").toString(), QStringLiteral("p1"));
        QCOMPARE(window.m_lastCommand.value("revision").toInt(), 4);
        // Escape on the timeline rejects.
        window.m_timeline->setFocus();
        QTRY_VERIFY(window.m_timeline->hasFocus());
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("reject"));
        proposal.insert(QStringLiteral("freshness"), QStringLiteral("stale"));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 2}, {"timeline", current},
                                         {"pendingProposal", proposal}, {"proposalTimeline", proposed}});
        QVERIFY(!accept->isEnabled());
        window.applySnapshot(QJsonObject{{"revision", 6}, {"duration", 2}, {"timeline", current}});
        QVERIFY(!window.m_timeline->hasProposal());
    }

    void timelineDoubleClicksOpenOrAsk() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "a"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}}}}}, 8, 24);
        timeline.show();
        QSignalSpy opened(&timeline, &TimelineWidget::clipActivated);
        QSignalSpy asked(&timeline, &TimelineWidget::askRequested);
        const QPoint point = timeline.clipBounds(QStringLiteral("a")).center();
        QVERIFY(!timeline.clipBounds(QStringLiteral("a")).isEmpty());
        QTest::mouseDClick(&timeline, Qt::LeftButton, Qt::NoModifier, point);
        QTest::mouseDClick(&timeline, Qt::LeftButton, Qt::AltModifier, point);
        QCOMPARE(opened.count(), 1);
        QCOMPARE(asked.count(), 1);
        QCOMPARE(asked[0][0].toString(), QStringLiteral("a"));
        // End with the button up and the interval past, so later tests' clicks stand alone.
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, point);
        QTest::qWait(QApplication::doubleClickInterval() + 50);
    }

    void timelineStacksPictureTracksUpwardFromV1() {
        TimelineWidget timeline;
        timeline.resize(700, 400);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{
            QJsonObject{{"id", "base"}, {"type", "video"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "dial"}, {"type", "audio"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "top"}, {"type", "video"}, {"clips", QJsonArray{
                QJsonObject{{"id", "title"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}},
            QJsonObject{{"id", "music"}, {"type", "audio"}, {"clips", QJsonArray{}}},
        }}}, 8, 24);
        timeline.show();
        QCOMPARE(timeline.trackRowCount(), 4);
        QCOMPARE(timeline.trackRowLabel(0), QStringLiteral("V2"));
        QCOMPARE(timeline.trackRowLabel(1), QStringLiteral("V1"));
        QCOMPARE(timeline.trackRowLabel(2), QStringLiteral("A1"));
        QCOMPARE(timeline.trackRowLabel(3), QStringLiteral("A2"));
        // Playback draws the later picture track on top, so it is the upper row.
        QSignalSpy selections(&timeline, &TimelineWidget::clipSelected);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier,
                          QPoint(kTimelineOrigin + 100, timeline.trackHeaderRect(0).center().y()));
        QCOMPARE(selections.count(), 1);
        QCOMPARE(selections.constLast()[0].toString(), QStringLiteral("title"));
    }

    // ---- fixtures --------------------------------------------------------

    // Every track's height, top to bottom.
    static QList<int> trackHeights(const TimelineWidget &timeline) {
        QList<int> heights;
        for (int row = 0; row < timeline.trackRowCount(); ++row) heights << timeline.trackHeaderRect(row).height();
        return heights;
    }

    static QJsonObject trackList(const QStringList &kinds) {
        QJsonArray tracks;
        for (int index = 0; index < kinds.size(); ++index)
            tracks.append(QJsonObject{{"id", QStringLiteral("T%1").arg(index)}, {"type", kinds.at(index)}, {"clips", QJsonArray{}}});
        return QJsonObject{{"fps", 24}, {"tracks", tracks}};
    }

    static QColor token(QRgb rgba) { return tokens::color(rgba); }
    static QColor white(double alpha) {
        QColor color(255, 255, 255);
        color.setAlphaF(float(alpha));
        return color;
    }
    static bool near(const QColor &a, const QColor &b, int tolerance = 2) { return nearColor(a.rgb(), b.rgb(), tolerance); }

    // The mockup's sequence (mockup-spec 7.2 and 7.4), in the snapshot's
    // shape: tracks in layer order, V1 first among the pictures.
    static QJsonObject mockupTimeline() {
        const auto clip = [](const char *id, double start, double end, QJsonObject extra = {}) {
            extra.insert("id", id);
            extra.insert("name", id);
            extra.insert("timelineStart", start);
            extra.insert("duration", end - start);
            if (!extra.contains("kind")) extra.insert("kind", "media");
            return extra;
        };
        const auto track = [](const char *id, const char *type, const char *name, const QJsonArray &clips, QJsonObject extra = {}) {
            extra.insert("id", id);
            extra.insert("type", type);
            extra.insert("name", name);
            extra.insert("clips", clips);
            return extra;
        };
        const QJsonArray picture{clip("A012_C004", 0.0, 4.24), clip("A014_C002", 4.24, 8.48),
                                 clip("B003_C011", 8.48, 13.2, QJsonObject{{"effects", QJsonArray{QJsonObject{{"fx", "color.grade"}}}}}),
                                 clip("A019_C001", 13.2, 19.0), clip("B002_C006", 19.0, 23.4),
                                 clip("B007_C003", 23.4, 30.0, QJsonObject{{"speed", 1.2}})};
        const QJsonArray captions{clip("Caption 1", 1.0, 5.64, QJsonObject{{"kind", "caption"}, {"text", "The city is still asleep"}}),
                                  clip("Caption 4", 16.0, 19.8, QJsonObject{{"kind", "caption"}, {"text", "We open before the city does"}})};
        return QJsonObject{
            {"fps", 25},
            {"markers", QJsonArray{QJsonObject{{"id", "act2"}, {"name", "Act 2"}, {"time", 13.2}, {"duration", 0.0}}}},
            {"tracks", QJsonArray{
                track("V1", "video", "Picture", picture),
                track("V2", "video", "Multicam B", QJsonArray{clip("B011_C002", 4.8, 7.6)}),
                track("V3", "video", "Titles", QJsonArray{clip("Title_open", 0.48, 3.8, QJsonObject{{"kind", "generator"}, {"fx", "title"},
                                                                                                    {"props", QJsonObject{{"text", "Atlas Coffee"}}}})}),
                track("V4", "video", "Graphics", QJsonArray{clip("Logo_reveal", 26.48, 29.92, QJsonObject{{"kind", "generator"}})}),
                track("V5", "subtitle", "Captions", captions),
                track("A1", "audio", "Voice-over", QJsonArray{clip("VO_take3", 1.48, 11.0)}),
                track("A2", "audio", "Music", QJsonArray{clip("Atlas_cue_v2", 0.0, 30.0, QJsonObject{{"props", QJsonObject{{"gain", QJsonArray{
                    QJsonObject{{"at", 23.0}, {"frame", 575}, {"value", -6.0}, {"interp", "linear"}},
                    QJsonObject{{"at", 25.0}, {"frame", 625}, {"value", -18.0}, {"interp", "linear"}}}}}}})}),
                track("A3", "audio", "SFX", QJsonArray{clip("Whoosh_logo", 26.28, 27.08)}),
                track("A4", "audio", "Room tone", QJsonArray{clip("Room_cafe", 0.0, 30.0)}, QJsonObject{{"role", "room tone"}})}}};
    }

    // A timeline at the mockup's scale (26.8 pt/s) and track heights.
    static void mockupHeights(TimelineWidget &timeline) {
        timeline.setScale(26.8);
        const QHash<QString, int> heights{{"V5", 24}, {"V4", 24}, {"V3", 24}, {"V2", 36}, {"V1", 48}};
        for (int row = 0; row < timeline.trackRowCount(); ++row) {
            const QString id = timeline.trackRowId(row);
            timeline.setTrackHeight(row, heights.value(id, kTrackHeight));
        }
    }

    // ---- heads -----------------------------------------------------------

    void timelineHeadsCarryTargetNameAndToggles() {
        QCOMPARE(kTimelineLabelWidth, 140);
        QCOMPARE(kRulerHeight, 32);
        TimelineWidget timeline;
        timeline.setProject(mockupTimeline(), 30, 25);
        mockupHeights(timeline);
        timeline.resize(1024, 340);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        QCOMPARE(trackHeights(timeline), (QList<int>{24, 24, 24, 36, 48, 32, 32, 32, 32}));
        QStringList labels;
        for (int row = 0; row < timeline.trackRowCount(); ++row) labels << timeline.trackRowLabel(row);
        QCOMPARE(labels, (QStringList{"V5", "V4", "V3", "V2", "V1", "A1", "A2", "A3", "A4"}));
        using C = TimelineWidget::HeaderControl;
        const int v1 = 4;
        const QRect row = timeline.trackHeaderRect(v1);
        QCOMPARE(row.width(), 140);
        QCOMPARE(timeline.headerControlRect(v1, C::Target), QRect(8, row.top() + 14, 24, 20));
        // Lock and the eye against the padding on picture; lock, mute and solo on sound.
        QCOMPARE(timeline.headerControlRect(v1, C::Lock), QRect(92, row.top() + 14, 20, 20));
        QCOMPARE(timeline.headerControlRect(v1, C::Output), QRect(114, row.top() + 14, 20, 20));
        QVERIFY(timeline.headerControlRect(v1, C::Solo).isEmpty());
        const QRect a1 = timeline.trackHeaderRect(5);
        QCOMPARE(timeline.headerControlRect(5, C::Lock), QRect(70, a1.top() + 6, 20, 20));
        QCOMPARE(timeline.headerControlRect(5, C::Output), QRect(92, a1.top() + 6, 20, 20));
        QCOMPARE(timeline.headerControlRect(5, C::Solo), QRect(114, a1.top() + 6, 20, 20));
        QCOMPARE(timeline.targetTracks(), (QStringList{"V1", "A1"}));
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio); };
        // The heads are chrome, the lanes the well.
        QVERIFY(near(at(4, row.center().y()), token(tokens::chrome)));
        // A targeted track's id sits on the on fill.
        const QRect target = timeline.headerControlRect(v1, C::Target);
        QVERIFY(near(at(target.left() + 2, target.top() + 3), token(tokens::on), 3));
        const QRect untargeted = timeline.headerControlRect(3, C::Target);
        QVERIFY(near(at(untargeted.left() + 2, untargeted.top() + 3), token(tokens::chrome), 3));
        // line2 parts picture from sound; line parts the rest.
        QVERIFY(near(at(4, row.bottom()), laidOn(token(tokens::chrome), token(tokens::line2))));
        QVERIFY(near(at(4, timeline.trackHeaderRect(3).bottom()), laidOn(token(tokens::chrome), token(tokens::line))));
        QVERIFY(near(at(kTimelineOrigin + 300, row.bottom()), laidOn(token(tokens::well), token(tokens::line2))));
        // The heads' right line.
        QVERIFY(near(at(kTimelineLabelWidth - 1, row.center().y()), laidOn(token(tokens::chrome), token(tokens::line))));
        // The toggles appear on hover over that head, and only there.
        const QRect eye = timeline.headerControlRect(v1, C::Output);
        const auto inkIn = [&ratio](const QImage &shot, const QRect &rect) {
            int differ = 0;
            const QColor first = shot.pixelColor(rect.topLeft() * ratio);
            for (int y = rect.top(); y <= rect.bottom(); ++y)
                for (int x = rect.left(); x <= rect.right(); ++x)
                    if (shot.pixelColor(QPoint(x, y) * ratio) != first) ++differ;
            return differ;
        };
        QVERIFY(inkIn(image, eye) < 40); // at most a name's tail
        QTest::mouseMove(&timeline, QPoint(30, row.center().y()));
        const QImage hovered = timeline.grab().toImage();
        QVERIFY(inkIn(hovered, eye) >= 8);
        QVERIFY(hovered != image);
        // A click on one needs no second look: it is where the pointer is.
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, eye.center());
        QCOMPARE(toggles.count(), 1);
        QCOMPARE(toggles[0][0].toString(), QStringLiteral("V1"));
        QCOMPARE(toggles[0][1].toString(), QStringLiteral("enabled"));
        QCOMPARE(toggles[0][2].toBool(), false);
        // M on sound sets `muted`.
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerControlRect(5, C::Output).center());
        QCOMPARE(toggles.constLast()[1].toString(), QStringLiteral("muted"));
        QCOMPARE(toggles.constLast()[2].toBool(), true);
        // The targets show where source edits land (MainWindow::sourceTargets()
        // writes to the first unlocked track of each kind): a click changes
        // nothing, and the tip says so.
        QSignalSpy targets(&timeline, &TimelineWidget::targetsChanged);
        QVERIFY(!timeline.targetsEditable());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerControlRect(6, C::Target).center());
        QCOMPARE(timeline.targetTracks(), (QStringList{"V1", "A1"}));
        QCOMPARE(targets.count(), 0);
        const QPoint targetTip = timeline.headerControlRect(v1, C::Target).center();
        QHelpEvent help(QEvent::ToolTip, targetTip, timeline.mapToGlobal(targetTip));
        QApplication::sendEvent(&timeline, &help);
        QCOMPARE(QToolTip::text(), QStringLiteral("Targeted: source edits land here (the first unlocked picture track)"));
        QToolTip::hideText();
        // The lock's tip names no shortcut: none locks a track.
        const QPoint lockTip = timeline.headerControlRect(v1, C::Lock).center();
        QHelpEvent lockHelp(QEvent::ToolTip, lockTip, timeline.mapToGlobal(lockTip));
        QApplication::sendEvent(&timeline, &lockHelp);
        QCOMPARE(QToolTip::text(), QStringLiteral("Lock track"));
        QToolTip::hideText();
        // Once source edits follow them, the targets toggle for the session.
        timeline.setTargetsEditable(true);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerControlRect(6, C::Target).center());
        QCOMPARE(timeline.targetTracks(), (QStringList{"V1", "A1", "A2"}));
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.headerControlRect(v1, C::Target).center());
        QCOMPARE(timeline.targetTracks(), (QStringList{"A1", "A2"}));
        QCOMPARE(targets.count(), 2);
        if (const QString folder = qEnvironmentVariable("KERF_TIMELINE_GALLERY"); !folder.isEmpty())
            QVERIFY(timeline.grab().save(QDir(folder).filePath(QStringLiteral("timeline-heads.png"))));
    }

    // The default heights: V5-V3 20, V2 28, V1 40, sound 24 under a wide
    // picture, the design's nine filling its 225 lanes; vertical keeps its own.
    void wideSequencesDefaultToCompactTracks() {
        TimelineWidget timeline;
        timeline.setProject(mockupTimeline(), 30, 25);
        QCOMPARE(trackHeights(timeline), (QList<int>{36, 36, 36, 36, 36, 32, 32, 32, 32}));
        timeline.setWideSequence(true);
        const QList<int> wide{20, 20, 20, 28, 40, 24, 24, 24, 24};
        QCOMPARE(trackHeights(timeline), wide);
        QCOMPARE(std::accumulate(wide.cbegin(), wide.cend(), 0), 285 - 28 - 32 - 1);
        // By number, for any count of tracks.
        timeline.setProject(trackList({"video", "video", "audio"}), 8, 24);
        QCOMPARE(trackHeights(timeline), (QList<int>{28, 40, 24}));
        // A track's own height still wins, and the reset comes back to these.
        timeline.setTrackHeight(0, 60);
        QCOMPARE(trackHeights(timeline), (QList<int>{60, 40, 24}));
        timeline.resetTrackHeights();
        QCOMPARE(trackHeights(timeline), (QList<int>{28, 40, 24}));
        timeline.setWideSequence(false);
        QCOMPARE(trackHeights(timeline), (QList<int>{36, 36, 32}));
    }

    // S on a sound track hears it alone (monitoring only): the window sends
    // solo {tracks}, and a second click hears every track again.
    void soloSendsSoloForTheSession() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        TimelineWidget *timeline = window.m_timeline;
        timeline->resize(1024, 400);
        int a2 = -1;
        for (int row = 0; row < timeline->trackRowCount(); ++row)
            if (timeline->trackRowId(row) == QStringLiteral("A2")) a2 = row;
        QVERIFY(a2 >= 0);
        const QRect solo = timeline->headerControlRect(a2, TimelineWidget::HeaderControl::Solo);
        mouse(timeline, QEvent::MouseButtonPress, solo.center());
        mouse(timeline, QEvent::MouseButtonRelease, solo.center());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("solo"));
        QCOMPARE(window.m_lastCommand.value("tracks").toArray(), (QJsonArray{"A2"}));
        QCOMPARE(timeline->soloTracks(), QStringList{QStringLiteral("A2")});
        // A snapshot keeps it; a track that goes takes its solo with it.
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 30}, {"timeline", mockupTimeline()}});
        QCOMPARE(timeline->soloTracks(), QStringList{QStringLiteral("A2")});
        mouse(timeline, QEvent::MouseButtonPress, solo.center());
        mouse(timeline, QEvent::MouseButtonRelease, solo.center());
        QCOMPARE(window.m_lastCommand.value("tracks").toArray(), QJsonArray{});
        // A new engine (a reconnect) hears the solo again once it is ready.
        mouse(timeline, QEvent::MouseButtonPress, solo.center());
        mouse(timeline, QEvent::MouseButtonRelease, solo.center());
        QList<QJsonObject> sent;
        window.m_bus->setSender([&sent](const QJsonObject &command) {
            sent << command;
            return true;
        });
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(sent.isEmpty());
        window.setConnection(QStringLiteral("ready"));
        const auto solos = std::count_if(sent.cbegin(), sent.cend(), [](const QJsonObject &command) {
            return command.value("type").toString() == QStringLiteral("solo") && command.value("tracks").toArray() == QJsonArray{"A2"};
        });
        QCOMPARE(solos, 1);
    }

    // Sync lock lives in the track's context menu; its row sends set-track.
    void syncLockSendsSetTrack() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        TimelineWidget *timeline = window.m_timeline;
        timeline->resize(1024, 400);
        int a1 = -1;
        for (int row = 0; row < timeline->trackRowCount(); ++row)
            if (timeline->trackRowId(row) == QStringLiteral("A1")) a1 = row;
        const QPoint head(80, timeline->trackHeaderRect(a1).center().y());
        const auto chooseSync = [timeline, head] {
            QAction *sync = nullptr;
            QTimer::singleShot(0, timeline, [&sync] {
                auto *menu = qobject_cast<QMenu *>(QApplication::activePopupWidget());
                QVERIFY(menu);
                for (QAction *action : menu->actions())
                    if (action->objectName() == QLatin1String("trackSyncLock")) sync = action;
                QVERIFY(sync);
                sync->trigger();
                menu->close();
            });
            QContextMenuEvent event(QContextMenuEvent::Mouse, head, timeline->mapToGlobal(head));
            QApplication::sendEvent(timeline, &event);
        };
        chooseSync();
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set-track"));
        QCOMPARE(op.value("track").toString(), QStringLiteral("A1"));
        QCOMPARE(op.value("syncLocked").toBool(true), false);
        // A track that is not sync-locked turns it back on.
        QJsonObject project = mockupTimeline();
        QJsonArray tracks = project.value("tracks").toArray();
        for (int index = 0; index < tracks.size(); ++index) {
            QJsonObject track = tracks.at(index).toObject();
            if (track.value("id").toString() == QStringLiteral("A1")) track.insert("syncLocked", false);
            tracks.replace(index, track);
        }
        project.insert("tracks", tracks);
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 30}, {"timeline", project}});
        chooseSync();
        QCOMPARE(window.m_lastCommand.value("ops").toArray().first().toObject().value("syncLocked").toBool(false), true);
    }

    // ---- ruler -----------------------------------------------------------

    // Ticks every 10 frames, major every 50, labels every 100 at the
    // design's 25p and 26.8 pt/s; other rates take the same ladder.
    void rulerTicksFollowTheFrameLadder() {
        const RulerTicks design = rulerTicks(26.8, 25.0);
        QCOMPARE(design.minorFrames, 10);
        QCOMPARE(design.majorFrames, 50);
        QCOMPARE(design.labelFrames, 100);
        QCOMPARE(rulerTicks(480.0, 25.0).minorFrames, 1);
        QCOMPARE(rulerTicks(20.0, 60.0).minorFrames, 50);
        QCOMPARE(rulerLabel(100, 25.0, 100), QStringLiteral("00:04"));
        QCOMPARE(rulerLabel(100, 24.0, 100), QStringLiteral("00:04:04"));
        QCOMPARE(rulerLabel(25 * 3725, 25.0, 250), QStringLiteral("1:02:05"));

        TimelineWidget timeline;
        timeline.setProject(mockupTimeline(), 30, 25);
        timeline.setScale(26.8);
        timeline.setPlayhead(17.32);
        timeline.resize(1024, 340);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio); };
        const auto x = [](double seconds) { return int(std::lround(kTimelineOrigin + seconds * 26.8)); };
        // A major tick (frame 50, 2 s) is 8 tall in track2; a minor one
        // (frame 10) 4 tall in track; between them the ruler's chrome.
        QVERIFY(near(at(x(2.0), kRulerHeight - 2), token(tokens::track2)));
        QVERIFY(near(at(x(2.0), kRulerHeight - 8), token(tokens::track2)));
        QVERIFY(near(at(x(2.0), kRulerHeight - 9), token(tokens::chrome)));
        QVERIFY(near(at(x(2.4), kRulerHeight - 3), token(tokens::track)));
        QVERIFY(near(at(x(2.4), kRulerHeight - 5), token(tokens::chrome)));
        // ...but none under a label (00:00, from 0 s).
        QVERIFY(near(at(x(0.4), kRulerHeight - 3), token(tokens::chrome)));
        // A label at 4 s; the one at 16 s, within 24 of the playhead at
        // 17.32, has faded.
        const auto inked = [&](double seconds) {
            int count = 0;
            // Grey ink only: the playhead's red head may stand beside a label.
            for (int px = x(seconds) + 4; px < x(seconds) + 40; ++px)
                for (int py = 13; py < 29; ++py) {
                    const QColor pixel = at(px, py);
                    count += pixel.lightness() > 90 && std::abs(pixel.red() - pixel.blue()) < 12 ? 1 : 0;
                }
            return count;
        };
        QVERIFY(inked(4.0) > 10);
        QVERIFY2(inked(16.0) == 0, qPrintable(QString::number(inked(16.0))));
        // The playhead's head at top 13, in its red with a dark edge.
        QVERIFY(near(at(x(17.32), 17), token(tokens::playhead), 6));
        // The sequence marker's flag at 13.2 s, in ink2.
        const QRect flag = timeline.markerBounds(QStringLiteral("act2"));
        QCOMPARE(flag.top(), 1);
        QVERIFY(flag.bottom() < kRulerMarkerRow);
        QVERIFY(near(at(flag.left() + 5, flag.top() + 3), token(tokens::ink2), 6));
    }

    // The marker row: comment dots and ranges from the review (current in
    // ink, resolved faded), proposal bands over each open proposal's range
    // with "Proposal · <author>", and a click on a dot opens its thread.
    void rulerShowsCommentMarksAndProposalBands() {
        TimelineWidget timeline;
        timeline.setProject(mockupTimeline(), 30, 25);
        timeline.setScale(26.8);
        timeline.resize(1024, 340);
        QList<TimelineWidget::CommentMark> marks;
        marks.append({QStringLiteral("c-mina"), 6.0, 6.0, false, true, false, false, QStringLiteral("Mina Park")});
        marks.append({QStringLiteral("c-cursor"), 17.32, 19.8, true, false, true, false, QStringLiteral("Cursor")});
        marks.append({QStringLiteral("c-done"), 2.0, 2.0, false, true, false, true, QStringLiteral("Tom")});
        timeline.setCommentMarks(marks);
        TimelineWidget::ProposalBand band;
        band.id = QStringLiteral("proposal-3");
        band.author = QStringLiteral("Claude Code");
        band.start = 8.48;
        band.end = 13.2;
        band.tracks = {QStringLiteral("V1")};
        band.clips = {QStringLiteral("B003_C011")};
        timeline.setProposalBands({band});
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](const QPoint &point) { return image.pixelColor(point * ratio); };
        const QRect mina = timeline.commentMarkBounds(QStringLiteral("c-mina"));
        QCOMPARE(mina.size(), QSize(10, 10));
        QCOMPARE(mina.top(), 1);
        QVERIFY(near(at(mina.center()), token(tokens::ink3), 3));
        // The current one is ink, an agent's a rounded square with its range's bar.
        const QRect cursor = timeline.commentMarkBounds(QStringLiteral("c-cursor"));
        QVERIFY(cursor.width() > 50);
        QVERIFY(near(at(QPoint(cursor.left() + 5, cursor.top() + 5)), token(tokens::ink), 3));
        QVERIFY(near(at(QPoint(cursor.left() + 1, cursor.top() + 1)), token(tokens::ink), 30));
        QVERIFY(near(at(QPoint(cursor.left() + 30, 6)), token(tokens::accent), 3));
        // A resolved one at 40%.
        const QRect done = timeline.commentMarkBounds(QStringLiteral("c-done"));
        QColor faded = token(tokens::ink3);
        faded.setAlphaF(0.4f);
        QVERIFY(near(at(done.center()), laidOn(token(tokens::chrome), faded), 3));
        // The band, 12 tall over 8.48–13.2, and the veil on V1 only.
        const QRect bandRect = timeline.proposalBandBounds(QStringLiteral("proposal-3"));
        QCOMPARE(bandRect.top(), 0);
        QCOMPARE(bandRect.height(), kRulerMarkerRow);
        QCOMPARE(bandRect.left(), int(std::lround(kTimelineOrigin + 8.48 * 26.8)));
        QVERIFY(near(at(QPoint(bandRect.center().x() + 40, 6)), laidOn(token(tokens::chrome), white(0.06))));
        int v1 = -1;
        int v2 = -1;
        for (int row = 0; row < timeline.trackRowCount(); ++row) {
            if (timeline.trackRowId(row) == QStringLiteral("V1")) v1 = row;
            if (timeline.trackRowId(row) == QStringLiteral("V2")) v2 = row;
        }
        // The clip it changes is outlined in the accent.
        const QRect changed = timeline.clipBounds(QStringLiteral("B003_C011"));
        QVERIFY(near(at(QPoint(changed.left(), changed.center().y())), laidOn(token(tokens::card), token(tokens::accent)), 3));
        const int veilX = bandRect.center().x();
        QVERIFY(near(at(QPoint(veilX, timeline.trackHeaderRect(v2).center().y())), token(tokens::well), 1));
        // On V1 the veil shows in the lane's top line, over the clips' inset.
        QVERIFY(near(at(QPoint(veilX, timeline.trackHeaderRect(v1).top())), laidOn(token(tokens::well), white(0.03)), 1));
        QVERIFY(near(at(QPoint(bandRect.left() - 20, timeline.trackHeaderRect(v1).top())), token(tokens::well), 1));
        // The same bands again keep the lane cache; a changed one redraws.
        timeline.m_lanesPainted = 0;
        timeline.setProposalBands({band});
        timeline.repaint();
        QCOMPARE(timeline.m_lanesPainted, 0);
        TimelineWidget::ProposalBand moved = band;
        moved.ghosts.append({QStringLiteral("V1"), 8.0, 12.0});
        timeline.setProposalBands({moved});
        timeline.repaint();
        QVERIFY(timeline.m_lanesPainted > 0);
        timeline.setProposalBands({band});
        // Tooltips name the author, and a commenter with the time.
        QHelpEvent help(QEvent::ToolTip, QPoint(bandRect.center().x() + 40, 3), timeline.mapToGlobal(QPoint(bandRect.center().x() + 40, 3)));
        QApplication::sendEvent(&timeline, &help);
        QCOMPARE(QToolTip::text(), QStringLiteral("Proposal · Claude Code"));
        QHelpEvent dot(QEvent::ToolTip, mina.center(), timeline.mapToGlobal(mina.center()));
        QApplication::sendEvent(&timeline, &dot);
        QCOMPARE(QToolTip::text(), QStringLiteral("Mina Park  ·  00:00:06:00"));
        QToolTip::hideText();
        QSignalSpy opened(&timeline, &TimelineWidget::commentMarkActivated);
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, mina.center());
        QCOMPARE(opened.count(), 1);
        QCOMPARE(opened[0][0].toString(), QStringLiteral("c-mina"));
        QCOMPARE(seeks.count(), 0);
        if (const QString folder = qEnvironmentVariable("KERF_TIMELINE_GALLERY"); !folder.isEmpty())
            QVERIFY(image.save(QDir(folder).filePath(QStringLiteral("timeline-ruler.png"))));
    }

    // The window feeds the ruler from the engine: comments become marks,
    // the snapshot's open proposals bands with their author, and a mark
    // clicked opens its thread on the bus.
    void rulerMarksComeFromTheReview() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject proposed = mockupTimeline();
        {
            QJsonArray tracks = proposed.value("tracks").toArray();
            QJsonObject v1 = tracks.at(0).toObject();
            QJsonArray clips = v1.value("clips").toArray();
            QJsonObject graded = clips.at(2).toObject();
            graded.insert("duration", 4.0);
            clips.replace(2, graded);
            v1.insert("clips", clips);
            tracks.replace(0, v1);
            proposed.insert("tracks", tracks);
        }
        const QJsonObject proposal{{"proposalId", "p-kg"}, {"status", "pending"}, {"freshness", "fresh"}, {"baseRevision", 3},
                                   {"script", "trim B003_C011 out -18f\n"}, {"author", QJsonObject{{"kind", "mcp"}, {"name", "Claude Code"}}},
                                   {"version", 1}};
        window.applySnapshot(QJsonObject{{"revision", 3}, {"duration", 30}, {"timeline", mockupTimeline()},
                                         {"proposals", QJsonArray{proposal}}, {"proposalTimelines", QJsonObject{{"p-kg", proposed}}},
                                         {"pendingProposal", proposal}, {"proposalTimeline", proposed}});
        const QList<TimelineWidget::ProposalBand> bands = window.m_timeline->proposalBands();
        QCOMPARE(bands.size(), 1);
        QCOMPARE(bands.first().author, QStringLiteral("Claude Code"));
        QCOMPARE(bands.first().tracks, QStringList{QStringLiteral("V1")});
        QCOMPARE(bands.first().clips, QStringList{QStringLiteral("B003_C011")});
        QVERIFY(std::abs(bands.first().start - 8.48) < 1e-6 && std::abs(bands.first().end - 13.2) < 1e-6);
        const auto comment = [](const char *id, const char *kind, const char *name, double at, double duration, bool resolved) {
            return QJsonObject{{"id", id}, {"parent", QJsonValue()}, {"author", QJsonObject{{"kind", kind}, {"name", name}, {"id", name}}},
                               {"at", at}, {"duration", duration}, {"body", "Note"}, {"resolved", resolved}, {"tags", QJsonArray{}},
                               {"created", "2026-11-04T10:00:00Z"}, {"revision", 3}};
        };
        window.m_bus->dispatch(QJsonObject{{"type", "comments"}, {"reviewRevision", 1}, {"items", QJsonArray{
            comment("c1", "person", "Mina Park", 6.0, 0.0, false), comment("c2", "mcp", "Cursor", 17.32, 2.48, false),
            comment("c3", "person", "Tom", 2.0, 0.0, true)}}});
        const QList<TimelineWidget::CommentMark> marks = window.m_timeline->commentMarks();
        QCOMPARE(marks.size(), 3);
        QStringList ids;
        for (const auto &mark : marks) ids << mark.id;
        QCOMPARE(ids, (QStringList{"c3", "c1", "c2"}));
        QVERIFY(marks.at(0).resolved && marks.at(0).circle);
        QVERIFY(marks.at(2).ranged && !marks.at(2).circle);
        QSignalSpy opened(window.m_bus, &EditorBus::openThread);
        emit window.m_timeline->commentMarkActivated(QStringLiteral("c2"));
        QCOMPARE(opened.count(), 1);
        QCOMPARE(opened[0][0].toString(), QStringLiteral("c2"));
        QVERIFY(window.m_timeline->commentMarks().at(2).current);
    }

    // ---- lanes -----------------------------------------------------------

    // Flat, neutral clips (mockup-spec 7.4): card with an inset line2,
    // radius 4, one point in from the lane and a point short of the next;
    // selected in 1.5 ink; titles and captions on raised; video name bars
    // on black at 55%; sound on its peak and RMS greys.
    void timelineClipsAreFlatAndNeutral() {
        MediaArtwork artwork;
        artwork.peaks.insert(QStringLiteral("snd"), fullScalePeaks(10.0));
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        QJsonObject project = mockupTimeline();
        QJsonArray tracks = project.value("tracks").toArray();
        QJsonObject vo = tracks.at(5).toObject();
        QJsonArray clips = vo.value("clips").toArray();
        QJsonObject take = clips.at(0).toObject();
        take.insert("sourceId", "snd");
        take.insert("in", 0.0);
        take.insert("out", 9.52);
        clips.replace(0, take);
        vo.insert("clips", clips);
        tracks.replace(5, vo);
        project.insert("tracks", tracks);
        timeline.setProject(project, 30, 25);
        mockupHeights(timeline);
        timeline.setSelectedClip(QStringLiteral("A014_C002"));
        timeline.resize(1024, 340);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const auto at = [&](int x, int y) { return image.pixelColor(QPoint(x, y) * ratio); };
        const QRect a = timeline.clipBounds(QStringLiteral("A012_C004"));
        const QRect b = timeline.clipBounds(QStringLiteral("A014_C002"));
        int v1 = -1;
        for (int row = 0; row < timeline.trackRowCount(); ++row)
            if (timeline.trackRowId(row) == QStringLiteral("V1")) v1 = row;
        const QRect lane = timeline.trackHeaderRect(v1);
        QCOMPARE(a.top(), lane.top() + 1);
        QCOMPARE(a.height(), lane.height() - 2);
        QCOMPARE(a.left(), int(std::lround(kTimelineOrigin + 0.0)));
        QCOMPARE(a.right() + 2, b.left());
        // The pixel between them is the well.
        QVERIFY(near(at(a.right() + 1, a.center().y()), token(tokens::well)));
        // Card under the name bar's black at 55%, inset line2 at the edge.
        QVERIFY(near(at(a.center().x(), a.bottom() - 6), token(tokens::card)));
        QVERIFY(near(at(a.center().x() + 30, a.top() + 3), laidOn(token(tokens::card), QColor(0, 0, 0, 140)), 3));
        const QRect c = timeline.clipBounds(QStringLiteral("A019_C001"));
        QVERIFY(near(at(c.left(), c.center().y()), laidOn(token(tokens::card), token(tokens::line2))));
        // Selected: 1.5 ink inside its edge.
        QVERIFY(near(at(b.left(), b.center().y()), token(tokens::ink), 3));
        // Titles and captions sit on raised.
        const QRect title = timeline.clipBounds(QStringLiteral("Title_open"));
        QVERIFY(near(at(title.right() - 4, title.bottom() - 3), token(tokens::raised)));
        const QRect caption = timeline.clipBounds(QStringLiteral("Caption 1"));
        QVERIFY(near(at(caption.right() - 4, caption.top() + 3), token(tokens::raised)));
        // A sound clip's waveform: its grey ink mid-clip, no hue anywhere.
        const QRect sound = timeline.clipBounds(QStringLiteral("VO_take3"));
        QVERIFY(isWaveInk(at(sound.center().x(), sound.center().y())));
        QCOMPARE(amberPixels(image), 0);
        for (int px = kTimelineLabelWidth; px < image.width() / ratio; px += 7)
            for (int py = kRulerHeight; py < timeline.trackHeaderRect(timeline.trackRowCount() - 1).bottom(); py += 5) {
                const QColor pixel = at(px, py);
                if (pixel == token(tokens::playhead)) continue;
                QVERIFY2(std::abs(pixel.red() - pixel.green()) <= 12 && std::abs(pixel.green() - pixel.blue()) <= 12 ||
                             pixel.blue() > pixel.red() + 40,
                         qPrintable(QStringLiteral("%1 at %2,%3").arg(pixel.name()).arg(px).arg(py)));
            }
        if (const QString folder = qEnvironmentVariable("KERF_TIMELINE_GALLERY"); !folder.isEmpty())
            QVERIFY(image.save(QDir(folder).filePath(QStringLiteral("timeline-clips.png"))));
    }

    // A picture clip is a filmstrip at its height: 16:9 frames with a 1 pt
    // black gap, under a name bar on black at 55%.
    void timelineFilmstripsAre16By9WithGaps() {
        MediaArtwork artwork;
        QImage frame(160, 90, QImage::Format_RGB32);
        frame.fill(QColor("#7a8a9a"));
        artwork.frames.insert(QStringLiteral("pic"), frame);
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(QJsonObject{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "p"}, {"kind", "media"}, {"sourceId", "pic"}, {"timelineStart", 0.0}, {"duration", 6.0}}}}}}}}, 8, 24);
        timeline.setAllTrackHeights(kLargeTrackHeight);
        timeline.resize(900, 200);
        timeline.show();
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const QRect clip = timeline.clipBounds(QStringLiteral("p"));
        const int tile = int(std::lround(clip.height() * 16.0 / 9.0));
        const int y = clip.bottom() - 4;
        QVERIFY(near(image.pixelColor(QPoint(clip.left() + tile / 2, y) * ratio), QColor("#7a8a9a"), 3));
        QCOMPARE(image.pixelColor(QPoint(clip.left() + tile, y) * ratio).rgb(), qRgb(0, 0, 0));
        QVERIFY(near(image.pixelColor(QPoint(clip.left() + tile + 2, y) * ratio), QColor("#7a8a9a"), 3));
    }

    // The QC dot (--neg) leads the name of a clip a finished check found a
    // problem in, from the bridge's `check` findings.
    void qcDotMarksTheClipsACheckFound() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        window.m_bus->dispatch(QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false}, {"findings", QJsonArray{
            QJsonObject{{"severity", "error"}, {"code", "text.overlap"}, {"entity", "Caption 4"}, {"message", "Overlaps the lower third"}},
            QJsonObject{{"severity", "info"}, {"code", "note"}, {"entity", "A012_C004"}, {"message", "Fine"}}}}});
        QCOMPARE(window.m_timeline->qcClips(), QSet<QString>{QStringLiteral("Caption 4")});
        TimelineWidget &timeline = *window.m_timeline;
        timeline.resize(1024, 340);
        mockupHeights(timeline);
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const QRect caption = timeline.clipBounds(QStringLiteral("Caption 4"));
        // 6 in, centred: the dot's middle.
        QVERIFY(near(image.pixelColor(QPoint(caption.left() + 9, caption.center().y()) * ratio), token(tokens::neg), 30));
        const QRect clean = timeline.clipBounds(QStringLiteral("Caption 1"));
        QVERIFY(!near(image.pixelColor(QPoint(clean.left() + 9, clean.center().y()) * ratio), token(tokens::neg), 60));
        // A later check that finds nothing clears it.
        window.m_bus->dispatch(QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", true}, {"findings", QJsonArray{}}});
        QVERIFY(window.m_timeline->qcClips().isEmpty());
    }

    // The volume rubber band is the clip's gain keys; dragging a key writes
    // the keys back ({keys: [{at: frame, value, interp}]}), dragging a level
    // without keys writes the level, and Option-click adds a key.
    void rubberBandDragWritesGainKeys() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        TimelineWidget &timeline = *window.m_timeline;
        timeline.resize(1100, 400);
        timeline.setScale(26.8);
        timeline.setAllTrackHeights(kLargeTrackHeight);
        const QPolygonF band = timeline.gainBand(QStringLiteral("Atlas_cue_v2"));
        QCOMPARE(band.size(), 2);
        const QRect clip = timeline.clipBounds(QStringLiteral("Atlas_cue_v2"));
        QVERIFY(std::abs(band.at(0).x() - (clip.left() + 23.0 * 26.8)) < 0.01);
        QVERIFY(band.at(1).y() > band.at(0).y());
        QCOMPARE(band.at(0).y(), TimelineWidget::gainY(-6.0, QRectF(clip)));
        QSignalSpy gains(&timeline, &TimelineWidget::gainRequested);
        // The first key, 6 pt up.
        const QPoint key = band.at(0).toPoint();
        mouse(&timeline, QEvent::MouseButtonPress, key);
        mouse(&timeline, QEvent::MouseMove, key - QPoint(0, 6));
        mouse(&timeline, QEvent::MouseButtonRelease, key - QPoint(0, 6));
        QCOMPARE(gains.count(), 1);
        const QJsonArray keys = gains[0][1].toJsonValue().toObject().value("keys").toArray();
        QCOMPARE(keys.size(), 2);
        QCOMPARE(keys.at(0).toObject().value("at").toInt(), 575);
        QCOMPARE(keys.at(1).toObject().value("at").toInt(), 625);
        QCOMPARE(keys.at(1).toObject().value("value").toDouble(), -18.0);
        QCOMPARE(keys.at(0).toObject().value("interp").toString(), QStringLiteral("linear"));
        const double raised = keys.at(0).toObject().value("value").toDouble();
        QVERIFY2(raised > -6.0 && raised < 6.0, qPrintable(QString::number(raised)));
        // Sent as a typed set of the clip's gain.
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("Atlas_cue_v2"));
        QCOMPARE(op.value("props").toObject().value("gain").toObject().value("keys").toArray(), keys);
        // A clip without keys: its level at 0 dB, dragged, is a level.
        const QRect room = timeline.clipBounds(QStringLiteral("Room_cafe"));
        mouse(&timeline, QEvent::MouseMove, room.center());
        const QPolygonF level = timeline.gainBand(QStringLiteral("Room_cafe"));
        QCOMPARE(level.size(), 2);
        const QPoint on(room.center().x(), int(std::lround(level.at(0).y())));
        mouse(&timeline, QEvent::MouseButtonPress, on);
        mouse(&timeline, QEvent::MouseMove, on + QPoint(0, 8));
        mouse(&timeline, QEvent::MouseButtonRelease, on + QPoint(0, 8));
        QCOMPARE(gains.count(), 2);
        QVERIFY(gains[1][1].toJsonValue().isDouble());
        QVERIFY(gains[1][1].toJsonValue().toDouble() < 0.0);
        QVERIFY(window.m_lastCommand.value("ops").toArray().first().toObject().value("props").toObject().value("gain").isDouble());
        // Option-click on the band adds a key there.
        QMouseEvent press(QEvent::MouseButtonPress, QPointF(on), timeline.mapToGlobal(QPointF(on)), Qt::LeftButton, Qt::LeftButton, Qt::AltModifier);
        QApplication::sendEvent(&timeline, &press);
        QMouseEvent release(QEvent::MouseButtonRelease, QPointF(on), timeline.mapToGlobal(QPointF(on)), Qt::LeftButton, Qt::NoButton, Qt::AltModifier);
        QApplication::sendEvent(&timeline, &release);
        QCOMPARE(gains.count(), 3);
        const QJsonArray added = gains[2][1].toJsonValue().toObject().value("keys").toArray();
        QCOMPARE(added.size(), 1);
        QCOMPARE(added.at(0).toObject().value("at").toInt(), int(std::lround((room.center().x() - room.left()) / 26.8 * 25.0)));
    }

    // While a proposal plays as a preview its clips stand where it puts
    // them and dashed ghosts where they are now; without one, ghosts hide.
    void previewShowsGhostsWhereClipsWere() {
        TimelineWidget timeline;
        const QJsonObject current = mockupTimeline();
        timeline.setProject(current, 30, 25);
        timeline.setScale(26.8);
        timeline.resize(1100, 400);
        QJsonObject proposed = current;
        QJsonArray tracks = proposed.value("tracks").toArray();
        QJsonObject v4 = tracks.at(3).toObject();
        QJsonArray clips = v4.value("clips").toArray();
        QJsonObject logo = clips.at(0).toObject();
        logo.insert("timelineStart", 25.8);
        clips.replace(0, logo);
        v4.insert("clips", clips);
        tracks.replace(3, v4);
        proposed.insert("tracks", tracks);
        TimelineWidget::ProposalBand band;
        band.id = QStringLiteral("p-cc");
        band.author = QStringLiteral("Codex");
        band.start = 25.8;
        band.end = 29.92;
        band.tracks = {QStringLiteral("V4")};
        band.clips = {QStringLiteral("Logo_reveal")};
        band.ghosts.append({QStringLiteral("V4"), 26.48, 29.92});
        timeline.setProposalBands({band});
        timeline.show();
        int v4row = -1;
        for (int row = 0; row < timeline.trackRowCount(); ++row)
            if (timeline.trackRowId(row) == QStringLiteral("V4")) v4row = row;
        const QRect lane = timeline.trackHeaderRect(v4row);
        const auto x = [](double seconds) { return int(std::lround(kTimelineOrigin + seconds * 26.8)); };
        const auto at = [&](const QImage &image, int px, int py) { return image.pixelColor(QPoint(px, py) * image.devicePixelRatio()); };
        // Not previewing: the clip where it is, at 26.48; nothing at 25.9.
        QImage image = timeline.grab().toImage();
        QVERIFY(near(at(image, x(28.0), lane.bottom() - 3), token(tokens::raised)));
        QVERIFY(near(at(image, x(25.9), lane.bottom() - 3), laidOn(token(tokens::well), white(0.03))));
        timeline.setPreview(QStringLiteral("p-cc"), proposed);
        image = timeline.grab().toImage();
        // The clip from 25.8, and the ghost's dashed edge where it was.
        QVERIFY(near(at(image, x(25.9), lane.bottom() - 3), token(tokens::raised)));
        // Its top edge alternates 3 on, 3 off: lit dashes over the ghost's fill.
        const int fill = at(image, x(29.5), lane.center().y()).lightness();
        int dashes = 0;
        int gaps = 0;
        for (int px = x(29.5); px < x(29.92) - 6; ++px) (at(image, px, lane.top() + 1).lightness() > fill + 8 ? dashes : gaps) += 1;
        QVERIFY(dashes > 0);
        QVERIFY(gaps > 0);
        // Past the moved clip's end (29.12), the ghost: its fill over the veil.
        QVERIFY(near(at(image, x(29.5), lane.center().y()), laidOn(laidOn(token(tokens::well), white(0.03)), white(0.03)), 2));
        timeline.setPreview({}, {});
        image = timeline.grab().toImage();
        QVERIFY(near(at(image, x(25.9), lane.bottom() - 3), laidOn(token(tokens::well), white(0.03))));
    }

    // Sequence markers are flags on the ruler's marker row; a marker with a
    // duration reaches over it.
    void timelineDrawsSequenceMarkersOnTheRuler() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        const QJsonArray markers{QJsonObject{{"id", "beat1"}, {"time", 2.0}, {"duration", 0.0}},
                                 QJsonObject{{"id", "chorus"}, {"time", 4.0}, {"duration", 1.0}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}, {"markers", markers}}, 8, 24);
        timeline.setPlayhead(6.0);
        timeline.show();
        const QRect beat = timeline.markerBounds(QStringLiteral("beat1"));
        const QRect chorus = timeline.markerBounds(QStringLiteral("chorus"));
        QVERIFY(!beat.isEmpty());
        QVERIFY(timeline.markerBounds(QStringLiteral("nowhere")).isEmpty());
        QVERIFY(beat.bottom() < kRulerMarkerRow);
        QVERIFY(chorus.width() > beat.width() + 10);
        QVERIFY(beat.right() < chorus.left());
        const QImage image = timeline.grab().toImage();
        QVERIFY(near(image.pixelColor(QPoint(beat.left() + 5, beat.top() + 2) * image.devicePixelRatio()), token(tokens::ink2), 6));
        // Its notch: below 70% at the middle, the ruler.
        QVERIFY(near(image.pixelColor(QPoint(beat.left() + 5, beat.top() + 9) * image.devicePixelRatio()), token(tokens::chrome), 6));
    }

    // The corner: the playhead's timecode in mono 500 13/20 ink and the rate.
    void cornerShowsTheTimecodeAndRate() {
        TimelineWidget timeline;
        timeline.setProject(mockupTimeline(), 30, 25);
        timeline.setPlayhead(17.32);
        timeline.resize(800, 240);
        TextRecorder recorder;
        timeline.render(&recorder);
        QCOMPARE(recorder.colorOf(QStringLiteral("00:00:17:08")), token(tokens::ink));
        QCOMPARE(recorder.colorOf(QStringLiteral("25p")), token(tokens::ink3));
        QCOMPARE(QFontInfo(tokens::font(tokens::monoLarge)).pixelSize(), 13);
    }

    // ---- bar -------------------------------------------------------------

    void timelineStripRunsTheTransportToolsAndSnap() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        TimelineToolStrip *tools = window.m_timelineWiring->tools();
        QVERIFY(tools);
        QTRY_VERIFY(tools->isVisible());
        // At the top of the panel, across it, 28 tall.
        QCOMPARE(tools->parentWidget(), window.m_timelinePanel);
        QCOMPARE(tools->geometry(), QRect(0, 0, window.m_timelinePanel->width(), tokens::layout::strip));
        // The strip holds no words, and neither Add marker, Add track nor Ask.
        QVERIFY(tools->findChildren<QLabel *>().isEmpty());
        for (const char *gone : {"timelineAddMarker", "timelineAddTrack", "timelineAsk", "timelineHeights", "timelineDetail"})
            QVERIFY2(!window.findChild<QWidget *>(QString::fromLatin1(gone)), gone);
        // Their commands stay.
        for (const char *id : {"marker.add", "timeline.addTrack", "ai.ask", "view.fitTracks", "view.resetTrackHeights"})
            QVERIFY2(window.m_commands->action(QString::fromLatin1(id)), id);
        // Transport first, in the mockup's order, then the tools, Snap, then far
        // right the zoom.
        using Transport = TimelineToolStrip::Transport;
        int x = -1;
        for (const Transport which : {Transport::Start, Transport::Previous, Transport::Play, Transport::Next, Transport::End}) {
            auto *button = tools->transportButton(which);
            QVERIFY(button->x() > x);
            QCOMPARE(button->size(), QSize(24, 24));
            x = button->x();
        }
        QVERIFY(tools->tools()->x() > x);
        QVERIFY(tools->snapButton()->x() > tools->tools()->x());
        QVERIFY(tools->zoomOutButton()->x() > tools->snapButton()->x());
        QCOMPARE(tools->zoomInButton()->geometry().right() + 1, tools->width() - 12);
        // The transport runs the registry's playback commands.
        window.m_duration = 30.0;
        window.m_bus->setDuration(30.0);
        tools->transportButton(Transport::End)->click();
        QCOMPARE(window.m_playhead, 30.0);
        tools->transportButton(Transport::Start)->click();
        QCOMPARE(window.m_playhead, 0.0);
        tools->transportButton(Transport::Next)->click();
        QVERIFY(std::abs(window.m_playhead - 1.0 / 25.0) < 1e-6);
        tools->transportButton(Transport::Previous)->click();
        QCOMPARE(window.m_playhead, 0.0);
        window.m_lastCommand = {};
        tools->transportButton(Transport::Play)->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("play"));
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", true}});
        QVERIFY(tools->playing());
        QCOMPARE(ui::tipName(tools->transportButton(Transport::Play)), QStringLiteral("Pause"));
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", false}});
        QVERIFY(!tools->playing());
        // Tools: Selection and Razor; the others stay on keys and in the menu.
        ui::SegmentedControl *segments = tools->tools();
        QCOMPARE(segments->count(), 2);
        QCOMPARE(segments->currentIndex(), 0);
        QTest::mouseClick(segments->segment(1), Qt::LeftButton);
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Razor);
        QVERIFY(window.m_commands->action(QStringLiteral("tool.razor"))->isChecked());
        // One of the others: no segment is on, the tool is.
        window.m_commands->action(QStringLiteral("tool.slip"))->trigger();
        QCOMPARE(window.m_timeline->tool(), TimelineTool::Slip);
        QCOMPARE(segments->currentIndex(), -1);
        window.m_commands->action(QStringLiteral("tool.selection"))->trigger();
        QCOMPARE(segments->currentIndex(), 0);
        // Snap is sequence.snap.
        QVERIFY(tools->snapButton()->isChecked());
        QTest::mouseClick(tools->snapButton(), Qt::LeftButton);
        QVERIFY(!window.m_timeline->snapping());
        QVERIFY(!window.m_commands->action(QStringLiteral("sequence.snap"))->isChecked());
        window.m_commands->action(QStringLiteral("sequence.snap"))->trigger();
        QVERIFY(tools->snapButton()->isChecked());
        // Add marker's command adds one at the playhead.
        window.m_playhead = 2.0;
        window.triggerCommand(QStringLiteral("marker.add"));
        const QJsonObject marker = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(marker.value("op").toString(), QStringLiteral("add-marker"));
        QCOMPARE(marker.value("at").toInt(), 50);
        if (const QString folder = qEnvironmentVariable("KERF_TIMELINE_GALLERY"); !folder.isEmpty())
            QVERIFY(window.grab().save(QDir(folder).filePath(QStringLiteral("timeline-window.png"))));
    }

    // The ruler's scale fits the sequence to the lanes by default, and follows
    // the lanes' width as the window resizes.
    void theRulerFitsTheSequenceToTheLanes() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        QCoreApplication::processEvents();
        TimelineWidget *timeline = window.m_timeline;
        QTRY_VERIFY(timeline->isFitToWidth());
        const auto lanes = [&window] { return window.m_timelineScroll->viewport()->width() - kTimelineLabelWidth; };
        // 8 at each end, as the mockup's (lane - 16) / seconds.
        QVERIFY2(std::abs(timeline->scale() - (lanes() - 16.0) / 30.0) < 4.0, qPrintable(QString::number(timeline->scale())));
        const double wide = timeline->scale();
        window.resize(1200, 1000);
        QCoreApplication::processEvents();
        QTRY_VERIFY(timeline->scale() < wide);
        window.close();
    }

    void timelineStripFollowsTheRegistry() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("disconnected"));
        TimelineToolStrip *tools = window.m_timelineWiring->tools();
        QVERIFY(tools);
        CommandRegistry *commands = window.m_commands;
        using Transport = TimelineToolStrip::Transport;
        QVERIFY(!commands->action(QStringLiteral("playback.toggle"))->isEnabled());
        QVERIFY(!tools->transportButton(Transport::Play)->isEnabled());
        QVERIFY(tools->snapButton()->isEnabled());
        const QString offline = commands->reason(QStringLiteral("playback.goToEnd"));
        QVERIFY(!offline.isEmpty());
        QCOMPARE(ui::tipName(tools->transportButton(Transport::End)), QStringLiteral("Go to end · %1").arg(offline));
        // Ready, they follow the actions back.
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30}, {"timeline", mockupTimeline()}});
        QVERIFY(tools->transportButton(Transport::End)->isEnabled());
        QCOMPARE(ui::tipName(tools->transportButton(Transport::End)), QStringLiteral("Go to end"));
        // The tips carry the keymap's keys: Kerf's, then Final Cut Pro's.
        const auto keys = [commands](const char *id) {
            return commands->action(QString::fromLatin1(id))->shortcut().toString(QKeySequence::NativeText);
        };
        ui::SegmentedControl *segments = tools->tools();
        QCOMPARE(ui::tipName(segments->segment(0)), QStringLiteral("Selection"));
        QCOMPARE(ui::tipShortcut(segments->segment(0)), keys("tool.selection"));
        QCOMPARE(ui::tipShortcut(tools->transportButton(Transport::Play)), keys("playback.toggle"));
        commands->setKeymap(QStringLiteral("fcp"));
        QCOMPARE(ui::tipShortcut(segments->segment(0)), QKeySequence(Qt::Key_A).toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(segments->segment(1)), QKeySequence(Qt::Key_B).toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(tools->snapButton()), QKeySequence(Qt::Key_N).toString(QKeySequence::NativeText));
        commands->setKeymap(QStringLiteral("kerf"));
        QCOMPARE(ui::tipShortcut(tools->snapButton()), keys("sequence.snap"));
    }

    // The zoom's slider runs over the scale on a log scale, its keys step
    // it, and it follows a fit to the width.
    void timelineZoomIsASliderBetweenKeys() {
        TimelineWidget timeline;
        timeline.setProject(trackList({"video"}), 20, 24);
        TimelineToolStrip zoom(&timeline);
        QCOMPARE(zoom.slider()->width(), 64);
        QCOMPARE(zoom.zoomOutButton()->size(), QSize(24, 24));
        QCOMPARE(zoom.height(), tokens::layout::strip);
        QCOMPARE(TimelineToolStrip::position(TimelineWidget::kMinScale), 0);
        QCOMPARE(TimelineToolStrip::position(TimelineWidget::kMaxScale), TimelineToolStrip::kSteps);
        QVERIFY(std::abs(TimelineToolStrip::scaleAt(TimelineToolStrip::position(72.0)) - 72.0) < 3.0);
        const double before = timeline.scale();
        QTest::mouseClick(zoom.zoomInButton(), Qt::LeftButton);
        QVERIFY(timeline.scale() > before);
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(timeline.scale() < before);
        timeline.fitToWidth(900);
        QCOMPARE(int(std::lround(zoom.slider()->value())), TimelineToolStrip::position(timeline.scale()));
        // Command-= and Command-minus step it while the timeline has focus,
        // and the keys' tips name them as this Mac writes them.
        QCOMPARE(ui::tipShortcut(zoom.zoomInButton()), QKeySequence(Qt::CTRL | Qt::Key_Equal).toString(QKeySequence::NativeText));
        QCOMPARE(ui::tipShortcut(zoom.zoomOutButton()), QKeySequence(Qt::CTRL | Qt::Key_Minus).toString(QKeySequence::NativeText));
        timeline.resize(900, 200);
        timeline.show();
        timeline.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&timeline));
        timeline.setFocus();
        QTRY_VERIFY(timeline.hasFocus());
        const double fitted = timeline.scale();
        QTest::keyClick(&timeline, Qt::Key_Equal, Qt::ControlModifier);
        QVERIFY(timeline.scale() > fitted);
        QCOMPARE(int(std::lround(zoom.slider()->value())), TimelineToolStrip::position(timeline.scale()));
        QTest::keyClick(&timeline, Qt::Key_Minus, Qt::ControlModifier);
        QTest::keyClick(&timeline, Qt::Key_Minus, Qt::ControlModifier);
        QVERIFY(timeline.scale() < fitted);
    }

    // Fitted, a new length moves the scale: the slider follows it, and Zoom
    // out starts from what is on screen.
    void timelineZoomFollowsAFitTheLengthMoved() {
        TimelineWidget timeline;
        timeline.setProject(trackList({"video"}), 10, 24);
        TimelineToolStrip zoom(&timeline);
        timeline.fitToWidth(900);
        const auto place = [&] { return int(std::lround(zoom.slider()->value())); };
        QCOMPARE(place(), TimelineToolStrip::position(timeline.scale(), timeline.minScale()));
        const double before = timeline.scale();
        timeline.setProject(trackList({"video"}), 30, 24);
        QVERIFY(timeline.isFitToWidth());
        QVERIFY(timeline.scale() < before);
        QCOMPARE(place(), TimelineToolStrip::position(timeline.scale(), timeline.minScale()));
        const double fitted = timeline.scale();
        QVERIFY(fitted > TimelineWidget::kMinScale);
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(timeline.scale() < fitted);
        // The first import into an empty project: a second, then a longer cut.
        TimelineWidget empty;
        empty.setProject(trackList({"video"}), 1, 24);
        TimelineToolStrip strip(&empty);
        empty.fitToWidth(900);
        QCOMPARE(int(std::lround(strip.slider()->value())), TimelineToolStrip::kSteps);
        empty.setProject(trackList({"video"}), 8, 24);
        const double wide = empty.scale();
        QVERIFY(wide < TimelineWidget::kMaxScale);
        QCOMPARE(int(std::lround(strip.slider()->value())), TimelineToolStrip::position(wide, empty.minScale()));
        QTest::mouseClick(strip.zoomOutButton(), Qt::LeftButton);
        QVERIFY(empty.scale() < wide);
    }

    // A long sequence's fit is below kMinScale: the slider reaches down to it,
    // and comes back to the fit after zooming.
    void timelineZoomComesBackToTheWholeOfALongSequence() {
        TimelineWidget timeline;
        timeline.setProject(trackList({"video"}), 600, 24);
        TimelineToolStrip zoom(&timeline);
        timeline.fitToWidth(1600);
        const double fitted = timeline.scale();
        QVERIFY(fitted < TimelineWidget::kMinScale);
        QCOMPARE(timeline.minScale(), fitted);
        QCOMPARE(int(std::lround(zoom.slider()->value())), 0);
        const int whole = timeline.minimumWidth();
        // By the keys.
        QTest::mouseClick(zoom.zoomInButton(), Qt::LeftButton);
        QVERIFY(!timeline.isFitToWidth());
        QVERIFY(timeline.scale() > fitted && timeline.scale() < TimelineWidget::kMinScale);
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(timeline.isFitToWidth());
        QCOMPARE(timeline.scale(), fitted);
        QCOMPARE(timeline.minimumWidth(), whole);
        // By the slider.
        zoom.slider()->setValue(40);
        QVERIFY(!timeline.isFitToWidth());
        QVERIFY(timeline.scale() > TimelineWidget::kMinScale);
        zoom.slider()->setValue(0);
        QVERIFY(timeline.isFitToWidth());
        QCOMPARE(timeline.scale(), fitted);
        // Zoom out at the fit goes no further.
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(timeline.isFitToWidth());
        // Fitted again at the width the viewport took while zoomed.
        zoom.slider()->setValue(40);
        timeline.followViewportWidth(1400);
        zoom.slider()->setValue(0);
        QVERIFY(timeline.isFitToWidth());
        QVERIFY(timeline.scale() < fitted);
        QCOMPARE(timeline.minimumWidth(), 1400);
    }

    // A short sequence's fit is inside the range: stepping stops at it on the
    // way back, and past it the view zooms on out.
    void timelineZoomStopsAtTheFitOfAShortSequence() {
        TimelineWidget timeline;
        timeline.setProject(trackList({"video"}), 8, 24);
        TimelineToolStrip zoom(&timeline);
        timeline.fitToWidth(900);
        const double fitted = timeline.scale();
        const int place = int(std::lround(zoom.slider()->value()));
        QVERIFY(place > 10 && place < TimelineToolStrip::kSteps - 20);
        zoom.slider()->setValue(place + 13);
        QVERIFY(!timeline.isFitToWidth());
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(!timeline.isFitToWidth());
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(timeline.isFitToWidth());
        QCOMPARE(timeline.scale(), fitted);
        QTest::mouseClick(zoom.zoomOutButton(), Qt::LeftButton);
        QVERIFY(!timeline.isFitToWidth());
        QVERIFY(timeline.scale() < fitted);
    }

    // Fit shares the panel under the ruler and leaves the add-track row in
    // view: nothing is left to scroll.
    void timelineFitLeavesNothingToScroll() {
        TimelineScrollArea scroll;
        scroll.resize(900, 400);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        timeline->setProject(trackList({"video", "video", "audio", "audio"}), 8, 24);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        timeline->fitTracksToView();
        QTest::qWait(20);
        QVERIFY(timeline->trackHeaderRect(0).height() > kVideoTrackHeight);
        QCOMPARE(scroll.horizontalScrollBar()->maximum(), 0);
        QCOMPARE(scroll.verticalScrollBar()->maximum(), 0);
    }

    void timelineSetsEveryTrackHeight() {
        QCOMPARE(kVideoTrackHeight, 36);
        QCOMPARE(kTrackHeight, 32);
        QCOMPARE(kSmallTrackHeight, 24);
        QCOMPARE(kLargeTrackHeight, 48);
        TimelineWidget timeline;
        timeline.resize(900, 520);
        timeline.setProject(trackList({"video", "video", "audio", "audio"}), 8, 24);
        timeline.show();
        QCOMPARE(trackHeights(timeline), (QList<int>{36, 36, 32, 32}));
        timeline.setAllTrackHeights(kSmallTrackHeight);
        QCOMPARE(trackHeights(timeline), QList<int>(4, 24));
        timeline.setAllTrackHeights(kLargeTrackHeight);
        QCOMPARE(trackHeights(timeline), QList<int>(4, 48));
        // One track dragged apart.
        const int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(50, edge));
        QTest::mouseMove(&timeline, QPoint(50, edge + 10));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(50, edge + 10));
        QCOMPARE(trackHeights(timeline), (QList<int>{58, 48, 48, 48}));
        timeline.resetTrackHeights();
        QCOMPARE(trackHeights(timeline), (QList<int>{36, 36, 32, 32}));
        // Fit fills the lanes under the ruler, leaving the add-track row.
        const int each = (timeline.height() - kRulerHeight - kAddLaneHeight) / 4;
        timeline.fitTracksToView();
        QCOMPARE(trackHeights(timeline), QList<int>(4, each));
    }

    void timelineTrackHeightsFollowTheHeaderEdge() {
        TimelineWidget timeline;
        timeline.resize(700, 400);
        const QJsonObject project{{"tracks", QJsonArray{QJsonObject{{"id", "v"}, {"type", "video"}, {"clips", QJsonArray{}}},
                                                        QJsonObject{{"id", "a"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}};
        timeline.setProject(project, 8, 24);
        timeline.show();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
        int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, edge));
        QTest::mouseMove(&timeline, QPoint(100, edge + 30));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, edge + 30));
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).top(), kRulerHeight + kVideoTrackHeight + 30);
        edge = timeline.trackHeaderRect(1).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, edge));
        QTest::mouseMove(&timeline, QPoint(100, 0));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, 0));
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        // Heights belong to the track, so a new snapshot keeps them.
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        timeline.fitTracksToView();
        QCOMPARE(timeline.trackHeaderRect(0).height(), timeline.trackHeaderRect(1).height());
        QVERIFY(timeline.trackHeaderRect(0).height() > kVideoTrackHeight);
        QVERIFY(timeline.trackHeaderRect(1).bottom() < timeline.height());
        timeline.resetTrackHeights();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
    }

    // Option-Up and Option-Down, and Option-scroll over the headers, step
    // each track through the heights from its own, keeping in place the
    // track at the top of the view or under the pointer; plain keys and the
    // wheel elsewhere leave heights alone.
    void timelineStepsEveryTrackWithOptionKeysAndScroll() {
        TimelineScrollArea scroll;
        scroll.resize(900, 400);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        QStringList kinds;
        for (int index = 0; index < 40; ++index) kinds << (index % 4 == 3 ? "audio" : "video");
        timeline->setProject(trackList(kinds), 8, 24);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Rows run V30 … V1, then A1 … A10.
        const auto heights = [](int picture, int sound) {
            QList<int> expected(30, picture);
            expected.append(QList<int>(10, sound));
            return expected;
        };
        QCOMPARE(trackHeights(*timeline), heights(36, 32));
        timeline->setFocus();
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(28, 24));
        QTest::keyClick(timeline, Qt::Key_Down);
        QCOMPARE(trackHeights(*timeline), heights(28, 24));

        const auto wheel = [timeline](const QPoint &at, int delta, Qt::KeyboardModifiers modifiers,
                                      Qt::ScrollPhase phase = Qt::NoScrollPhase) {
            QWheelEvent event(at, timeline->mapToGlobal(at), QPoint(), QPoint(0, delta), Qt::NoButton, modifiers, phase, false);
            QApplication::sendEvent(timeline, &event);
            return event.isAccepted();
        };
        const auto inView = [timeline, &scroll](const QPoint &point) { return timeline->mapFrom(scroll.viewport(), point); };
        QVERIFY(wheel(inView(QPoint(100, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        // A trackpad's small deltas add up to a notch.
        for (int part = 0; part < 3; ++part) wheel(inView(QPoint(100, 200)), -40, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(28, 24));
        // Without Option, or over the lanes, the wheel is the scroll area's.
        QVERIFY(!wheel(inView(QPoint(100, 200)), 120, Qt::NoModifier));
        QVERIFY(!wheel(inView(QPoint(500, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(28, 24));
        // The momentum after a flick steps nothing, and a new gesture starts
        // its notch afresh.
        QVERIFY(wheel(inView(QPoint(100, 200)), 80, Qt::AltModifier, Qt::ScrollBegin));
        QVERIFY(wheel(inView(QPoint(100, 200)), 80, Qt::AltModifier, Qt::ScrollUpdate));
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        for (int event = 0; event < 6; ++event) QVERIFY(wheel(inView(QPoint(100, 200)), 120, Qt::AltModifier, Qt::ScrollMomentum));
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        wheel(inView(QPoint(100, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        wheel(inView(QPoint(100, 200)), 0, Qt::AltModifier, Qt::ScrollEnd);
        wheel(inView(QPoint(100, 200)), 0, Qt::AltModifier, Qt::ScrollBegin);
        wheel(inView(QPoint(100, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        wheel(inView(QPoint(100, 200)), -40, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(28, 24));

        // The track under the pointer stays where it is in the view.
        scroll.verticalScrollBar()->setValue(300);
        QCoreApplication::processEvents();
        const QPoint over = inView(QPoint(100, 250));
        const int row = timeline->trackRowAt(over.y());
        QVERIFY(row > 5);
        const auto top = [timeline](int at) { return timeline->trackHeaderRect(at).top() + timeline->pos().y(); };
        const int before = top(row);
        wheel(over, -120, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(24, 20));
        QCOMPARE(top(row), before);
        wheel(QPoint(over.x(), timeline->trackHeaderRect(row).top() + 2), 240, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(32, 28));
        QCOMPARE(top(row), before);
        // The keys keep the track at the top of the view in place.
        const int first = timeline->trackRowAt(-timeline->pos().y() + kRulerHeight);
        const int firstTop = top(first);
        QTest::keyClick(timeline, Qt::Key_Up, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(36, 32));
        QCOMPARE(top(first), firstTop);
    }

    // A step taller never makes a track shorter: picture tracks dragged tall
    // and one hero layer taller still each step from their own heights, the
    // sound tracks from theirs, and a track added later from the height
    // every track was stepped to.
    void timelineStepsKeepTracksSizedApart() {
        TimelineWidget timeline;
        timeline.resize(900, 900);
        QJsonObject project = trackList({"video", "video", "video", "audio", "audio"});
        timeline.setProject(project, 8, 24);
        timeline.show();
        const int pictureEdge = timeline.trackHeaderRect(2).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(100, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(100, pictureEdge + 100 - kVideoTrackHeight));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(100, pictureEdge + 100 - kVideoTrackHeight));
        QCOMPARE(trackHeights(timeline), (QList<int>{100, 100, 100, 32, 32}));
        const int heroEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, heroEdge));
        QTest::mouseMove(&timeline, QPoint(100, heroEdge + 20));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, heroEdge + 20));
        QCOMPARE(trackHeights(timeline), (QList<int>{120, 100, 100, 32, 32}));
        timeline.stepAllTrackHeights(1);
        QCOMPARE(trackHeights(timeline), (QList<int>{128, 104, 104, 36, 36}));
        timeline.stepAllTrackHeights(-1);
        QCOMPARE(trackHeights(timeline), (QList<int>{104, 86, 86, 32, 32}));
        QWheelEvent wheel(QPointF(100, timeline.trackHeaderRect(3).center().y()), timeline.mapToGlobal(QPointF(100, 0)), QPoint(),
                          QPoint(0, 120), Qt::NoButton, Qt::AltModifier, Qt::NoScrollPhase, false);
        QApplication::sendEvent(&timeline, &wheel);
        QCOMPARE(trackHeights(timeline), (QList<int>{128, 104, 104, 36, 36}));
        QJsonArray tracks = project.value("tracks").toArray();
        tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
        project.insert("tracks", tracks);
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.allTrackHeight(), 42);
        QCOMPARE(trackHeights(timeline).constLast(), 42);
        timeline.setAllTrackHeights(kSmallTrackHeight);
        QCOMPARE(trackHeights(timeline), QList<int>(6, kSmallTrackHeight));
    }

    // At every height down to S the line between two headers shows the
    // resize cursor and says what dragging it does; dragging it resizes the
    // track above, Option-dragging every track of that kind; and a head's
    // toggles keep their clicks, however close to the line.
    void timelineHeaderEdgesResizeAtEveryHeight() {
        TimelineWidget timeline;
        timeline.resize(800, 400);
        timeline.setProject(trackList({"video", "video", "video", "audio", "audio"}), 8, 24);
        timeline.setAllTrackHeights(kSmallTrackHeight);
        timeline.show();
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        const int edge = timeline.trackHeaderRect(1).bottom() + 1;
        for (const int dy : {-2, 0, 2}) {
            QTest::mouseMove(&timeline, QPoint(60, edge + dy));
            QCOMPARE(timeline.cursor().shape(), Qt::SplitVCursor);
        }
        QHelpEvent help(QEvent::ToolTip, QPoint(60, edge), timeline.mapToGlobal(QPoint(60, edge)));
        QApplication::sendEvent(&timeline, &help);
        QVERIFY2(QToolTip::text().contains(QStringLiteral("Option-drag")), qPrintable(QToolTip::text()));
        QToolTip::hideText();

        const QRect lock = timeline.headerToggleRect(2, 1);
        QVERIFY(lock.top() > edge && lock.top() - edge <= 3);
        QTest::mouseMove(&timeline, lock.center());
        QVERIFY(timeline.cursor().shape() != Qt::SplitVCursor);
        for (const QPoint &at : {lock.center(), QPoint(lock.center().x(), lock.top())}) {
            QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, at);
            QCOMPARE(toggles.constLast().at(1).toString(), QStringLiteral("locked"));
        }
        QCOMPARE(toggles.count(), 2);
        QCOMPARE(trackHeights(timeline), QList<int>(5, kSmallTrackHeight));

        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(60, edge - 1));
        QTest::mouseMove(&timeline, QPoint(60, edge + 19));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(60, edge + 19));
        QCOMPARE(trackHeights(timeline), (QList<int>{24, 44, 24, 24, 24}));
        const int soundEdge = timeline.trackHeaderRect(3).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(60, soundEdge));
        QTest::mouseMove(&timeline, QPoint(60, soundEdge + 12));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(60, soundEdge + 12));
        QCOMPARE(trackHeights(timeline), (QList<int>{24, 44, 24, 36, 36}));
        const int pictureEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(60, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(60, pictureEdge + 6));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(60, pictureEdge + 6));
        QCOMPARE(trackHeights(timeline), (QList<int>{30, 30, 30, 36, 36}));
        QCOMPARE(toggles.count(), 2);
    }

    // Track heights are remembered per project in the editor's settings
    // (never in project.kerf, which collaborators share), restored when the
    // project opens again; a track added later takes the height every
    // track was last set to.
    void timelineRemembersTrackHeightsPerProject() {
        QSettings().remove(QStringLiteral("timelineTrackHeights"));
        const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("timelineTrackHeights")); });
        QTemporaryDir first;
        QTemporaryDir second;
        const QJsonObject project = trackList({"video", "audio"});
        {
            TimelineWidget timeline;
            timeline.resize(700, 400);
            timeline.setHeightsKey(first.path());
            timeline.setProject(project, 8, 24);
            timeline.show();
            timeline.setAllTrackHeights(26);
            const int edge = timeline.trackHeaderRect(0).bottom() + 1;
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, edge));
            QTest::mouseMove(&timeline, QPoint(100, edge + 24));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(100, edge + 24));
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
        }
        {
            TimelineWidget timeline;
            timeline.setHeightsKey(first.path());
            timeline.setProject(project, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
            QCOMPARE(timeline.allTrackHeight(), 26);
            QJsonObject grown = project;
            QJsonArray tracks = grown.value("tracks").toArray();
            tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
            grown.insert("tracks", tracks);
            timeline.setProject(grown, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26, 26}));
        }
        {
            TimelineWidget timeline;
            timeline.setProject(project, 8, 24);
            timeline.setHeightsKey(first.path());
            QCOMPARE(trackHeights(timeline), (QList<int>{50, 26}));
        }
        {
            TimelineWidget timeline;
            timeline.setHeightsKey(second.path());
            timeline.setProject(project, 8, 24);
            QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        }
        QSettings settings;
        settings.beginGroup(QStringLiteral("timelineTrackHeights"));
        QStringList paths;
        for (const QString &group : settings.childGroups()) paths << settings.value(group + QStringLiteral("/path")).toString();
        settings.endGroup();
        const QString remembered = QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath());
        QCOMPARE(paths, QStringList{remembered});
        MainWindow window{first.path(), QString()};
        QCOMPARE(window.m_timeline->m_heightsPath, remembered);
        window.m_timeline->setProject(project, 8, 24);
        QCOMPARE(trackHeights(*window.m_timeline), (QList<int>{50, 26}));
        window.m_timeline->resetTrackHeights();
        TimelineWidget reopened;
        reopened.setHeightsKey(first.path());
        reopened.setProject(project, 8, 24);
        QCOMPARE(trackHeights(reopened), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        // Left at S, it reopens at S with S chosen.
        reopened.setAllTrackHeights(kSmallTrackHeight);
        {
            TimelineWidget atSmall;
            atSmall.setHeightsKey(first.path());
            atSmall.setProject(project, 8, 24);
            QCOMPARE(trackHeights(atSmall), QList<int>(2, kSmallTrackHeight));
        }
        {
            MainWindow atSmall{first.path(), QString()};
            atSmall.setConnection(QStringLiteral("ready"));
            atSmall.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", project}});
            QCOMPARE(trackHeights(*atSmall.m_timeline), QList<int>(2, kSmallTrackHeight));
        }
        // A snapshot that removes the one track sized apart leaves every
        // track at one height, and its preset chosen.
        reopened.setAllTrackHeights(kLargeTrackHeight);
        reopened.setTrackHeight(1, 40);
        QJsonObject shrunk = project;
        QJsonArray tracks = shrunk.value("tracks").toArray();
        tracks.removeLast();
        shrunk.insert("tracks", tracks);
        reopened.setProject(shrunk, 8, 24);
    }

    void timelineRevealsAddLaneFromTheHeader() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}}, 8, 24);
        timeline.show();
        QSignalSpy adds(&timeline, &TimelineWidget::addLaneRequested);
        QSignalSpy seeks(&timeline, &TimelineWidget::seekRequested);
        QVERIFY(!timeline.addLaneVisible());
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QTest::mouseClick(&timeline, Qt::LeftButton, Qt::NoModifier, timeline.addLaneButtonCenter());
        QCOMPARE(adds.count(), 1);
        QVERIFY(!timeline.addLaneVisible());
        QCOMPARE(seeks.count(), 0);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "audio"}, {"clips", QJsonArray{}}}}}}, 8, 24);
        QTest::mouseMove(&timeline, timeline.addLaneButtonCenter());
        QVERIFY(timeline.addLaneVisible());
        QCOMPARE(timeline.addLaneButtonCenter().y(), kRulerHeight + kTrackHeight + (kAddLaneHeight - 24) / 2 + (24 - 1) / 2);
    }

    void timelinePinsHeadersAndRulerWhileScrolling() {
        QScrollArea scroll;
        scroll.resize(640, 260);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        QJsonArray tracks;
        for (int i = 0; i < 12; ++i) tracks.append(QJsonObject{{"id", QString::number(i)}, {"type", "video"}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 120, 24);
        scroll.show();
        QTest::qWait(20);
        scroll.horizontalScrollBar()->setValue(2000);
        scroll.verticalScrollBar()->setValue(150);
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->value(), 2000);
        QCOMPARE(scroll.verticalScrollBar()->value(), 150);
        const QImage view = scroll.viewport()->grab().toImage();
        const qreal ratio = view.devicePixelRatio();
        QVERIFY(near(view.pixelColor(QPoint(4, 4) * ratio), token(tokens::chrome)));
        QVERIFY(near(view.pixelColor(QPoint(4, 60) * ratio), token(tokens::chrome)));
        QVERIFY(near(view.pixelColor(QPoint(kTimelineLabelWidth + 50, kRulerHeight - 1) * ratio), laidOn(token(tokens::chrome), token(tokens::line))) ||
                near(view.pixelColor(QPoint(kTimelineLabelWidth + 51, kRulerHeight - 1) * ratio), laidOn(token(tokens::chrome), token(tokens::line))));

        QSignalSpy seeks(timeline, &TimelineWidget::seekRequested);
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier, timeline->mapFrom(scroll.viewport(), QPoint(80, 60)));
        QCOMPARE(seeks.count(), 0);
        QTest::mouseMove(timeline, timeline->mapFrom(scroll.viewport(), QPoint(80, 60)));
        QVERIFY(timeline->skimLabel().isEmpty());
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier, timeline->mapFrom(scroll.viewport(), QPoint(kTimelineOrigin + 72, 20)));
        QVERIFY(!seeks.isEmpty());
        QVERIFY(std::abs(seeks.constLast()[0].toDouble() - 2072.0 / 72.0) < 0.05);
    }

    // A dense edit: 48 tracks at S, one line each, with the playhead through every row.
    void denseTimelineDrawsEveryHeadAtS() {
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(denseTimeline(48, 24.0), 24.0, 24.0);
        timeline.setSelectedClip(QStringLiteral("c20_2"));
        timeline.setPlayhead(7.5);
        timeline.setAllTrackHeights(kSmallTrackHeight);
        timeline.resize(1440, kRulerHeight + 48 * kSmallTrackHeight + kAddLaneHeight);
        timeline.show();
        QCOMPARE(trackHeights(timeline), QList<int>(48, kSmallTrackHeight));
        const int playX = kTimelineOrigin + int(std::floor(7.5 * 72.0));
        const QImage image = timeline.grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const int lastPicture = 31;
        for (int row = 0; row < 48; ++row) {
            const QRect header = timeline.trackHeaderRect(row);
            QVERIFY(header.bottom() < image.height() / ratio);
            QVERIFY(near(image.pixelColor(QPoint(4, header.center().y()) * ratio), token(tokens::chrome)));
            QVERIFY(near(image.pixelColor(QPoint(4, header.bottom()) * ratio),
                         laidOn(token(tokens::chrome), token(row == lastPicture ? tokens::line2 : tokens::line))));
            QCOMPARE(image.pixelColor(QPoint(playX, header.center().y()) * ratio), token(tokens::playhead));
        }
        QCOMPARE(amberPixels(image), 0);
    }

    // ---- performance contracts -----------------------------------------

    // A full repaint of a 300-track timeline paints the lanes in view, not
    // all of them; and the pointer crossing from clip to clip repaints the
    // clips it leaves and enters, not the view.
    void timelinePaintsOnlyTheTracksInView() {
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineScrollArea scroll;
        scroll.resize(900, 500);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        timeline->setProject(denseTimeline(300, 10.0), 10.0, 24.0);
        timeline->setAllTrackHeights(kMinTrackHeight);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Small enough that the whole timeline could be cached at once
        // (renderLanes' limit), so painting only the rows in view is the
        // timeline's choice, not the cache's.
        QVERIFY(double(timeline->width()) * timeline->height() * timeline->devicePixelRatioF() * timeline->devicePixelRatioF() < 12000000.0);
        scroll.verticalScrollBar()->setValue(2000);
        QCoreApplication::processEvents();
        timeline->invalidateLanes();
        timeline->m_lanesPainted = 0;
        scroll.viewport()->repaint();
        const int inView = scroll.viewport()->height() / kMinTrackHeight + 2;
        QVERIFY2(timeline->m_lanesPainted > 0 && timeline->m_lanesPainted <= inView,
                 qPrintable(QStringLiteral("%1 lanes painted for %2 in view").arg(timeline->m_lanesPainted).arg(inView)));
        // Across three clips of one track: each change of the clip (or the
        // clip's edge) under the pointer repaints the one it leaves and the
        // one it enters, a lane's worth each, never the lanes in view.
        const int y = timeline->mapFrom(scroll.viewport(), QPoint(0, 200)).y();
        QVERIFY(timeline->trackRowAt(y) >= 0);
        QStringList crossed;
        int changes = 0;
        timeline->m_lanesPainted = 0;
        for (int x = kTimelineOrigin + 10; x < kTimelineOrigin + 3 * 216; x += 9) {
            const auto before = std::make_pair(timeline->m_hoverClipId, timeline->m_hoverEdge);
            QTest::mouseMove(timeline, QPoint(x, y));
            QCoreApplication::processEvents();
            changes += before != std::make_pair(timeline->m_hoverClipId, timeline->m_hoverEdge) ? 1 : 0;
            if (!timeline->m_hoverClipId.isEmpty() && !crossed.contains(timeline->m_hoverClipId)) crossed << timeline->m_hoverClipId;
        }
        QVERIFY2(crossed.size() >= 3, qPrintable(crossed.join(QLatin1Char(' '))));
        QVERIFY2(timeline->m_lanesPainted <= 2 * changes && 2 * changes < changes * inView,
                 qPrintable(QStringLiteral("%1 lanes for %2 changes").arg(timeline->m_lanesPainted).arg(changes)));
    }

    // Scrolling moves what is on screen and paints only what scrolls in
    // and the pinned ruler and headers, and what is on screen afterwards is
    // exactly what a full repaint draws: the headers stay in step with
    // their lanes, and the ruler and corner stay pinned.
    void timelineScrollsByMovingWhatIsOnScreen() {
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineScrollArea scroll;
        scroll.resize(900, 500);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        timeline->setProject(denseTimeline(60, 30.0), 30.0, 24.0);
        timeline->setPlayhead(9.5);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        QCoreApplication::processEvents();
        struct PaintMeter final : QObject {
            qint64 pixels = 0;
            bool eventFilter(QObject *, QEvent *event) override {
                if (event->type() == QEvent::Paint)
                    for (const QRect &rect : static_cast<QPaintEvent *>(event)->region()) pixels += qint64(rect.width()) * rect.height();
                return false;
            }
        } meter;
        timeline->installEventFilter(&meter);
        QScrollBar *vertical = scroll.verticalScrollBar();
        QScrollBar *horizontal = scroll.horizontalScrollBar();
        QVERIFY(vertical->maximum() > 400 && horizontal->maximum() > 400);
        const QSize view = scroll.viewport()->size();
        const auto onScreen = [&scroll] { return scroll.screen()->grabWindow(scroll.winId()).toImage(); };
        struct Step {
            int dx;
            int dy;
        };
        for (const Step step : {Step{0, 40}, Step{0, 7}, Step{0, -23}, Step{0, 200}, Step{60, 0}, Step{-15, 0}, Step{3, 0},
                                Step{30, 30}, Step{-41, -9}}) {
            meter.pixels = 0;
            horizontal->setValue(horizontal->value() + step.dx);
            vertical->setValue(vertical->value() + step.dy);
            QCoreApplication::processEvents();
            const qint64 painted = meter.pixels;
            if (step.dx == 0)
                QVERIFY2(painted <= qint64(std::abs(step.dy) + 2 * kRulerHeight + 4) * view.width(),
                         qPrintable(QStringLiteral("%1 px for %2 down").arg(painted).arg(step.dy)));
            if (step.dy == 0)
                QVERIFY2(painted <= qint64(std::abs(step.dx) + 2 * kTimelineLabelWidth + 4) * view.height(),
                         qPrintable(QStringLiteral("%1 px for %2 across").arg(painted).arg(step.dx)));
            const QImage scrolled = onScreen();
            // A paint still pending at the grab shows as a change here.
            QCoreApplication::processEvents();
            const bool settled = scrolled == onScreen();
            scroll.viewport()->repaint();
            const QImage repainted = onScreen();
            // Where they differ, to tell a stale strip from a stray pixel. An
            // antialiased edge laid over a moved strip may round one step
            // apart from the same edge painted whole (a filmstrip under the
            // flat theme's translucent name bar shows it), so a pixel counts
            // only when a channel is more than one step off.
            QRect differ;
            int count = 0;
            if (scrolled.size() == repainted.size())
                for (int y = 0; y < scrolled.height(); ++y)
                    for (int x = 0; x < scrolled.width(); ++x)
                        if (!nearColor(scrolled.pixel(x, y), repainted.pixel(x, y), 1)) {
                            differ |= QRect(x, y, 1, 1);
                            ++count;
                        }
            QVERIFY2(scrolled.size() == repainted.size() && count == 0,
                     qPrintable(QStringLiteral("after %1, %2: %3 px differ in %4,%5 %6x%7 (%8 px painted; viewport at %9,%10 %11x%12; %13)")
                                    .arg(step.dx)
                                    .arg(step.dy)
                                    .arg(count)
                                    .arg(differ.x())
                                    .arg(differ.y())
                                    .arg(differ.width())
                                    .arg(differ.height())
                                    .arg(painted)
                                    .arg(scroll.viewport()->x())
                                    .arg(scroll.viewport()->y())
                                    .arg(view.width())
                                    .arg(view.height())
                                    .arg(settled ? QStringLiteral("settled") : QStringLiteral("changed on a second pass"))));
        }
        timeline->removeEventFilter(&meter);
    }

    // Rows are found by a binary search: correct at every edge, and a
    // hundred times the tracks costs nowhere near a hundred times the time.
    void timelineFindsRowsWithoutWalkingThemAll() {
        const auto timed = [](int count) {
            QStringList kinds;
            for (int index = 0; index < count; ++index) kinds << (index % 3 == 2 ? "audio" : "video");
            TimelineWidget timeline;
            timeline.setProject(trackList(kinds), 8, 24);
            for (int row = 0; row < count; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                if (timeline.trackRowAt(header.top()) != row || timeline.trackRowAt(header.bottom()) != row) return -1.0;
            }
            if (timeline.trackRowAt(kRulerHeight - 1) != -1 || timeline.trackRowAt(timeline.trackHeaderRect(count - 1).bottom() + 1) != -1)
                return -1.0;
            const int span = timeline.trackHeaderRect(count - 1).bottom();
            double best = 1e9;
            qint64 sum = 0;
            for (int run = 0; run < 3; ++run) {
                QElapsedTimer timer;
                timer.start();
                for (int lookup = 0; lookup < 200000; ++lookup) sum += timeline.trackRowAt(kRulerHeight + int((lookup * 7919LL) % span));
                best = std::min(best, timer.nsecsElapsed() / 1e6);
            }
            return sum > 0 ? best : -1.0;
        };
        const double few = timed(30);
        const double many = timed(3000);
        QVERIFY(few >= 0.0 && many >= 0.0);
        // A walk would be some 90 times slower; a search is a few times.
        QVERIFY2(many < 20.0 * std::max(few, 0.2), qPrintable(QStringLiteral("%1 ms for 30 tracks, %2 ms for 3000").arg(few).arg(many)));
    }

    void timelineScrubsFromThePointer() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "clip"}, {"timelineStart", 1.0}, {"duration", 4.0}, {"name", "Opening"}}
        }}}}}}, 8, 24);
        timeline.show();
        QSignalSpy hovers(&timeline, &TimelineWidget::hoverSeekRequested);
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 144, kTrackHeight / 2));
        QVERIFY(!hovers.isEmpty());
        QCOMPARE(hovers.constLast()[0].toDouble(), 2.0);
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:02.00"));
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 36, kTrackHeight / 2));
        QCOMPARE(timeline.skimLabel(), QStringLiteral("00:00.50"));
        QCOMPARE(hovers.constLast()[0].toDouble(), 0.5);
        QTest::mouseMove(&timeline, QPoint(20, kTrackHeight / 2));
        QVERIFY(timeline.skimLabel().isEmpty());
    }

    void laneCacheBlitKeepsLogicalPixelsAtRetinaScale() {
        constexpr qreal dpr = 2;
        constexpr int width = 480;
        constexpr int height = 90;
        QPixmap cache(width * dpr, height * dpr);
        cache.setDevicePixelRatio(dpr);
        cache.fill(QColor("#141414"));
        {
            QPainter painter(&cache);
            painter.fillRect(QRect(0, 0, width, kRulerHeight), QColor("#2a2a2a"));
            painter.fillRect(QRect(120, 40, 40, 16), QColor("#3d7eff"));
        }

        const auto at = [](const QImage &image, int x, int y) {
            return image.pixelColor(QPoint(x * int(dpr), y * int(dpr)));
        };
        QImage destination(width * dpr, height * dpr, QImage::Format_ARGB32_Premultiplied);
        destination.setDevicePixelRatio(dpr);
        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(0, 0, width, height)));
        }
        QCOMPARE(at(destination, width - 8, 6).red(), 0x2a);
        QCOMPARE(at(destination, 140, 48).blue(), 0xff);

        destination.fill(Qt::red);
        {
            QPainter painter(&destination);
            blitLaneCache(painter, cache, QRegion(QRect(300, 0, 100, height)));
        }
        QCOMPARE(at(destination, 340, 6).red(), 0x2a);
        QCOMPARE(at(destination, 20, 6).red(), 255);
    }

    void timelinePointerTimeAdvancesEveryFewPixels() {
        TimelineWidget timeline;
        timeline.resize(900, 280);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{}}}}}}, 9, 24);
        timeline.fitToWidth(900);
        timeline.show();
        QTest::qWait(20);
        const auto seconds = [](const QString &label) {
            const QStringList clock = label.split(QLatin1Char(':'));
            const QStringList sec = clock.value(1).split(QLatin1Char('.'));
            return clock.value(0).toDouble() * 60.0 + sec.value(0).toDouble() + sec.value(1).toDouble() / 100.0;
        };
        const int y = kRulerHeight + kTrackHeight / 2;
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 160, y));
        const double start = seconds(timeline.skimLabel());
        QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + 168, y));
        const double next = seconds(timeline.skimLabel());
        QVERIFY(std::abs((next - start) - 0.1) < 0.02);
        const QString screenshot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT");
        if (!screenshot.isEmpty()) {
            timeline.setProject(QJsonObject{{"tracks", QJsonArray{
                QJsonObject{{"type", "video"}, {"name", "Video 1"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "opening"}, {"name", "Interview"}, {"timelineStart", 0.4}, {"duration", 3.2}},
                    QJsonObject{{"id", "broll"}, {"name", "B-roll"}, {"timelineStart", 3.8}, {"duration", 2.6}},
                }}},
                QJsonObject{{"type", "audio"}, {"name", "Dialogue"}, {"clips", QJsonArray{
                    QJsonObject{{"id", "dial"}, {"name", "Dialogue"}, {"timelineStart", 0.2}, {"duration", 7.2}},
                }}},
            }}}, 8, 24);
            timeline.fitToWidth(980);
            timeline.resize(980, 220);
            timeline.setSelectedClip(QStringLiteral("opening"));
            timeline.setPlayhead(3.4);
            timeline.showSkimmer(5.75);
            QTest::qWait(20);
            QVERIFY(timeline.grab().save(screenshot + QStringLiteral(".timeline.png")));
        }
    }

    void timelineFitsAndAllTracksRemainReachable() {
        QScrollArea scroll;
        scroll.resize(640, 230);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        scroll.show();
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->maximum(), 0);
        QJsonArray tracks;
        for (int i = 0; i < 12; ++i) tracks.append(QJsonObject{{"id", QString::number(i)}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 3600, 24);
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        // The vertical scrollbar changes the available width.
        timeline->fitToWidth(scroll.viewport()->width());
        QTest::qWait(20);
        // An hour stays at least 2 pt per second, so it still scrolls: the
        // pointer can address more than a handful of pixels a minute.
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
        QVERIFY(scroll.verticalScrollBar()->maximum() > 0);
        QVERIFY(timeline->minimumHeight() >= kRulerHeight + 12 * kTrackHeight);
        timeline->setZoom(1.0);
        QTest::qWait(20);
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
    }

    // What a 300-track edit costs: applying a snapshot, painting the panel,
    // scrolling it, skimming across clips, and playing. Prints timings; run
    // with KERF_TIMELINE_BENCH=1 (or the number of tracks).
    void timelineBenchmark() {
        if (qEnvironmentVariableIsEmpty("KERF_TIMELINE_BENCH")) QSKIP("Set KERF_TIMELINE_BENCH=1 to time the timeline.");
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        const int count = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH") > 1 ? qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH") : 300;
        const QJsonObject project = denseTimeline(count, 60.0);
        TimelineScrollArea scroll;
        scroll.resize(1440, 800);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        timeline->setArtwork(&artwork);
        scroll.setWidget(timeline);
        scroll.show();
        QVERIFY(QTest::qWaitForWindowExposed(&scroll));
        // Painted area per paint event, to tell a full repaint from a strip.
        struct PaintMeter final : QObject {
            qint64 pixels = 0;
            int events = 0;
            bool eventFilter(QObject *, QEvent *event) override {
                if (event->type() == QEvent::Paint) {
                    ++events;
                    for (const QRect &rect : static_cast<QPaintEvent *>(event)->region()) pixels += qint64(rect.width()) * rect.height();
                }
                return false;
            }
        } meter;
        timeline->installEventFilter(&meter);
        const auto median = [](QList<double> samples) {
            std::sort(samples.begin(), samples.end());
            return samples.at(samples.size() / 2);
        };
        const auto time = [](const std::function<void()> &work) {
            QElapsedTimer timer;
            timer.start();
            work();
            return timer.nsecsElapsed() / 1e6;
        };
        QList<double> samples;
        for (int run = 0; run < 7; ++run)
            samples << time([&] {
                timeline->setProject(project, 60.0, 24.0);
                QCoreApplication::sendPostedEvents();
                QCoreApplication::processEvents();
            });
        const double apply = median(samples);
        const auto report = [&](const char *what, double ms, qint64 pixels, int events) {
            qInfo("%-28s %8.3f ms  %9lld px painted in %d paint events", what, ms, pixels, events);
        };
        report(qPrintable(QStringLiteral("apply snapshot (%1 tracks)").arg(count)), apply, 0, 0);
        // KERF_TIMELINE_BENCH_HEIGHT puts every track at one height.
        if (const int height = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH_HEIGHT"); height > 0) {
            timeline->setAllTrackHeights(height);
            QCoreApplication::processEvents();
        }
        samples.clear();
        for (int run = 0; run < 21; ++run)
            samples << time([&] {
                timeline->stepAllTrackHeights(run % 2 ? -1 : 1);
                QCoreApplication::processEvents();
            });
        report("step every track's height", median(samples), 0, 0);
        if (const int height = qEnvironmentVariableIntValue("KERF_TIMELINE_BENCH_HEIGHT"); height > 0) timeline->setAllTrackHeights(height);
        else timeline->resetTrackHeights();
        QCoreApplication::processEvents();
        samples.clear();
        for (int run = 0; run < 21; ++run) samples << time([&] { scroll.viewport()->repaint(); });
        report("repaint the panel", median(samples), 0, 0);

        QScrollBar *vertical = scroll.verticalScrollBar();
        QVERIFY(vertical->maximum() > 0);
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 1; step <= 60; ++step)
            samples << time([&] {
                vertical->setValue(step * 40);
                QCoreApplication::processEvents();
            });
        report("scroll down 40px", median(samples), meter.pixels / 60, meter.events);

        QScrollBar *horizontal = scroll.horizontalScrollBar();
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 1; step <= 60; ++step)
            samples << time([&] {
                horizontal->setValue(step * 40);
                QCoreApplication::processEvents();
            });
        report("scroll right 40px", median(samples), meter.pixels / 60, meter.events);
        horizontal->setValue(0);
        vertical->setValue(vertical->maximum() / 2);
        QCoreApplication::processEvents();

        // Skimming across clips: each step crosses into the next clip or gap.
        const QPoint origin = timeline->mapFrom(scroll.viewport(), QPoint(0, 0));
        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int step = 0; step < 60; ++step)
            samples << time([&] {
                QTest::mouseMove(timeline, origin + QPoint(kTimelineOrigin + 20 + step * 37, 300 + (step % 7) * 11));
                QCoreApplication::processEvents();
            });
        report("skim across clips", median(samples), meter.pixels / 60, meter.events);

        QTest::mouseMove(timeline, origin + QPoint(40, 300));
        QCoreApplication::processEvents();
        samples.clear();
        for (int step = 0; step < 60; ++step)
            samples << time([&] {
                QTest::mouseMove(timeline, origin + QPoint(40 + step % 3, 120 + step * 9));
                QCoreApplication::processEvents();
            });
        report("pointer over the headers", median(samples), 0, 0);

        samples.clear();
        meter.pixels = 0;
        meter.events = 0;
        for (int frame = 0; frame < 60; ++frame)
            samples << time([&] {
                timeline->setPlayhead(frame / 24.0);
                QCoreApplication::processEvents();
            });
        report("play a frame", median(samples), meter.pixels / 60, meter.events);
        timeline->removeEventFilter(&meter);
    }

private:
    // A big edit: `count` tracks, two thirds picture and a third sound, each
    // with a clip every three seconds over `seconds`, cut from the fake
    // artwork's sources so filmstrips and waveforms draw.
    static QJsonObject denseTimeline(int count, double seconds) {
        const char *pictures[] = {"wide", "close", "drone"};
        const char *sounds[] = {"wide", "close", "score"};
        QJsonArray sources{
            QJsonObject{{"id", "wide"}, {"path", "media/harbor-wide.mov"}, {"duration", 20}, {"hasAudio", true}},
            QJsonObject{{"id", "close"}, {"path", "media/fisher-close.mov"}, {"duration", 20}, {"hasAudio", true}},
            QJsonObject{{"id", "drone"}, {"path", "media/drone-pass.mov"}, {"duration", 20}},
            QJsonObject{{"id", "score"}, {"path", "media/harbor-score.wav"}, {"duration", 40}, {"hasAudio", true}}};
        QJsonArray tracks;
        for (int index = 0; index < count; ++index) {
            const bool audio = index % 3 == 2;
            QJsonArray clips;
            for (int n = 0; n * 3.0 < seconds; ++n) {
                // Gaps and lengths vary by track, so the cut reads as an edit.
                const double start = n * 3.0 + (index % 4) * 0.25;
                const double length = 2.0 + ((index + n) % 3) * 0.4;
                QJsonObject clip{{"id", QStringLiteral("c%1_%2").arg(index).arg(n)}, {"kind", "media"},
                                 {"sourceId", audio ? sounds[(index + n) % 3] : pictures[(index + n) % 3]},
                                 {"name", QStringLiteral("Shot %1-%2").arg(index + 1).arg(n + 1)},
                                 {"timelineStart", start}, {"duration", std::min(length, seconds - start)}, {"in", 1.0},
                                 {"out", 1.0 + length}};
                if ((index + n) % 5 == 0)
                    clip.insert("effects", QJsonArray{QJsonObject{{"fx", "color.grade"}, {"version", 1}, {"enabled", true}}});
                clips.append(clip);
            }
            tracks.append(QJsonObject{{"id", QStringLiteral("T%1").arg(index)}, {"type", audio ? "audio" : "video"},
                                      {"name", audio ? QStringLiteral("Sound %1").arg(index / 3 + 1)
                                                     : QStringLiteral("Layer %1").arg(index - index / 3 + 1)},
                                      {"clips", clips}});
        }
        return QJsonObject{{"fps", 24}, {"sources", sources}, {"tracks", tracks}};
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::TimelineTest)

#include "timeline_test.moc"
