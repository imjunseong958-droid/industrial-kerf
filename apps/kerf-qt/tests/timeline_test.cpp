// Tests for the timeline: headers, lanes, clips, tools and proposals.

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

class TimelineTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // Esc on the timeline asks to clear the selection (when no proposal
    // takes Esc for Reject); so does a click on an empty lane.
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
        // Reviewed in the conversation while it is open, on the timeline's
        // bar while it is closed.
        QVERIFY(!window.m_aiBar->isVisible());
        window.setChatOpen(false);
        QVERIFY(window.m_aiBar->isVisible());
        if (const QString shot = qEnvironmentVariable("KERF_QT_UI_SCREENSHOT"); !shot.isEmpty()) {
            window.resize(1500, 940);
            QCoreApplication::processEvents();
            QVERIFY(window.grab().save(shot + QStringLiteral(".proposal.png")));
        }
        QVERIFY(window.m_aiBarText->text().startsWith(QStringLiteral("Proposal")));
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
        QVERIFY(!window.m_aiBar->isVisible());
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

    void timelineDrawsSequenceMarkersOnTheRuler() {
        TimelineWidget timeline;
        timeline.resize(700, 240);
        const QJsonArray markers{
            QJsonObject{{"id", "beat1"}, {"time", 2.0}, {"duration", 0.0}, {"color", "red"}},
            QJsonObject{{"id", "chorus"}, {"time", 4.0}, {"duration", 1.0}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{}}, {"markers", markers}}, 8, 24);
        // A marker on the playhead is drawn over its tab.
        timeline.setPlayhead(2.0);
        timeline.show();
        const QRect beat = timeline.markerBounds(QStringLiteral("beat1"));
        const QRect chorus = timeline.markerBounds(QStringLiteral("chorus"));
        QVERIFY(!beat.isEmpty());
        QVERIFY(timeline.markerBounds(QStringLiteral("nowhere")).isEmpty());
        QVERIFY(beat.bottom() < kRulerHeight);
        // A marker with a duration spans it; one without is a tab.
        QVERIFY(chorus.width() > beat.width() + 10);
        QVERIFY(beat.right() < chorus.left());
        const QImage image = timeline.grab().toImage();
        QCOMPARE(image.pixelColor(beat.center().x(), beat.top() + 2), editorTheme().marker(QStringLiteral("red")));
        QCOMPARE(image.pixelColor(chorus.left() + 4, chorus.top() + 2), editorTheme().marker(QString()));
    }

    // The timeline paints from the theme, in light and in dark, and repaints
    // its cached lanes when the appearance changes under it.
    void timelineFollowsTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        setEditorColorScheme(ColorScheme::Light);
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        const QJsonObject snapshot = showcaseSnapshot(1, QStringLiteral("clean"));
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(snapshot.value("timeline").toObject(), 18, 24);
        timeline.setProposal(snapshot.value("proposalTimeline").toObject());
        timeline.setSelectedClip(QStringLiteral("v1b"));
        timeline.setPlayhead(7.5);
        timeline.resize(timeline.minimumWidth(), 360);
        timeline.show();
        const auto x = [](double seconds) { return kTimelineOrigin + int(std::lround(seconds * 72.0)); };
        // Rows, top down: V3 captions, V2 titles, V1, A1 dialogue, A2 music.
        const int captions = kRulerHeight + kVideoTrackHeight / 2;
        const int titles = captions + kVideoTrackHeight;
        const int music = kRulerHeight + 3 * kVideoTrackHeight + kTrackHeight + kTrackHeight / 2;
        const QRect selected = timeline.clipBounds(QStringLiteral("v1b"));
        const QRect unselected = timeline.clipBounds(QStringLiteral("v1a"));
        // Dragging v1b by this much lands its end near the caption's end at 13 s.
        const QPoint grip = selected.center();
        const QPoint drop = grip + QPoint(x(13.0) - x(11.0) - 5, 0);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            // Switched under the open timeline, with nothing else to invalidate its lanes.
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("timeline-%1.png").arg(theme.dark ? "dark" : "light"))));
            QCOMPARE(image.pixelColor(4, 4), theme.rulerCorner);
            // (Clear of the grid line at 9 s.)
            QCOMPARE(image.pixelColor(x(9.0) + 4, titles), theme.trackLane);
            QCOMPARE(image.pixelColor(x(7.5), titles), theme.playhead);
            // A pixel of lane after each clip, so clips that meet read as two.
            QCOMPARE(image.pixelColor(selected.right(), selected.center().y()), theme.trackLane);
            QCOMPARE(image.pixelColor(selected.right() - 1, selected.center().y()), theme.clipSelectedRingOuter);
            QCOMPARE(image.pixelColor(selected.right() - 2, selected.center().y()), theme.clipSelectedRing);
            QCOMPARE(image.pixelColor(unselected.left(), unselected.center().y()), theme.clipVideo.border);
            // The clip the proposal removes is outlined in red to its corners.
            const QRect removed = timeline.clipBounds(QStringLiteral("v1c"));
            for (const QPoint &corner : {removed.topLeft(), removed.topRight(), removed.bottomLeft(), removed.bottomRight()})
                QCOMPARE(image.pixelColor(corner), theme.proposalRemove);
            // A header's stripe is its clips' colour.
            QCOMPARE(image.pixelColor(1, captions), theme.clipCaption.header);
            QCOMPARE(image.pixelColor(1, titles), theme.clipGenerator.header);
            QCOMPARE(image.pixelColor(1, music), theme.clipMusic.header);
            QCOMPARE(amberPixels(image), 0);

            // A drag shows the edit point it snaps to, until it drops.
            // At rest, 13 s is the lane with its faint grid line.
            const auto over = [](const QColor &top, const QColor &under) {
                const double a = top.alphaF();
                return QColor::fromRgbF(top.redF() * a + under.redF() * (1 - a), top.greenF() * a + under.greenF() * (1 - a),
                                        top.blueF() * a + under.blueF() * (1 - a));
            };
            const QColor gridded = over(theme.laneGrid, theme.trackLane);
            const auto near = [](const QColor &a, const QColor &b) {
                return std::abs(a.red() - b.red()) <= 1 && std::abs(a.green() - b.green()) <= 1 && std::abs(a.blue() - b.blue()) <= 1;
            };
            QVERIFY(near(image.pixelColor(x(13.0), captions), gridded));
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, grip);
            QTest::mouseMove(&timeline, drop);
            const QImage dragging = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(dragging.save(QDir(shots).filePath(QStringLiteral("timeline-drag-%1.png").arg(theme.dark ? "dark" : "light"))));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, drop);
            QVERIFY(std::abs(qGray(dragging.pixel(x(13.0), captions)) - qGray(theme.trackLane.rgb())) > 100);
            QCOMPARE(amberPixels(dragging), 0);
            // Where it will overwrite v1c (11 s to its new end at 13 s), a
            // dashed edge inside its selection ring, which stays whole:
            // white dashes on a dark line, in either appearance.
            {
                const int top = selected.top();
                const int bottom = selected.bottom();
                int dashes = 0;
                // (Clear of its rounded corners and the pixel of lane after it.)
                for (int px = x(11.0) + 6; px <= x(13.0) - 7; ++px) {
                    QCOMPARE(dragging.pixelColor(px, top), theme.clipSelectedRingOuter);
                    QCOMPARE(dragging.pixelColor(px, top + 1), theme.clipSelectedRing);
                    QCOMPARE(dragging.pixelColor(px, bottom - 1), theme.clipSelectedRing);
                    QCOMPARE(dragging.pixelColor(px, bottom), theme.clipSelectedRingOuter);
                    dashes += dragging.pixel(px, top + 3) == qRgb(255, 255, 255) ? 1 : 0;
                }
                QVERIFY(dashes > 8);
            }
            QVERIFY(near(timeline.grab().toImage().pixelColor(x(13.0), captions), gridded));
        }
        // At 00:00:00, where every project opens, the playhead's head shows
        // whole, clear of the track headers.
        timeline.setPlayhead(0.0);
        const QImage start = timeline.grab().toImage();
        for (int y = 6; y <= 14; ++y)
            for (int px = kTimelineOrigin - 4; px <= kTimelineOrigin + 4; ++px)
                QCOMPARE(start.pixelColor(px, y), editorTheme().playhead);
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
        QCOMPARE(timeline.addLaneButtonCenter().y(), kRulerHeight + kTrackHeight + (kAddLaneHeight - 1) / 2);
    }

    void timelineHeaderShowsVideoPreview() {
        MediaArtwork artwork;
        TimelineWidget timeline;
        timeline.resize(700, 240);
        timeline.setArtwork(&artwork);
        const QJsonObject clip{{"sourceId", "sample"}, {"timelineStart", 0}, {"duration", 2}};
        const QJsonObject track{{"type", "video"}, {"clips", QJsonArray{clip}}};
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{track}}}, 4, 24);
        timeline.show();
        // A preview needs a header of two lines.
        timeline.setAllTrackHeights(kDetailedTrackHeight);
        const QPoint thumbnail(kTimelineLabelWidth - 9 - kHeaderPreviewWidth / 2, kRulerHeight + kDetailedTrackHeight / 2);
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), editorTheme().trackHeader);
        QImage frame(160, 90, QImage::Format_RGB32);
        frame.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("sample"), frame);
        emit artwork.changed();
        QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail), QColor("#e87342"));
        // A one-line header, at the default height or any below two lines,
        // has no room for it.
        for (const int height : {kDetailedTrackHeight - 1, kVideoTrackHeight, kMinTrackHeight}) {
            timeline.setAllTrackHeights(height);
            QCOMPARE(timeline.grab().toImage().pixelColor(thumbnail.x(), kRulerHeight + height / 2), editorTheme().trackHeader);
        }
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

    void timelinePinsHeadersAndRulerWhileScrolling() {
        QScrollArea scroll;
        scroll.resize(640, 260);
        scroll.setWidgetResizable(true);
        auto *timeline = new TimelineWidget;
        scroll.setWidget(timeline);
        QJsonArray tracks;
        for (int i = 0; i < 12; ++i)
            tracks.append(QJsonObject{{"id", QString::number(i)}, {"type", "video"}, {"clips", QJsonArray{}}});
        timeline->setProject(QJsonObject{{"tracks", tracks}}, 120, 24);
        scroll.show();
        QTest::qWait(20);
        scroll.horizontalScrollBar()->setValue(2000);
        scroll.verticalScrollBar()->setValue(150);
        QTest::qWait(20);
        QCOMPARE(scroll.horizontalScrollBar()->value(), 2000);
        QCOMPARE(scroll.verticalScrollBar()->value(), 150);
        const QImage view = scroll.viewport()->grab().toImage();
        QCOMPARE(view.pixelColor(4, 4), editorTheme().rulerCorner);
        QCOMPARE(view.pixelColor(6, 60), editorTheme().trackHeader);
        QCOMPARE(view.pixelColor(kTimelineLabelWidth + 50, kRulerHeight - 1), editorTheme().divider);

        QSignalSpy seeks(timeline, &TimelineWidget::seekRequested);
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QCOMPARE(seeks.count(), 0);
        QTest::mouseMove(timeline, timeline->mapFrom(scroll.viewport(), QPoint(60, 60)));
        QVERIFY(timeline->skimLabel().isEmpty());
        QTest::mouseClick(timeline, Qt::LeftButton, Qt::NoModifier,
                          timeline->mapFrom(scroll.viewport(), QPoint(kTimelineOrigin + 72, 10)));
        QVERIFY(!seeks.isEmpty());
        QVERIFY(std::abs(seeks.constLast()[0].toDouble() - 2072.0 / 72.0) < 0.05);
    }

    void timelineTrackHeightsFollowTheHeaderEdge() {
        TimelineWidget timeline;
        timeline.resize(700, 400);
        const QJsonObject project{{"tracks", QJsonArray{
            QJsonObject{{"id", "v"}, {"type", "video"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "a"}, {"type", "audio"}, {"clips", QJsonArray{}}}}}};
        timeline.setProject(project, 8, 24);
        timeline.show();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
        int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge));
        QTest::mouseMove(&timeline, QPoint(40, edge + 30));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge + 30));
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).top(), kRulerHeight + kVideoTrackHeight + 30);
        edge = timeline.trackHeaderRect(1).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, edge));
        QTest::mouseMove(&timeline, QPoint(40, 0));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(40, 0));
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        // Heights belong to the track, so a new snapshot keeps them.
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight + 30);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kMinTrackHeight);
        timeline.fitTracksToView();
        QCOMPARE(timeline.trackHeaderRect(0).height(), timeline.trackHeaderRect(1).height());
        QVERIFY(timeline.trackHeaderRect(0).height() > kVideoTrackHeight);
        QVERIFY(timeline.trackHeaderRect(1).bottom() + kTrackHeight + 8 <= timeline.height());
        timeline.resetTrackHeights();
        QCOMPARE(timeline.trackHeaderRect(0).height(), kVideoTrackHeight);
        QCOMPARE(timeline.trackHeaderRect(1).height(), kTrackHeight);
    }

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

    // The skimmed playhead keeps the playhead's halo, so its line reads at
    // 3:1 where it crosses clips and pictures, not only on bare lanes.
    void skimmedPlayheadKeepsItsHaloOverClips() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.resize(800, 200);
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
            QJsonObject{{"id", "c"}, {"kind", "media"}, {"timelineStart", 0.0}, {"duration", 6.0}}}}}}}}, 8, 24);
        timeline.setPlayhead(2.0);
        timeline.show();
        const int playX = kTimelineOrigin + 144;
        const QRect clip = timeline.clipBounds(QStringLiteral("c"));
        QVERIFY(clip.left() < playX - 1 && clip.right() > playX + 1);
        // In the clip's body, under its name bar.
        const int y = clip.bottom() - 4;
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            QTest::mouseMove(&timeline, QPoint(kTimelineOrigin + (scheme == ColorScheme::Dark ? 420 : 400), clip.center().y()));
            QVERIFY(!timeline.skimLabel().isEmpty());
            const QImage skimmed = timeline.grab().toImage();
            QCOMPARE(skimmed.pixel(playX, y), theme.playheadIdle.rgb());
            QCOMPARE(skimmed.pixel(playX - 1, y), theme.playheadHalo.rgb());
            QCOMPARE(skimmed.pixel(playX + 1, y), theme.playheadHalo.rgb());
            QVERIFY(contrastRatio(theme.playheadIdle, theme.playheadHalo) >= 3.0);
            // Not skimming, the playhead has the same halo.
            timeline.clearSkimmer();
            const QImage playing = timeline.grab().toImage();
            QCOMPARE(playing.pixel(playX, y), theme.playhead.rgb());
            QCOMPARE(playing.pixel(playX - 1, y), theme.playheadHalo.rgb());
            QCOMPARE(playing.pixel(playX + 1, y), theme.playheadHalo.rgb());
        }
    }

    // The header bar sets every track's height at once (Mini, the default
    // Medium, Large, and Fit) and shows which is in force; in the editor it
    // is in the Timeline panel's title strip, before the track count.
    void timelineHeaderBarSetsEveryTrackHeight() {
        TimelineWidget timeline;
        timeline.resize(900, 520);
        timeline.setProject(trackList({"video", "video", "audio", "audio"}), 8, 24);
        timeline.show();
        const QList<QAction *> actions = timeline.trackHeightActions();
        QStringList names;
        for (const QAction *action : actions) names << action->objectName();
        QCOMPARE(names, (QStringList{"trackHeightMini", "trackHeightMedium", "trackHeightLarge", "trackHeightFit"}));
        QAction *mini = actions.at(0);
        QAction *medium = actions.at(1);
        QAction *large = actions.at(2);
        QAction *fit = actions.at(3);
        for (QAction *action : actions) QVERIFY(action->toolTip().contains(QStringLiteral("Option-scroll")));
        // Tracks start dense (eight fit the timeline at 1440x900): one-line
        // headers, a filmstrip under a picture clip's name and a waveform
        // under a sound clip's; Mini is one line of 18px.
        QVERIFY(kVideoTrackHeight < kDetailedTrackHeight && kVideoTrackHeight >= 28);
        QVERIFY(kTrackHeight <= kVideoTrackHeight && kTrackHeight >= 22);
        QCOMPARE(kMinTrackHeight, 18);
        QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kVideoTrackHeight, kTrackHeight, kTrackHeight}));
        QVERIFY(medium->isChecked() && !mini->isChecked() && !large->isChecked() && !fit->isCheckable());
        mini->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(4, kMinTrackHeight));
        QVERIFY(mini->isChecked() && !medium->isChecked());
        large->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(4, 60));
        QVERIFY(large->isChecked() && !mini->isChecked());
        // One track dragged apart: no height is every track's.
        const int edge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge));
        QTest::mouseMove(&timeline, QPoint(140, edge + 10));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 10));
        QCOMPARE(trackHeights(timeline), (QList<int>{70, 60, 60, 60}));
        QVERIFY(!mini->isChecked() && !medium->isChecked() && !large->isChecked());
        medium->trigger();
        QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kVideoTrackHeight, kTrackHeight, kTrackHeight}));
        QVERIFY(medium->isChecked());
        fit->trigger();
        const int each = (timeline.height() - kRulerHeight - kAddLaneHeight - 8) / 4;
        QCOMPARE(trackHeights(timeline), QList<int>(4, std::clamp(each, kMinTrackHeight, kMaxTrackHeight)));
        // Their symbols are the glyph colour, and the accent when chosen.
        QCOMPARE(ink(mini->icon()), editorTheme().glyph.rgb());
        QCOMPARE(ink(mini->icon(), QIcon::Normal, QIcon::On), editorTheme().accentText.rgb());

        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject project = trackList({"video", "video", "audio"});
        project.insert("width", 1920);
        project.insert("height", 1080);
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", project}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ads::CDockWidget *dock = window.m_workspace->panel(QStringLiteral("timeline"));
        QCOMPARE(dock->titleBarActions(), window.m_timeline->trackHeightActions());
        auto *button = dock->dockAreaWidget()->findChild<QToolButton *>(QStringLiteral("trackHeightMini"));
        QVERIFY(button && button->isVisible());
        QTest::mouseClick(button, Qt::LeftButton);
        QCOMPARE(trackHeights(*window.m_timeline), QList<int>(3, kMinTrackHeight));
        QVERIFY(button->isChecked());
        // The heights are one bank of keys, edge to edge, the chosen one
        // held in: the bank's well at its ends, the key's face inside.
        QCOMPARE(button->size(), QSize(20, 24));
        auto *fitButton = dock->dockAreaWidget()->findChild<QToolButton *>(QStringLiteral("trackHeightFit"));
        QVERIFY(fitButton && fitButton->isVisible());
        QCOMPARE(fitButton->geometry().left(), button->geometry().left() + 3 * 20);
        QTest::mouseMove(&window, QPoint(4, window.height() - 4));
        QTRY_VERIFY(!button->underMouse());
        const QImage chosen = button->grab().toImage();
        const qreal ratio = chosen.devicePixelRatio();
        QCOMPARE(chosen.pixelColor(QPoint(1, 12) * ratio).rgb(), editorTheme().bankWell.rgb());
        QCOMPARE(chosen.pixelColor(QPoint(15, 15) * ratio).rgb(), editorTheme().keyLatched.rgb());
        // Its legend is engraved; the action keeps its name and tip.
        QCOMPARE(button->defaultAction(), window.m_timeline->trackHeightActions().at(0));
        QCOMPARE(button->toolTip(), window.m_timeline->trackHeightActions().at(0)->toolTip());
    }

    // The header bar's track heights as the dock's title strip makes them:
    // a button for each action, which the timeline paints as one bank of
    // latching keys (S, M, L, Fit) in its well, in either appearance.
    void timelineTrackHeightsAreABankOfKeys() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.setProject(trackList({"video", "audio"}), 8, 24);
        QWidget strip;
        auto *layout = new QHBoxLayout(&strip);
        layout->setContentsMargins(10, 0, 10, 0);
        layout->setSpacing(0);
        QList<QToolButton *> buttons;
        for (QAction *action : timeline.trackHeightActions()) {
            auto *button = new QToolButton(&strip);
            button->setDefaultAction(action);
            button->setAutoRaise(true);
            button->setFixedSize(20, 24);
            layout->addWidget(button);
            buttons << button;
        }
        strip.show();
        QVERIFY(QTest::qWaitForWindowExposed(&strip));
        // The timeline takes them over when its heights next change.
        timeline.updateHeightActions();
        QVERIFY(buttons.at(1)->isChecked());
        // A row of a key's face below its legend, clear of its rounded
        // corners at x = 15: the well stands 2px inside a button, the keys
        // 2px inside the well above its 1px lip, and the face's corners round
        // its last rows.
        const int faceRow = buttons.at(0)->height() - 9;
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = strip.grab().toImage();
            const qreal ratio = image.devicePixelRatio();
            const auto at = [&](int button, int x, int y) {
                return image.pixelColor((buttons.at(button)->pos() + QPoint(x, y)) * ratio).rgb();
            };
            // Outside the bank, the strip; at its ends and between its keys, its well.
            QCOMPARE(image.pixelColor(QPoint(5, 12) * ratio).rgb(), strip.palette().window().color().rgb());
            QCOMPARE(at(0, 1, 12), theme.bankWell.rgb());
            QCOMPARE(at(3, 18, 12), theme.bankWell.rgb());
            for (int button = 1; button < 4; ++button) QCOMPARE(at(button, 0, 12), theme.bankWell.rgb());
            // Medium, the default, is held in; the others stand raised, a
            // face between the key's two tones (clear of the legends).
            QCOMPARE(at(1, 15, faceRow), theme.keyLatched.rgb());
            for (const int raised : {0, 2, 3}) {
                const QColor face = QColor::fromRgb(at(raised, 15, faceRow));
                const auto between = [&](int (QColor::*channel)() const) {
                    const int a = (theme.keyTop.*channel)();
                    const int b = (theme.keyBottom.*channel)();
                    return (face.*channel)() >= std::min(a, b) - 1 && (face.*channel)() <= std::max(a, b) + 1;
                };
                QVERIFY2(between(&QColor::red) && between(&QColor::green) && between(&QColor::blue),
                         qPrintable(face.name()));
            }
        }
        // A choice made elsewhere latches its key.
        timeline.trackHeightActions().at(0)->trigger();
        const QImage mini = strip.grab().toImage();
        const qreal ratio = mini.devicePixelRatio();
        QCOMPARE(mini.pixelColor((buttons.at(0)->pos() + QPoint(15, faceRow)) * ratio).rgb(), editorTheme().keyLatched.rgb());
        QVERIFY(mini.pixelColor((buttons.at(1)->pos() + QPoint(15, faceRow)) * ratio).rgb() != editorTheme().keyLatched.rgb());
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
        QCOMPARE(trackHeights(*timeline), heights(kVideoTrackHeight, kTrackHeight));
        timeline->setFocus();
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(30, 26));
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QTest::keyClick(timeline, Qt::Key_Down, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        QTest::keyClick(timeline, Qt::Key_Down);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));

        const auto wheel = [timeline](const QPoint &at, int delta, Qt::KeyboardModifiers modifiers,
                                      Qt::ScrollPhase phase = Qt::NoScrollPhase) {
            QWheelEvent event(at, timeline->mapToGlobal(at), QPoint(), QPoint(0, delta), Qt::NoButton, modifiers, phase, false);
            QApplication::sendEvent(timeline, &event);
            return event.isAccepted();
        };
        const auto inView = [timeline, &scroll](const QPoint &point) { return timeline->mapFrom(scroll.viewport(), point); };
        QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        // A trackpad's small deltas add up to a notch.
        for (int part = 0; part < 3; ++part) wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        // Without Option, or over the lanes, the wheel is the scroll area's.
        QVERIFY(!wheel(inView(QPoint(120, 200)), 120, Qt::NoModifier));
        QVERIFY(!wheel(inView(QPoint(500, 200)), 120, Qt::AltModifier));
        QCOMPARE(trackHeights(*timeline), heights(22, 18));
        // The momentum after a flick steps nothing, and a new gesture starts
        // its notch afresh.
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollBegin));
        QVERIFY(wheel(inView(QPoint(120, 200)), 80, Qt::AltModifier, Qt::ScrollUpdate));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        for (int event = 0; event < 6; ++event) QVERIFY(wheel(inView(QPoint(120, 200)), 120, Qt::AltModifier, Qt::ScrollMomentum));
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollEnd);
        wheel(inView(QPoint(120, 200)), 0, Qt::AltModifier, Qt::ScrollBegin);
        wheel(inView(QPoint(120, 200)), -80, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(26, 22));
        wheel(inView(QPoint(120, 200)), -40, Qt::AltModifier, Qt::ScrollUpdate);
        QCOMPARE(trackHeights(*timeline), heights(22, 18));

        // The track under the pointer stays where it is in the view.
        scroll.verticalScrollBar()->setValue(300);
        QCoreApplication::processEvents();
        const QPoint over = inView(QPoint(120, 250));
        const int row = timeline->trackRowAt(over.y());
        QVERIFY(row > 5);
        const auto top = [timeline](int at) { return timeline->trackHeaderRect(at).top() + timeline->pos().y(); };
        const int before = top(row);
        wheel(over, -120, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(18, 18));
        QCOMPARE(top(row), before);
        // (Over the same track, now shorter.)
        wheel(QPoint(over.x(), timeline->trackHeaderRect(row).top() + 2), 240, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(26, 26));
        QCOMPARE(top(row), before);
        // The keys keep the track at the top of the view in place.
        const int first = timeline->trackRowAt(-timeline->pos().y() + kRulerHeight);
        const int firstTop = top(first);
        QTest::keyClick(timeline, Qt::Key_Up, Qt::AltModifier);
        QCOMPARE(trackHeights(*timeline), heights(30, 30));
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
        // Rows, top down: V3, V2, V1, A1, A2. Option-drag picture to 100.
        const int pictureEdge = timeline.trackHeaderRect(2).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(140, pictureEdge + 100 - kVideoTrackHeight));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge + 100 - kVideoTrackHeight));
        QCOMPARE(trackHeights(timeline), (QList<int>{100, 100, 100, kTrackHeight, kTrackHeight}));
        // V3, the hero layer, taller still.
        const int heroEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, heroEdge));
        QTest::mouseMove(&timeline, QPoint(140, heroEdge + 20));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, heroEdge + 20));
        QCOMPARE(trackHeights(timeline), (QList<int>{120, 100, 100, kTrackHeight, kTrackHeight}));
        timeline.stepAllTrackHeights(1);
        const QList<int> taller = trackHeights(timeline);
        QCOMPARE(taller, (QList<int>{128, 104, 104, 36, 36}));
        timeline.stepAllTrackHeights(-1);
        QCOMPARE(trackHeights(timeline), (QList<int>{104, 86, 86, 30, 30}));
        // The tallest stays the tallest; Option-scroll steps the same way.
        QWheelEvent wheel(QPointF(120, timeline.trackHeaderRect(3).center().y()), timeline.mapToGlobal(QPointF(120, 0)), QPoint(),
                          QPoint(0, 120), Qt::NoButton, Qt::AltModifier, Qt::NoScrollPhase, false);
        QApplication::sendEvent(&timeline, &wheel);
        QCOMPARE(trackHeights(timeline), (QList<int>{128, 104, 104, 36, 36}));
        // A new track takes the step every track without a height of its
        // own was taken to: one step from the picture default.
        QJsonArray tracks = project.value("tracks").toArray();
        tracks.append(QJsonObject{{"id", "added"}, {"type", "audio"}, {"clips", QJsonArray{}}});
        project.insert("tracks", tracks);
        timeline.setProject(project, 8, 24);
        QCOMPARE(timeline.allTrackHeight(), 36);
        QCOMPARE(trackHeights(timeline).constLast(), 36);
        // Presets still set every track alike.
        timeline.trackHeightActions().at(0)->trigger();
        QCOMPARE(trackHeights(timeline), QList<int>(6, kMinTrackHeight));
    }

    // At every height down to Mini the line between two headers shows the
    // resize cursor and says what dragging it does; dragging it resizes the
    // track above, Option-dragging every track of that kind; and a header's
    // toggles keep their clicks, however close to the line.
    void timelineHeaderEdgesResizeAtEveryHeight() {
        TimelineWidget timeline;
        timeline.resize(800, 400);
        timeline.setProject(trackList({"video", "video", "video", "audio", "audio"}), 8, 24);
        timeline.setAllTrackHeights(kMinTrackHeight);
        timeline.show();
        QSignalSpy toggles(&timeline, &TimelineWidget::trackToggleRequested);
        // Rows, top down: V3, V2, V1, A1, A2. This line parts V2 from V1.
        const int edge = timeline.trackHeaderRect(1).bottom() + 1;
        for (const int dy : {-2, 0, 2}) {
            QTest::mouseMove(&timeline, QPoint(140, edge + dy));
            QCOMPARE(timeline.cursor().shape(), Qt::SplitVCursor);
        }
        QHelpEvent help(QEvent::ToolTip, QPoint(140, edge), timeline.mapToGlobal(QPoint(140, edge)));
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
        QCOMPARE(trackHeights(timeline), QList<int>(5, kMinTrackHeight));

        QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge - 1));
        QTest::mouseMove(&timeline, QPoint(140, edge + 19));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 19));
        QCOMPARE(trackHeights(timeline), (QList<int>{18, 38, 18, 18, 18}));
        // Option: every sound track...
        const int soundEdge = timeline.trackHeaderRect(3).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, soundEdge));
        QTest::mouseMove(&timeline, QPoint(140, soundEdge + 12));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, soundEdge + 12));
        QCOMPARE(trackHeights(timeline), (QList<int>{18, 38, 18, 30, 30}));
        // ...or every picture track.
        const int pictureEdge = timeline.trackHeaderRect(0).bottom() + 1;
        QTest::mousePress(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge));
        QTest::mouseMove(&timeline, QPoint(140, pictureEdge + 6));
        QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::AltModifier, QPoint(140, pictureEdge + 6));
        QCOMPARE(trackHeights(timeline), (QList<int>{24, 24, 24, 30, 30}));
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
            QTest::mousePress(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge));
            QTest::mouseMove(&timeline, QPoint(140, edge + 24));
            QTest::mouseRelease(&timeline, Qt::LeftButton, Qt::NoModifier, QPoint(140, edge + 24));
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
            // Whether the snapshot comes first or not.
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
        // The editor remembers them for the project it opens, and Medium
        // (the defaults) forgets them.
        MainWindow window{first.path(), QString()};
        QCOMPARE(window.m_timeline->m_heightsPath, remembered);
        window.m_timeline->setProject(project, 8, 24);
        QCOMPARE(trackHeights(*window.m_timeline), (QList<int>{50, 26}));
        window.m_timeline->resetTrackHeights();
        TimelineWidget reopened;
        reopened.setHeightsKey(first.path());
        reopened.setProject(project, 8, 24);
        QCOMPARE(trackHeights(reopened), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        // Left at Mini, it reopens at Mini with Mini checked in the title
        // bar, the remembered heights arriving before any track does.
        const auto checkedHeight = [](const TimelineWidget &timeline) {
            QStringList checked;
            for (const QAction *action : timeline.trackHeightActions())
                if (action->isChecked()) checked << action->objectName();
            return checked;
        };
        reopened.trackHeightActions().at(0)->trigger();
        {
            TimelineWidget atMini;
            atMini.setHeightsKey(first.path());
            atMini.setProject(project, 8, 24);
            QCOMPARE(trackHeights(atMini), QList<int>(2, kMinTrackHeight));
            QCOMPARE(checkedHeight(atMini), QStringList{QStringLiteral("trackHeightMini")});
        }
        {
            MainWindow atMini{first.path(), QString()};
            atMini.setConnection(QStringLiteral("ready"));
            atMini.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 8}, {"timeline", project}});
            QCOMPARE(trackHeights(*atMini.m_timeline), QList<int>(2, kMinTrackHeight));
            QCOMPARE(checkedHeight(*atMini.m_timeline), QStringList{QStringLiteral("trackHeightMini")});
        }
        // A snapshot that removes the one track sized apart leaves every
        // track at one height, and its button checked.
        reopened.trackHeightActions().at(2)->trigger();
        reopened.setTrackHeight(1, 40);
        QVERIFY(checkedHeight(reopened).isEmpty());
        QJsonObject shrunk = project;
        QJsonArray tracks = shrunk.value("tracks").toArray();
        tracks.removeLast();
        shrunk.insert("tracks", tracks);
        reopened.setProject(shrunk, 8, 24);
        QCOMPARE(checkedHeight(reopened), QStringList{QStringLiteral("trackHeightLarge")});
    }

    // From Mini to the tallest, a header's badge and toggles sit inside its
    // row, clear of its top edge and its divider; it is one line (no clip
    // count, no preview) below two lines' height; clips keep their names at
    // every height, and show their filmstrip or waveform from a 20px clip
    // (rows of 23px, and 25px up, where the inset grows to 2px), the name
    // on a chip over it until the clip has room for a name bar too.
    void timelineTracksStayLegibleAtEveryHeight() {
        MediaArtwork artwork;
        QImage picture(160, 90, QImage::Format_RGB32);
        picture.fill(QColor("#e87342"));
        artwork.frames.insert(QStringLiteral("pic"), picture);
        artwork.peaks.insert(QStringLiteral("snd"), fullScalePeaks(4.0));
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(QJsonObject{{"fps", 24},
            {"sources", QJsonArray{QJsonObject{{"id", "pic"}, {"duration", 4}}, QJsonObject{{"id", "snd"}, {"duration", 4}}}},
            {"tracks", QJsonArray{
                QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{QJsonObject{
                    {"id", "v"}, {"kind", "media"}, {"sourceId", "pic"}, {"name", "Opening shot"}, {"timelineStart", 0.0},
                    {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}}}}},
                QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{QJsonObject{
                    {"id", "a"}, {"kind", "media"}, {"sourceId", "snd"}, {"name", "Dialogue"}, {"timelineStart", 0.0},
                    {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}}}}}}}}, 6, 24);
        timeline.resize(900, kRulerHeight + 2 * kMaxTrackHeight + kAddLaneHeight + 8);
        timeline.show();
        const EditorTheme &theme = editorTheme();
        // Whether any pixel across a clip's body, near its foot, is `color`.
        // A picture's frames fill the clip to its foot; a sound's waveform
        // is around its middle, under the name chip.
        const auto bodyShows = [](const QImage &image, const QRect &clip, const QColor &color) {
            const bool picture = color == QColor("#e87342");
            const int y = picture ? clip.bottom() - 3 : clip.center().y() + 2;
            for (int x = clip.center().x() - 20; x <= clip.center().x() + 20; ++x) {
                const QColor pixel = image.pixelColor(x, y);
                if (picture ? pixel == color : isWaveInk(pixel)) return true;
            }
            return false;
        };
        for (const int height : {18, 19, 20, 21, 22, 23, 24, 25, 26, 28, 29, 30, 32, 33, 34, 36, 39, 40, 44, 48, 49, 60, 96, 160}) {
            timeline.setAllTrackHeights(height);
            QCOMPARE(trackHeights(timeline), QList<int>(2, height));
            const QImage image = timeline.grab().toImage();
            for (int row = 0; row < 2; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                const QRect badge = timeline.headerBadgeRect(row);
                const QRect output = timeline.headerToggleRect(row, 0);
                const QRect lock = timeline.headerToggleRect(row, 1);
                for (const QRect &control : {badge, output, lock}) {
                    QVERIFY2(control.top() > header.top() && control.bottom() < header.bottom(),
                             qPrintable(QStringLiteral("row %1 at %2px").arg(row).arg(height)));
                    QVERIFY(control.height() >= 12);
                }
                QVERIFY(badge.right() < output.left() && output.right() < lock.left());
                // The number on its channel plate's well; V1's output key
                // held in (it plays), A1's mute key and both locks raised,
                // each on the header (sampled clear of the symbols).
                QCOMPARE(image.pixelColor(badge.left() + 2, badge.center().y()), theme.wellFill);
                QCOMPARE(image.pixelColor(badge.left() - 1, badge.center().y()), theme.trackHeader);
                QCOMPARE(image.pixelColor(output.center().x(), output.bottom() - 2) == theme.keyLatched, row == 0);
                for (const QRect &raised : row == 0 ? QList<QRect>{lock} : QList<QRect>{output, lock}) {
                    QVERIFY(image.pixelColor(raised.center().x(), raised.bottom() - 2) != theme.keyLatched);
                    QVERIFY(image.pixelColor(raised.center().x(), raised.bottom() - 2) != theme.trackHeader);
                }
                QCOMPARE(image.pixelColor(lock.right() + 2, lock.center().y()), theme.trackHeader);
                // The divider under the row is whole, past the name.
                QCOMPARE(image.pixelColor(170, header.bottom()), theme.trackDivider);
            }
            const QRect picture = timeline.clipBounds(QStringLiteral("v"));
            const QRect wave = timeline.clipBounds(QStringLiteral("a"));
            QCOMPARE(bodyShows(image, picture, QColor("#e87342")), height >= 22);
            QCOMPARE(bodyShows(image, wave, theme.clipAudio.wave), height >= 22);
            // Names: the clips', the tracks', the badges'; the clip count
            // only on two-line headers.
            TextRecorder recorder;
            QPainter painter(&recorder);
            timeline.paintLanes(painter, timeline.rect());
            timeline.paintHeaders(painter, timeline.rect());
            painter.end();
            for (const char *name : {"Opening shot", "Dialogue", "Video 1", "Audio 1", "V1", "A1"})
                QVERIFY2(recorder.colorOf(QString::fromLatin1(name)).isValid(), name);
            // Each number in its clips' colour, made to read in the well.
            QCOMPARE(recorder.colorOf(QStringLiteral("V1")), readableOn(theme.clipVideo.header, theme.wellFill, 4.5));
            QCOMPARE(recorder.colorOf(QStringLiteral("A1")), readableOn(theme.clipAudio.header, theme.wellFill, 4.5));
            QCOMPARE(recorder.colorOf(QStringLiteral("1 clip")).isValid(), height >= kDetailedTrackHeight);
        }
    }

    // A big edit's track numbers (V100 and up) fit their badges whole, and
    // the toggles after the badges line up down the column.
    void timelineBadgesFitEveryTrackNumber() {
        QStringList kinds;
        for (int index = 0; index < 160; ++index) kinds << (index < 120 ? "video" : "audio");
        TimelineWidget timeline;
        timeline.setProject(trackList(kinds), 8, 24);
        timeline.setAllTrackHeights(kMinTrackHeight);
        timeline.resize(800, 600);
        TextRecorder recorder;
        QPainter painter(&recorder);
        timeline.paintHeaders(painter, timeline.rect());
        painter.end();
        QStringList badges;
        for (const auto &run : recorder.runs)
            if (run.text.size() <= 5 && (run.text.startsWith(QLatin1Char('V')) || run.text.startsWith(QLatin1Char('A')))) badges << run.text;
        QVERIFY2(badges.contains(QStringLiteral("V120")) && badges.contains(QStringLiteral("V100")), qPrintable(badges.join(' ')));
        for (const QString &badge : badges) QVERIFY2(!badge.contains(QChar(0x2026)), qPrintable(badge));
        QVERIFY(timeline.headerBadgeRect(0).width() >=
                QFontMetrics(timeline.badgeFont()).horizontalAdvance(QStringLiteral("V120")) + 4);
        for (int row = 0; row < timeline.trackRowCount(); row += 37) {
            QCOMPARE(timeline.headerToggleRect(row, 0).left(), timeline.headerToggleRect(0, 0).left());
            QVERIFY(timeline.headerToggleRect(row, 0).left() > timeline.headerBadgeRect(row).right());
        }
    }

    // The header toggles are small latching keys: raised when off, held in
    // when on, brighter under the pointer, their symbols engraved; and the
    // track numbers' inks read in their plates, in either appearance.
    void timelineHeaderTogglesAreLatchingKeys() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.resize(800, 300);
        timeline.setProject(QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "V2"}, {"type", "video"}, {"enabled", false}, {"locked", true}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "A1"}, {"type", "audio"}, {"muted", true}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "A2"}, {"type", "audio"}, {"name", "Music"}, {"clips", QJsonArray{}}}}}}, 8, 24);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        // Rows, top down: V2 (hidden, locked), V1, A1 (muted), A2.
        // A key's face, under its symbol.
        const auto face = [&timeline](const QImage &image, int row, int index) {
            const QRect key = timeline.headerToggleRect(row, index);
            return image.pixelColor(key.center().x(), key.bottom() - 2);
        };
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            QCOMPARE(face(image, 0, 0) == theme.keyLatched, false); // Hidden: its output key is out.
            QCOMPARE(face(image, 0, 1), theme.keyLatched);          // Locked.
            QCOMPARE(face(image, 1, 0), theme.keyLatched);          // Plays.
            QCOMPARE(face(image, 1, 1) == theme.keyLatched, false);
            QCOMPARE(face(image, 2, 0), theme.keyLatched);          // Muted.
            QCOMPARE(face(image, 3, 0) == theme.keyLatched, false);
            // Symbols engraved, readable on a key raised or held in.
            TextRecorder recorder;
            QPainter painter(&recorder);
            timeline.paintHeaders(painter, timeline.rect());
            painter.end();
            QCOMPARE(recorder.colorOf(QStringLiteral("M")), theme.engravedInk);
            for (const QColor &key : {theme.keyTop, theme.keyBottom, theme.keyLatched})
                QVERIFY2(contrastRatio(theme.engravedInk, key) >= 4.5, qPrintable(key.name()));
            // Each track's number reads on its plate at 4.5:1.
            for (const ClipStyle *style : {&theme.clipVideo, &theme.clipAudio, &theme.clipMusic, &theme.clipGenerator,
                                           &theme.clipCaption})
                QVERIFY(contrastRatio(readableOn(style->header, theme.wellFill, 4.5), theme.wellFill) >= 4.5);
            QCOMPARE(recorder.colorOf(QStringLiteral("A2")), readableOn(theme.clipMusic.header, theme.wellFill, 4.5));
            // A hidden track's number is faint, as its name is.
            QCOMPARE(recorder.colorOf(QStringLiteral("V2")), theme.trackHeaderTextDisabled);
        }
        // Under the pointer a key brightens, and only that key.
        const QImage before = timeline.grab().toImage();
        QTest::mouseMove(&timeline, timeline.headerToggleRect(1, 1).center());
        const QImage hovered = timeline.grab().toImage();
        QVERIFY(face(hovered, 1, 1) != face(before, 1, 1));
        QCOMPARE(face(hovered, 1, 0), face(before, 1, 0));
        QCOMPARE(face(hovered, 2, 1), face(before, 2, 1));
        QTest::mouseMove(&timeline, QPoint(400, 200));
        QCOMPARE(face(timeline.grab().toImage(), 1, 1), face(before, 1, 1));
    }

    // The corner time sits on a display glass sized to its figures, cream
    // for the playhead and the glass's dimmer label ink while skimming.
    void timelineCornerTimeSitsOnDisplayGlass() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.resize(800, 200);
        timeline.setProject(trackList({"video", "audio"}), 8, 24);
        timeline.setPlayhead(7.5);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        const QString playing = timelineTimecode(7.5, 24);
        const QRect glass = timeline.cornerGlassRect(QPoint(), playing);
        // Its figures and 7px either side, inside the corner over the headers.
        QVERIFY(glass.width() >= QFontMetrics(TimelineWidget::cornerFont()).horizontalAdvance(playing) + 14);
        QVERIFY(glass.width() <= QFontMetrics(TimelineWidget::cornerFont()).horizontalAdvance(playing) + 22);
        QVERIFY(glass.left() > 4 && glass.top() > 0 && glass.bottom() < kRulerHeight - 1);
        QVERIFY(glass.right() < kTimelineLabelWidth - 20);
        // Figures of one width: the glass keeps its size as the time runs.
        QCOMPARE(timeline.cornerGlassRect(QPoint(), timelineTimecode(3599.0, 24)).size(), glass.size());
        QCOMPARE(TimelineWidget::cornerFont().family(), monoFont(12).family());
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            QCOMPARE(image.pixelColor(4, 4), theme.rulerCorner);
            QCOMPARE(image.pixelColor(glass.right() + 3, glass.center().y()), theme.rulerCorner);
            // Dark glass, between its two tones, clear of the figures.
            const QColor inside = image.pixelColor(glass.left() + 3, glass.bottom() - 4);
            QVERIFY2(inside.lightness() <= std::max(theme.glassTop.lightness(), theme.glassBottom.lightness()) + 2,
                     qPrintable(inside.name()));
            TextRecorder recorder;
            QPainter painter(&recorder);
            timeline.paintCornerTime(painter, QPoint(), 7.5, theme.glassText);
            painter.end();
            QCOMPARE(recorder.colorOf(playing), theme.glassText);
        }
        // The overlay picks the ink: cream for the playhead, the label ink
        // for the skimmer.
        const auto cornerInk = [&timeline](const QString &time) {
            TextRecorder recorder;
            QPainter painter(&recorder);
            timeline.paintOverlay(painter, timeline.rect());
            painter.end();
            return recorder.colorOf(time);
        };
        QCOMPARE(cornerInk(playing), editorTheme().glassText);
        timeline.showSkimmer(3.0);
        QCOMPARE(cornerInk(timelineTimecode(3.0, 24)), editorTheme().glassLabel);
    }

    // The timeline header's zoom: an engraved ZOOM, then − and + keys either
    // side of a short fader whose bakelite cap stands where the slider's
    // handle is, so a press on the cap drags it; tips and names as before.
    void timelineZoomIsAFaderBetweenKeys() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        TimelineWidget timeline;
        timeline.setProject(trackList({"video"}), 60, 24);
        TimelineZoomControl zoom(&timeline);
        zoom.show();
        QVERIFY(QTest::qWaitForWindowExposed(&zoom));
        QSlider *fader = zoom.slider();
        QCOMPARE(zoom.label(), QStringLiteral("Zoom"));
        QCOMPARE(zoom.zoomOutButton()->accessibleName(), QStringLiteral("Zoom out"));
        QCOMPARE(zoom.zoomInButton()->accessibleName(), QStringLiteral("Zoom in"));
        QCOMPARE(fader->accessibleName(), QStringLiteral("Timeline zoom"));
        QCOMPARE(fader->toolTip(), QStringLiteral("Timeline zoom"));
        QVERIFY(zoom.zoomInButton()->autoRepeat() && zoom.zoomOutButton()->autoRepeat());
        // The label, then −, the fader, +, left to right.
        QVERIFY(zoom.labelRect().right() < zoom.zoomOutButton()->x());
        QVERIFY(zoom.zoomOutButton()->geometry().right() < fader->x() && fader->geometry().right() < zoom.zoomInButton()->x());
        QCOMPARE(fader->width(), 72);
        // The cap stands on the handle, at either end of the travel at
        // either end of the range, as wide as a key's legend.
        fader->setValue(0);
        const QRectF low = TimelineZoomControl::faderCap(fader);
        QCOMPARE(low.width(), 10.0);
        QVERIFY(low.left() >= 0 && low.left() < 8);
        fader->setValue(TimelineZoomControl::kSteps);
        const QRectF high = TimelineZoomControl::faderCap(fader);
        QVERIFY(high.right() <= fader->width() && high.right() > fader->width() - 8);
        QCOMPARE(low.left(), fader->width() - high.right());
        fader->setValue(20);
        const QPoint grip = TimelineZoomControl::faderCap(fader).center().toPoint();
        QTest::mousePress(fader, Qt::LeftButton, Qt::NoModifier, grip);
        QTest::mouseMove(fader, grip + QPoint(20, 0));
        QTest::mouseRelease(fader, Qt::LeftButton, Qt::NoModifier, grip + QPoint(20, 0));
        QVERIFY2(fader->value() > 40, qPrintable(QString::number(fader->value())));
        QCOMPARE(timeline.scale(), TimelineZoomControl::scaleAt(fader->value()));
        // Painted from the theme in either appearance: a cream index line
        // down the dark cap, the groove's well beside it, the keys raised.
        fader->setValue(50);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = zoom.grab().toImage();
            const qreal ratio = image.devicePixelRatio();
            const QRectF cap = TimelineZoomControl::faderCap(fader).translated(fader->pos());
            const auto at = [&](const QPointF &point) { return image.pixelColor((point * ratio).toPoint()); };
            QCOMPARE(at(QPointF(cap.center().x(), cap.center().y())).rgb(), theme.knobPointer.rgb());
            QVERIFY(at(QPointF(cap.left() + 2.5, cap.center().y())).lightness() < 80);
            QCOMPARE(at(QPointF(fader->x() + 14.5, cap.center().y() + 0.5)).rgb(), theme.wellFill.rgb());
            const QRect out = zoom.zoomOutButton()->geometry();
            QCOMPARE(at(QPointF(out.left() + 4.5, out.bottom() - 4.5)) == theme.keyLatched, false);
            QVERIFY(at(QPointF(out.left() + 4.5, out.bottom() - 4.5)).rgb() != image.pixelColor(0, 0).rgb());
        }
    }

    // Pictures of the track headers, the corner time and the header bar's
    // zoom and track heights, to look at: run with
    // KERF_TIMELINE_HEADER_GALLERY set to a folder (and QT_SCALE_FACTOR=2
    // for 2x), in either appearance.
    void timelineHeaderGallery() {
        const QString folder = qEnvironmentVariable("KERF_TIMELINE_HEADER_GALLERY");
        if (folder.isEmpty()) QSKIP("Set KERF_TIMELINE_HEADER_GALLERY to draw the track headers and header bar.");
        QDir().mkpath(folder);
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        const QJsonObject clip{{"id", "c"}, {"timelineStart", 0.0}, {"duration", 4.0}};
        TimelineWidget timeline;
        timeline.setProject(QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V3"}, {"type", "video"}, {"name", "Captions"}, {"clips", QJsonArray{QJsonObject{
                {"id", "cap"}, {"kind", "caption"}, {"timelineStart", 0.0}, {"duration", 4.0}}}}},
            QJsonObject{{"id", "V2"}, {"type", "video"}, {"enabled", false}, {"locked", true}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip}}},
            QJsonObject{{"id", "A1"}, {"type", "audio"}, {"muted", true}, {"clips", QJsonArray{}}},
            QJsonObject{{"id", "A2"}, {"type", "audio"}, {"name", "Music"}, {"clips", QJsonArray{}}}}}}, 8, 24);
        timeline.setPlayhead(7.5);
        timeline.resize(520, 260);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        QTest::mouseMove(&timeline, timeline.headerToggleRect(2, 1).center());
        QWidget strip;
        auto *layout = new QHBoxLayout(&strip);
        layout->setContentsMargins(10, 4, 10, 4);
        layout->setSpacing(8);
        layout->addWidget(new TimelineZoomControl(&timeline, &strip));
        auto *bank = new QWidget(&strip);
        auto *keys = new QHBoxLayout(bank);
        keys->setContentsMargins(0, 0, 0, 0);
        keys->setSpacing(0);
        for (QAction *action : timeline.trackHeightActions()) {
            auto *button = new QToolButton(bank);
            button->setDefaultAction(action);
            button->setFixedSize(20, 24);
            keys->addWidget(button);
        }
        layout->addWidget(bank);
        strip.show();
        QVERIFY(QTest::qWaitForWindowExposed(&strip));
        timeline.updateHeightActions();
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            QCoreApplication::processEvents();
            const QString look = scheme == ColorScheme::Dark ? QStringLiteral("dark") : QStringLiteral("light");
            for (const int height : {0, kMinTrackHeight, 60}) {
                if (height > 0) timeline.setAllTrackHeights(height);
                else timeline.resetTrackHeights();
                QCoreApplication::processEvents();
                QVERIFY(timeline.grab().save(QDir(folder).filePath(QStringLiteral("%1-headers-%2.png").arg(look).arg(height))));
            }
            timeline.resetTrackHeights();
            QVERIFY(strip.grab().save(QDir(folder).filePath(QStringLiteral("%1-header-bar.png").arg(look))));
        }
        // And the Timeline panel's own title strip, in the window.
        QSettings().remove(QStringLiteral("workspace"));
        const auto forget = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 4}, {"timeline", trackList({"video", "audio"})}});
        window.resize(1440, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            QCoreApplication::processEvents();
            const QString look = scheme == ColorScheme::Dark ? QStringLiteral("dark") : QStringLiteral("light");
            QWidget *bar = window.m_workspace->panel(QStringLiteral("timeline"))->dockAreaWidget()->titleBar();
            QVERIFY(bar->grab().save(QDir(folder).filePath(QStringLiteral("%1-timeline-title.png").arg(look))));
        }
    }

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
        // Small enough that the whole timeline could be cached at once.
        QVERIFY(qint64(timeline->width()) * timeline->height() < 6000000);
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
            // Where they differ, to tell a stale strip from a stray pixel.
            QRect differ;
            int count = 0;
            if (scrolled.size() == repainted.size())
                for (int y = 0; y < scrolled.height(); ++y)
                    for (int x = 0; x < scrolled.width(); ++x)
                        if (scrolled.pixel(x, y) != repainted.pixel(x, y)) {
                            differ |= QRect(x, y, 1, 1);
                            ++count;
                        }
            QVERIFY2(scrolled == repainted,
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

    // A dense edit: 48 tracks at Mini, one line each, in both appearances.
    void denseTimelineFollowsTheAppearance() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        MediaArtwork artwork;
        fakeArtwork(&artwork);
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(denseTimeline(48, 24.0), 24.0, 24.0);
        timeline.setSelectedClip(QStringLiteral("c20_2"));
        timeline.setPlayhead(7.5);
        timeline.trackHeightActions().at(0)->trigger();
        timeline.resize(1440, kRulerHeight + 48 * kMinTrackHeight + kAddLaneHeight + 8);
        timeline.show();
        QCOMPARE(trackHeights(timeline), QList<int>(48, kMinTrackHeight));
        const int playX = kTimelineOrigin + int(std::floor(7.5 * 72.0));
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = timeline.grab().toImage();
            if (!shots.isEmpty())
                QVERIFY(image.save(QDir(shots).filePath(QStringLiteral("timeline-dense-%1.png").arg(theme.dark ? "dark" : "light"))));
            for (int row = 0; row < 48; ++row) {
                const QRect header = timeline.trackHeaderRect(row);
                QVERIFY(header.bottom() < image.height());
                // Its stripe is its clips' colour; its line of controls fits it.
                const bool sound = timeline.trackRowLabel(row).startsWith(QLatin1Char('A'));
                QCOMPARE(image.pixelColor(1, header.center().y()), (sound ? theme.clipAudio : theme.clipVideo).header);
                QCOMPARE(image.pixelColor(170, header.bottom()), theme.trackDivider);
                // The playhead's line runs through every row.
                QCOMPARE(image.pixelColor(playX, header.center().y()), theme.playhead);
            }
            QCOMPARE(amberPixels(image), 0);
        }
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
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"type", "video"}, {"clips", QJsonArray{}}}}}}, 90, 24);
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

    // At the default heights a picture clip is all filmstrip and a sound
    // clip all waveform, each name on a chip over the corner; a separate
    // name bar comes back from the Large preset up. The filmstrip's tiles
    // are the picture's own shape (a vertical clip's tall and narrow, no
    // bars), each the frame under its left edge.
    // Pictures of a real clip's filmstrip and waveform at the default and
    // taller heights, to look at: run with KERF_TIMELINE_GALLERY set to a
    // folder and KERF_GALLERY_MEDIA to a clip with picture and sound
    // (QT_SCALE_FACTOR=2 for a Retina screen's pixels).
    void timelineGallery() {
        const QString folder = qEnvironmentVariable("KERF_TIMELINE_GALLERY");
        const QString media = qEnvironmentVariable("KERF_GALLERY_MEDIA");
        if (folder.isEmpty() || media.isEmpty()) QSKIP("Set KERF_TIMELINE_GALLERY and KERF_GALLERY_MEDIA to draw the gallery.");
        const QFileInfo file(media);
        const double duration = 12.0;
        MediaArtwork artwork;
        artwork.request(QJsonArray{QJsonObject{{"id", "m"}, {"path", file.fileName()}, {"width", 1080}, {"height", 1920},
                                               {"duration", duration}, {"hasAudio", true}}}, file.absolutePath());
        QTRY_VERIFY_WITH_TIMEOUT(artwork.strips.contains(QStringLiteral("m")) && artwork.peaks.contains(QStringLiteral("m")), 120000);
        const auto clip = [](const char *id, double start, double length, double in) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", "m"}, {"name", "voice_take"},
                               {"timelineStart", start}, {"duration", length}, {"in", in}, {"out", in + length},
                               {"link", QStringLiteral("link-%1").arg(QChar(id[1]))}};
        };
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(QJsonObject{{"fps", 30}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("v1", 0.0, 5.0, 0.0), clip("v2", 5.0, 7.0, 5.0)}}},
            QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{clip("a1", 0.0, 5.0, 0.0), clip("a2", 5.0, 7.0, 5.0)}}}}}},
            duration, 30);
        timeline.resize(1200, 260);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        QDir().mkpath(folder);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const QString look = scheme == ColorScheme::Dark ? QStringLiteral("dark") : QStringLiteral("light");
            timeline.resetTrackHeights();
            QCoreApplication::processEvents();
            QVERIFY(timeline.grab().save(QDir(folder).filePath(QStringLiteral("%1-default.png").arg(look))));
            for (const int height : {60, 96}) {
                timeline.setAllTrackHeights(height);
                QCoreApplication::processEvents();
                QVERIFY(timeline.grab().save(QDir(folder).filePath(QStringLiteral("%1-tracks-%2.png").arg(look).arg(height))));
            }
        }
        // The suite runs in the dark appearance (initTestCase).
        setEditorColorScheme(ColorScheme::Dark);
    }

    // Picture, sound and music clips are one colour in either appearance,
    // their waveforms white on it; and a picture clip with sound shows it in
    // a band under its frames from the Large preset up, as a Final Cut clip
    // does, never at the default height.
    void timelineClipsKeepTheirColoursAndShowSoundUnderPicture() {
        const EditorTheme light = makeEditorTheme(false);
        const EditorTheme dark = makeEditorTheme(true);
        for (const auto &[a, b] : {std::pair{&light.clipVideo, &dark.clipVideo}, std::pair{&light.clipAudio, &dark.clipAudio},
                                   std::pair{&light.clipMusic, &dark.clipMusic}}) {
            QCOMPARE(a->fill.rgba(), b->fill.rgba());
            QCOMPARE(a->header.rgba(), b->header.rgba());
            QCOMPARE(a->border.rgba(), b->border.rgba());
        }
        QCOMPARE(light.waveformBody.rgba(), dark.waveformBody.rgba());
        QCOMPARE(light.waveformPeak.rgba(), dark.waveformPeak.rgba());
        QCOMPARE(light.waveformBody.rgb(), QColor(Qt::white).rgb());
        // White reads on every sound colour.
        for (const ClipStyle *style : {&light.clipAudio, &light.clipMusic})
            QVERIFY2(contrastRatio(Qt::white, style->fill) >= 4.5, qPrintable(style->fill.name()));

        MediaArtwork artwork;
        Filmstrip strip;
        strip.interval = 1.0;
        QImage frame(96, 54, QImage::Format_RGB32);
        frame.fill(QColor("#e87342"));
        strip.frames << frame;
        artwork.strips.insert(QStringLiteral("pic"), strip);
        artwork.peaks.insert(QStringLiteral("pic"), fullScalePeaks(4.0));
        const auto project = [](bool withSound, double gain = 0.0) {
            QJsonArray tracks{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{QJsonObject{
                {"id", "v"}, {"kind", "media"}, {"sourceId", "pic"}, {"name", "Shot"}, {"timelineStart", 0.0},
                {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}, {"link", "L1"}}}}}};
            if (withSound)
                tracks.append(QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{QJsonObject{
                    {"id", "a"}, {"kind", "media"}, {"sourceId", "pic"}, {"name", "Shot"}, {"timelineStart", 0.0},
                    {"duration", 4.0}, {"in", 0.0}, {"out", 4.0}, {"link", "L1"}, {"props", QJsonObject{{"gain", gain}}}}}}});
            return QJsonObject{{"fps", 24}, {"tracks", tracks}};
        };
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(project(true), 6, 24);
        timeline.resize(900, 260);
        timeline.show();
        const auto inkUnderPicture = [&timeline] {
            const QImage image = timeline.grab().toImage();
            const QRect clip = timeline.clipBounds(QStringLiteral("v"));
            int ink = 0;
            for (int x = clip.left() + 100; x < clip.right() - 10; ++x) ink += isWaveInk(image.pixelColor(x, clip.bottom() - 6)) ? 1 : 0;
            return ink;
        };
        QCOMPARE(inkUnderPicture(), 0);
        timeline.setAllTrackHeights(96);
        QCoreApplication::processEvents();
        QVERIFY(inkUnderPicture() > 20);
        // Its frames still fill the top of the clip.
        const QRect clip = timeline.clipBounds(QStringLiteral("v"));
        QCOMPARE(timeline.grab().toImage().pixelColor(clip.center().x(), clip.top() + 24), QColor("#e87342"));
        // The sound clip's glass is one colour in either appearance, white
        // reading on it at 4.5:1, and nothing of the lane shows through.
        // (Turned down 40 dB, so the glass shows under its waveform.)
        timeline.setProject(project(true, -40.0), 6, 24);
        QList<QRgb> glass;
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const QImage image = timeline.grab().toImage();
            const QRect sound = timeline.clipBounds(QStringLiteral("a"));
            const QColor pane = image.pixelColor(sound.left() + 30, sound.bottom() - 4);
            glass << pane.rgb();
            QVERIFY2(contrastRatio(Qt::white, pane) >= 4.5, qPrintable(pane.name()));
        }
        setEditorColorScheme(ColorScheme::Dark);
        QCOMPARE(glass.at(0), glass.at(1));
        // Unlinked, or with its sound clip gone, a picture clip shows no sound.
        timeline.setProject(project(false), 6, 24);
        timeline.setAllTrackHeights(96);
        QCoreApplication::processEvents();
        QCOMPARE(inkUnderPicture(), 0);
    }

    void timelineDrawsArtworkAtDefaultHeights() {
        const QRectF video(0, 0, 200, kVideoTrackHeight - 5);
        const QRectF sound(0, 0, 200, kTrackHeight - 5);
        for (const QRectF &clip : {video, sound}) {
            const auto layout = TimelineWidget::clipLayout(clip, true);
            QVERIFY(layout.chip);
            QCOMPARE(layout.body, clip);
            QCOMPARE(layout.band.height(), 15.0);
            // Without artwork, a name bar over a plain body, as before.
            QVERIFY(!TimelineWidget::clipLayout(clip, false).chip);
        }
        const QRectF large(0, 0, 200, kTrackHeightPresets[3].height - 5);
        QVERIFY(!TimelineWidget::clipLayout(large, true).chip);
        QCOMPARE(TimelineWidget::clipLayout(large, true).body.height(), large.height() - 15.0);
        QCOMPARE(TimelineWidget::clipLayout(QRectF(0, 0, 200, kMinTrackHeight - 3), true).body.height(), 0.0);

        // Ten frames a second apart, each its own colour.
        const QList<QColor> colors{"#d64545", "#e3a33b", "#d8d23a", "#5fbf4a", "#3fb6a8",
                                   "#3d7fd1", "#6a4fd1", "#b84fd1", "#d14f8e", "#8a8a8a"};
        MediaArtwork artwork;
        Filmstrip strip;
        strip.interval = 1.0;
        for (const QColor &color : colors) {
            QImage frame(54, 96, QImage::Format_RGB32);
            frame.fill(color);
            strip.frames << frame;
        }
        artwork.strips.insert(QStringLiteral("tall"), strip);
        artwork.peaks.insert(QStringLiteral("snd"), fullScalePeaks(10.0));
        TimelineWidget timeline;
        timeline.setArtwork(&artwork);
        timeline.setProject(QJsonObject{{"fps", 24}, {"tracks", QJsonArray{
            QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{QJsonObject{
                {"id", "v"}, {"kind", "media"}, {"sourceId", "tall"}, {"name", "Vertical"}, {"timelineStart", 0.0},
                {"duration", 5.0}, {"in", 3.0}, {"out", 8.0}}}}},
            QJsonObject{{"id", "A1"}, {"type", "audio"}, {"clips", QJsonArray{QJsonObject{
                {"id", "a"}, {"kind", "media"}, {"sourceId", "snd"}, {"name", "Dialogue"}, {"timelineStart", 0.0},
                {"duration", 5.0}, {"in", 0.0}, {"out", 5.0}}}}}}}}, 6, 24);
        timeline.resize(900, 200);
        timeline.show();
        QCOMPARE(trackHeights(timeline), (QList<int>{kVideoTrackHeight, kTrackHeight}));
        const QImage image = timeline.grab().toImage();
        const QRect picture = timeline.clipBounds(QStringLiteral("v"));
        // A tall picture's tiles are square, its middle cut out.
        const int tileWidth = picture.height();
        const int y = picture.bottom() - 3;
        // Past the name chip, every tile is the frame under its left edge,
        // all the way across, meeting without a seam: no bars, so no pixel
        // is black.
        for (int x = picture.left() + 80; x < picture.right() - 4; ++x) {
            const QColor pixel = image.pixelColor(x, y);
            const int tileLeft = picture.left() + (x - picture.left()) / tileWidth * tileWidth;
            const double seconds = 3.0 + (tileLeft - picture.left()) / timeline.scale();
            QVERIFY2(pixel == colors.at(strip.frameAt(seconds)),
                     qPrintable(QStringLiteral("x %1: %2").arg(x).arg(pixel.name())));
        }
        // The sound clip: its RMS, white on the clip's colour, reaches the
        // foot of the clip.
        const QRect wave = timeline.clipBounds(QStringLiteral("a"));
        QVERIFY(isWaveInk(image.pixelColor(wave.center().x(), wave.bottom() - 3)));
        QVERIFY(isWaveInk(image.pixelColor(wave.center().x(), wave.top() + 3)));
        // Clip gain scales it: at -12 dB the top of the clip is clear.
        QJsonObject project = timeline.m_project;
        QJsonArray tracks = project.value("tracks").toArray();
        QJsonObject track = tracks.at(1).toObject();
        QJsonObject quiet = track.value("clips").toArray().at(0).toObject();
        quiet.insert("props", QJsonObject{{"gain", -12.0}});
        track.insert("clips", QJsonArray{quiet});
        tracks.replace(1, track);
        project.insert("tracks", tracks);
        timeline.setProject(project, 6, 24);
        const QImage quieter = timeline.grab().toImage();
        QVERIFY(!isWaveInk(quieter.pixelColor(wave.center().x(), wave.top() + 3)));
        QVERIFY(isWaveInk(quieter.pixelColor(wave.center().x(), wave.center().y())));
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
        // An hour stays at least 80px per second, so the pointer can address
        // frames instead of collapsing onto a handful of pixels.
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
        QVERIFY(scroll.verticalScrollBar()->maximum() > 0);
        QVERIFY(timeline->minimumHeight() >= kRulerHeight + 12 * kTrackHeight);
        timeline->setZoom(1.0);
        QTest::qWait(20);
        QVERIFY(scroll.horizontalScrollBar()->maximum() > 0);
    }

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

    // A clip a proposal adds is named on a solid band, so its name reads
    // over a clip already in the track (an overwrite's, a replacement's) as
    // over the bare lane.
    void proposedClipNamesReadOverClipsAlreadyThere() {
        const auto restore = qScopeGuard([] { setEditorColorScheme(ColorScheme::Dark); });
        const QJsonObject wide{{"id", "a"}, {"kind", "media"}, {"timelineStart", 0.0}, {"duration", 8.0}};
        const QJsonObject inserted{{"id", "n"}, {"kind", "media"}, {"name", "Insert"}, {"timelineStart", 2.0}, {"duration", 4.0}};
        const auto timeline = [](const QJsonArray &clips) {
            return QJsonObject{{"fps", 24}, {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", clips}}}}};
        };
        TimelineWidget widget;
        widget.setProject(timeline(QJsonArray{wide}), 10, 24);
        widget.setProposal(timeline(QJsonArray{wide, inserted}));
        widget.resize(1000, 200);
        widget.show();
        const QRect existing = widget.clipBounds(QStringLiteral("a"));
        QVERIFY(existing.width() > 200);
        // Near the added clip's right end (6 s), clear of its name and its dashes,
        // inside its name's band, over the existing clip's name bar.
        const QPoint band(existing.left() + existing.width() * 3 / 4 - 10, existing.top() + 7);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = widget.grab().toImage();
            const QColor drawn = image.pixelColor(band * image.devicePixelRatio());
            const QColor expected = laidOn(theme.trackLane, theme.proposalAddFill);
            QVERIFY2(std::abs(drawn.red() - expected.red()) <= 2 && std::abs(drawn.green() - expected.green()) <= 2 &&
                         std::abs(drawn.blue() - expected.blue()) <= 2,
                     qPrintable(QStringLiteral("%1, not %2").arg(drawn.name(), expected.name())));
            QVERIFY(contrastRatio(theme.assistantText, drawn) >= 4.5);
        }

        // On a locked track the band keeps the lane's hatch, so the lock
        // shows under the name too, and the name still reads on it.
        QJsonObject locked = timeline(QJsonArray{wide});
        QJsonObject lockedProposal = timeline(QJsonArray{wide, inserted});
        for (QJsonObject *object : {&locked, &lockedProposal}) {
            QJsonArray tracks = object->value("tracks").toArray();
            QJsonObject track = tracks.at(0).toObject();
            track.insert("locked", true);
            tracks.replace(0, track);
            object->insert("tracks", tracks);
        }
        widget.setProject(locked, 10, 24);
        widget.setProposal(lockedProposal);
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark}) {
            setEditorColorScheme(scheme);
            const EditorTheme &theme = editorTheme();
            const QImage image = widget.grab().toImage();
            const auto near = [](const QColor &a, const QColor &b) {
                return std::abs(a.red() - b.red()) <= 2 && std::abs(a.green() - b.green()) <= 2 && std::abs(a.blue() - b.blue()) <= 2;
            };
            const QColor bare = laidOn(theme.trackLane, theme.proposalAddFill);
            const QColor hatch = laidOn(laidOn(theme.trackLane, theme.trackLaneLocked), theme.proposalAddFill);
            QVERIFY(!near(bare, hatch));
            // Along the band, clear of the name: the hatch's lines cross it.
            int lines = 0;
            int plain = 0;
            for (int x = band.x() - 16; x <= band.x() + 16; ++x) {
                const QColor drawn = image.pixelColor(QPoint(x, band.y()) * image.devicePixelRatio());
                lines += near(drawn, hatch) ? 1 : 0;
                plain += near(drawn, bare) ? 1 : 0;
            }
            QVERIFY2(lines >= 3 && plain >= 3, qPrintable(QStringLiteral("%1 hatch, %2 plain").arg(lines).arg(plain)));
            QVERIFY(contrastRatio(theme.assistantText, hatch) >= 4.5);
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::TimelineTest)

#include "timeline_test.moc"
