// Tests for the scrub strip (comment lane, bar, transport, marks and loop),
// the stage rail (the dBFS meter, LUFS M, the device menu, the comments
// badge) and the agent HUD (mockup-spec 3.3-3.5).

#include "fixtures.h"
#include "test_registry.h"

#include "agent_hud.h"
#include "components.h"
#include "editor_bus.h"
#include "main_window.h"
#include "media_artwork.h"
#include "program_level.h"
#include "review_model.h"
#include "scrub_strip.h"
#include "stage_rail.h"
#include "workspace.h"

#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QCoreApplication>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QMenu>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <QWidget>

#include <cmath>

namespace kerf {

using namespace test;

namespace {

// ---- fixtures: the mockup's threads and proposals (mockup-spec 4.9) ----------

QJsonObject person(const char *name, const char *email, const char *role) {
    return QJsonObject{{"kind", "person"}, {"name", name}, {"id", email}, {"role", role}};
}

QJsonObject agent(const char *kind, const char *name, const char *id, const char *role) {
    QJsonObject author{{"kind", kind}, {"name", name}, {"role", role}};
    if (id) author.insert("id", id);
    return author;
}

QJsonObject note(const char *id, const QJsonObject &author, const char *created, const char *body,
                 std::initializer_list<std::pair<const char *, QJsonValue>> extra = {}) {
    QJsonObject record{{"id", id},           {"parent", QJsonValue()}, {"author", author}, {"body", body},
                       {"tags", QJsonArray{}}, {"blocking", false},    {"resolved", false}, {"created", created},
                       {"revision", 22}};
    for (const auto &[key, value] : extra) record.insert(key, value);
    return record;
}

// The six timed threads of the lane: Mina's resolved opener, Tom's colour
// note, Kerf's caption overlap (ranged), Mina's logo note, QC's true peak
// (ranged) and Codex's resolved end card.
QJsonArray laneComments() {
    const QJsonObject mina = person("Mina Park", "mina@kerf.test", "director");
    return QJsonArray{
        note("c-0300", mina, "2026-11-02T09:00:00.000000Z", "Open on the road, not the sky",
             {{"at", 3.0}, {"resolved", true}}),
        note("c-0903", person("Tom Lind", "tom@kerf.test", "colourist"), "2026-11-02T10:00:00.000000Z",
             "The sea reads magenta here", {{"at", 9.12}}),
        note("c-1708", QJsonObject{{"kind", "kerf"}, {"name", "Kerf"}, {"role", "QC · captions"}},
             "2026-11-03T08:00:00.000000Z", "Caption overlaps the lower-third by 8 px", {{"at", 17.32}, {"duration", 2.48}}),
        note("c-2614", mina, "2026-11-03T09:00:00.000000Z", "Logo lands late, should hit on the snare",
             {{"at", 26.56}, {"blocking", true}}),
        note("c-2718", agent("qc", "QC", nullptr, "loudness"), "2026-11-03T10:00:00.000000Z",
             "True peak −0.4 dBTP, limit −1.0 dBTP", {{"at", 27.72}, {"duration", 1.08}, {"blocking", true}}),
        note("c-2916", agent("mcp", "Codex", "codex", "MCP"), "2026-11-01T12:00:00.000000Z",
             "End card holds a beat long, trimmed 12 frames", {{"at", 29.64}, {"resolved", true}}),
    };
}

// The program at 25 fps, 30 s; the logo (and its whoosh 0.28 s later) at `logoAt`.
QJsonObject programTimeline(double logoAt) {
    const auto clip = [](const char *id, const char *name, double start, double duration) {
        return QJsonObject{{"id", id}, {"name", name}, {"kind", "media"}, {"timelineStart", start}, {"duration", duration}};
    };
    return QJsonObject{
        {"fps", 25},
        {"width", 1920},
        {"height", 1080},
        {"tracks", QJsonArray{
                       QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip("road", "A012_C004", 0.0, 8.48)}}},
                       QJsonObject{{"id", "V4"}, {"type", "video"}, {"clips", QJsonArray{clip("logo", "Logo_reveal", logoAt, 30.0 - logoAt)}}},
                       QJsonObject{{"id", "A3"}, {"type", "audio"}, {"clips", QJsonArray{clip("whoosh", "Whoosh_logo", logoAt + 0.28, 1.0)}}},
                   }},
    };
}

// Claude Code's pending v2 of the logo move (v1 superseded), as the snapshot
// carries it.
QJsonObject proposalSnapshot() {
    const QJsonObject claude = agent("mcp", "Claude Code", "claude-code", "MCP · via Kerf");
    const QJsonArray versions{
        QJsonObject{{"version", 1}, {"proposalId", "proposal-cc1"}, {"script", "move logo to 00:00:25:21\n"}, {"documentDiff", ""}},
        QJsonObject{{"version", 2}, {"proposalId", "proposal-cc2"}, {"script", "move logo to 00:00:25:20\n"}, {"documentDiff", ""}}};
    const QJsonObject entry{{"proposalId", "proposal-cc2"}, {"status", "pending"}, {"freshness", "current"},
                            {"baseRevision", 22}, {"script", "move logo to 00:00:25:20\n"}, {"source", ""},
                            {"documentDiff", ""}, {"author", claude}, {"version", 2}, {"supersedes", "proposal-cc1"},
                            {"versions", versions}};
    return QJsonObject{{"revision", 22},
                       {"timeline", programTimeline(26.12)},
                       {"proposals", QJsonArray{entry}},
                       {"proposalTimelines", QJsonObject{{"proposal-cc2", programTimeline(25.8)}}}};
}

// A bus as MainWindow keeps one, without a window: what it sends and runs is
// recorded.
struct Harness {
    EditorBus bus;
    ReviewModel review;
    QList<QJsonObject> sent;
    QStringList ran;
    explicit Harness(double duration = 30.0, double fps = 25.0, QSize frame = QSize(1920, 1080)) {
        bus.setSender([this](const QJsonObject &command) {
            sent << command;
            return true;
        });
        bus.setRunner([this](const QString &id, const QJsonObject &) {
            ran << id;
            return true;
        });
        bus.setSnapshot(QJsonObject{}, QJsonObject{}, fps, frame);
        bus.setDuration(duration);
    }
    QJsonObject last() const { return sent.isEmpty() ? QJsonObject{} : sent.last(); }
};

QColor pixel(QWidget *widget, QPointF at) {
    const QImage image = widget->grab().toImage();
    return image.pixelColor((at * image.devicePixelRatio()).toPoint());
}

bool sameColor(const QColor &a, QRgb b, int tolerance = 3) {
    const QColor c = QColor::fromRgba(b);
    return std::abs(a.red() - c.red()) <= tolerance && std::abs(a.green() - c.green()) <= tolerance &&
           std::abs(a.blue() - c.blue()) <= tolerance;
}

QJsonObject meterEvent(QJsonValue peakL, QJsonValue peakR, QJsonValue rmsL, QJsonValue rmsR, QJsonValue momentary) {
    return QJsonObject{{"type", "meter"},
                       {"peak", QJsonArray{peakL, peakR}},
                       {"rms", QJsonArray{rmsL, rmsR}},
                       {"momentary", momentary},
                       {"shortTerm", momentary},
                       {"at", 3.0}};
}

} // namespace

class ScrubRailTest final : public QObject {
    Q_OBJECT

private slots:
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    // ---- the scrub strip --------------------------------------------------

    // One marker per timed thread at t/duration of the lane, clamped 10 in;
    // people round, agents and checks square with their glyphs; the current
    // and resolved states; markers within 20 points merge into a count stack.
    void scrubLaneStacksTheThreadsMarkers() {
        Harness h;
        h.review.setComments(laneComments());
        h.bus.setReview(&h.review);
        QWidget host;
        host.resize(900, 200);
        ScrubStrip strip(&host, &h.bus);
        strip.setGeometry(20, 120, 816, 56);
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));
        QCOMPARE(strip.height(), 56);

        // At the mockup's 816: six markers, none within 20 of the next.
        QList<ReviewModel::MarkerStack> stacks = strip.stacks();
        QCOMPARE(stacks.size(), 6);
        const double expected[] = {3.0 / 30 * 816, 9.12 / 30 * 816, 17.32 / 30 * 816, 26.56 / 30 * 816, 27.72 / 30 * 816,
                                   816 - 10.0};
        for (int i = 0; i < 6; ++i) {
            QVERIFY2(std::abs(stacks.at(i).x - expected[i]) < 1e-6, qPrintable(QString::number(stacks.at(i).x)));
            QCOMPARE(strip.markerRect(i), QRectF(stacks.at(i).x - 10, 0, 20, 20));
        }
        QVERIFY(stacks.at(0).markers.first().circle && stacks.at(0).markers.first().resolved);
        QCOMPARE(stacks.at(1).markers.first().initials, QStringLiteral("TL"));
        QVERIFY(stacks.at(2).markers.first().glyph && !stacks.at(2).markers.first().circle);
        QCOMPARE(stacks.at(4).markers.first().kind, QStringLiteral("qc"));

        // Paint: an open person marker on the on fill, a resolved one clear
        // with ink3 initials, the current one ringed in ink2.
        h.review.setCurrentThread(QStringLiteral("c-0903"));
        const QRectF tom = strip.markerRect(1);
        QVERIFY(sameColor(pixel(&strip, tom.center() + QPointF(-8, 0)), tokens::on));
        const QRectF mina = strip.markerRect(0);
        QVERIFY(!sameColor(pixel(&strip, mina.center() + QPointF(-8, 0)), tokens::on));
        // The current ring sits just outside the marker: lighter than the lane.
        QVERIFY(lightnessAt(strip.grab().toImage(), tom.center() + QPointF(-10.5, 0)) > 60);
        QVERIFY(lightnessAt(strip.grab().toImage(), strip.markerRect(3).center() + QPointF(-10.5, 0)) < 40);

        // At a phone's 360 the last three crowd together: 26.56, 27.72 and
        // 29.64 merge into one stack of 3 (a chain of near marks).
        strip.resize(360, 56);
        stacks = strip.stacks();
        QCOMPARE(stacks.size(), 4);
        QCOMPARE(stacks.last().markers.size(), 3);
        QVERIFY(stacks.last().isStack());
        QCOMPARE(stacks.last().markers.first().id, QStringLiteral("c-2614"));
        QVERIFY(sameColor(pixel(&strip, strip.markerRect(3).center() + QPointF(-8, 0)), tokens::raised));
        // No review, or no length: no markers.
        h.bus.setReview(nullptr);
        QVERIFY(strip.stacks().isEmpty());
    }

    // Hovering a marker shows the 280-wide popover 8 above it, clamped to the
    // strip; a click opens its thread, makes it current and seeks to it (sent
    // as a seek, or emitted for the stage when it listens); the bar seeks
    // where it is clicked, with a 4-point slop.
    void clickingAMarkerOpensItsThreadAndSeeks() {
        Harness h;
        h.review.setComments(laneComments());
        h.bus.setReview(&h.review);
        QWidget host;
        host.resize(900, 300);
        ScrubStrip strip(&host, &h.bus);
        strip.setGeometry(20, 220, 816, 56);
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));

        const QPoint tom = strip.markerRect(1).center().toPoint();
        QTest::mouseMove(&strip, tom);
        QTRY_VERIFY(strip.popoverShown());
        QCOMPARE(strip.hoveredStack(), 1);
        QWidget *pop = strip.popover();
        QCOMPARE(pop->parentWidget(), &host);
        const QMargins m = ui::shadowMargins();
        const QRect card = pop->geometry().marginsRemoved(m);
        QCOMPARE(card.width(), ScrubStrip::kPopoverWidth);
        QCOMPARE(card.bottom() + 1, 220 - 8);
        QCOMPARE(card.left(), qRound(20 + strip.markerRect(1).left() + 8 - 140));
        // Near the left end it stays on the strip.
        QTest::mouseMove(&strip, strip.markerRect(0).center().toPoint());
        QTRY_COMPARE(strip.hoveredStack(), 0);
        QCOMPARE(strip.popover()->geometry().marginsRemoved(m).left(), 20);
        QTest::mouseMove(&strip, QPoint(400, 45));
        QTRY_VERIFY(!strip.popoverShown());

        QSignalSpy opened(&h.bus, &EditorBus::openThread);
        QTest::mouseClick(&strip, Qt::LeftButton, {}, tom);
        QCOMPARE(opened.count(), 1);
        QCOMPARE(opened.first().first().toString(), QStringLiteral("c-0903"));
        QCOMPARE(h.review.currentThread(), QStringLiteral("c-0903"));
        QCOMPARE(h.last(), (QJsonObject{{"type", "seek"}, {"position", 9.12}, {"scrub", false}}));

        // The bar: a click 4 under it still seeks, at x / width of the length.
        QTest::mouseClick(&strip, Qt::LeftButton, {}, QPoint(408, ScrubStrip::kBarTop + ScrubStrip::kBarHeight + 3));
        QCOMPARE(h.last().value("type").toString(), QStringLiteral("seek"));
        QVERIFY(std::abs(h.last().value("position").toDouble() - 15.0) < 1e-9);
        const qsizetype before = h.sent.size();
        QTest::mouseClick(&strip, Qt::LeftButton, {}, QPoint(200, 45));
        QCOMPARE(h.sent.size(), before);

        // With the stage listening, the seek is its to queue.
        QSignalSpy seeks(&strip, &ScrubStrip::seekRequested);
        QTest::mouseClick(&strip, Qt::LeftButton, {}, strip.markerRect(2).center().toPoint());
        QCOMPARE(seeks.count(), 1);
        QCOMPARE(seeks.first().first().toDouble(), 17.32);
        QCOMPARE(h.sent.size(), before);
    }

    // The ranged threads draw on the bar from their time to their end: the
    // current in the accent, resolved faint, the rest grey at .6; the played
    // part follows the playhead.
    void scrubBarShowsRangesAndThePlayhead() {
        Harness h;
        h.review.setComments(laneComments());
        h.bus.setReview(&h.review);
        ScrubStrip strip(nullptr, &h.bus);
        strip.resize(816, 56);
        strip.show();
        QVERIFY(QTest::qWaitForWindowExposed(&strip));
        QList<ScrubStrip::Range> ranges = strip.ranges();
        QCOMPARE(ranges.size(), 2);
        QCOMPARE(ranges.first().id, QStringLiteral("c-1708"));
        QVERIFY(std::abs(ranges.first().rect.left() - 17.32 / 30 * 816) < 1e-6);
        QVERIFY(std::abs(ranges.first().rect.width() - 2.48 / 30 * 816) < 1e-6);
        QCOMPARE(ranges.first().rect.top(), double(ScrubStrip::kBarTop));
        QCOMPARE(ranges.first().color, QColor(138, 138, 138, 153));
        h.review.setCurrentThread(QStringLiteral("c-1708"));
        ranges = strip.ranges();
        QVERIFY(ranges.first().current);
        QCOMPARE(ranges.first().color, tokens::color(tokens::accent));
        QVERIFY(sameColor(pixel(&strip, QPointF(ranges.first().rect.center().x(), 25)), tokens::accent));

        h.bus.setPlayhead(15.0);
        QCOMPARE(strip.playheadX(), 408.0);
        QVERIFY(sameColor(pixel(&strip, QPointF(200, 25)), tokens::track2));
        QVERIFY(sameColor(pixel(&strip, QPointF(600, 25)), tokens::track));
        // The playhead line from the top, and its knob on the bar.
        QVERIFY(sameColor(pixel(&strip, QPointF(408, 4)), tokens::playhead, 6));
        QVERIFY(sameColor(pixel(&strip, QPointF(408, 26)), tokens::playhead, 6));
        QCOMPARE(strip.currentText(), QStringLiteral("00:00:15:00"));
        QCOMPARE(strip.timeText(), QStringLiteral("00:00:15:00 / 00:00:30:00"));
    }

    // The transport runs the registry's playback commands; play reads Pause
    // while playing; the marks set the loop's range. Narrow, the duration is
    // hidden; a strip under the full row's width is narrow by itself.
    void scrubTransportRunsThePlaybackCommands() {
        Harness h;
        ScrubStrip strip(nullptr, &h.bus);
        strip.resize(816, 56);
        strip.show();
        QVERIFY(QTest::qWaitForWindowExposed(&strip));
        const QList<QPair<QAbstractButton *, QString>> buttons{{strip.startButton(), "playback.goToStart"},
                                                              {strip.previousButton(), "playback.previousFrame"},
                                                              {strip.playButton(), "playback.toggle"},
                                                              {strip.nextButton(), "playback.nextFrame"},
                                                              {strip.endButton(), "playback.goToEnd"}};
        for (const auto &[button, command] : buttons) {
            QVERIFY(button->isEnabled());
            button->click();
            QCOMPARE(h.ran.last(), command);
        }
        QCOMPARE(strip.playButton()->size(), QSize(28, 28));
        QCOMPARE(strip.startButton()->size(), QSize(24, 24));
        QCOMPARE(ui::tipName(strip.playButton()), QStringLiteral("Play"));
        h.bus.setPlaying(true);
        QCOMPARE(ui::tipName(strip.playButton()), QStringLiteral("Pause"));

        // Centred between the two 96-point timecodes, 28-point row at y 28.
        const int groupWidth = 52 + 12 + 140 + 12 + 24;
        QCOMPARE(strip.markInButton()->x(), 96 + (816 - 192 - groupWidth) / 2);
        QCOMPARE(strip.playButton()->y(), ScrubStrip::kTransportTop);
        QCOMPARE(strip.durationRect(), QRect(816 - 96, 34, 96, 16));
        QVERIFY(!strip.isNarrow());
        strip.setNarrow(true);
        QVERIFY(strip.isNarrow() && strip.durationRect().isEmpty());
        strip.setNarrow(false);
        strip.resize(360, 56);
        QVERIFY(strip.isNarrow());
        QVERIFY(strip.loopButton()->geometry().right() < 360);

        // A registry action decides: disabled with it, its reason in the tip.
        QAction toggle(QStringLiteral("Play / Pause"));
        toggle.setShortcut(QKeySequence(Qt::Key_Space));
        QAction loop(QStringLiteral("Loop Playback"));
        loop.setEnabled(false);
        loop.setStatusTip(QStringLiteral("Coming in this release"));
        loop.setShortcut(QKeySequence(QStringLiteral("Ctrl+L")));
        strip.setActions({{QStringLiteral("playback.toggle"), &toggle}, {QStringLiteral("playback.loop"), &loop}});
        QCOMPARE(ui::tipShortcut(strip.playButton()), QKeySequence(Qt::Key_Space).toString(QKeySequence::NativeText));
        toggle.setEnabled(false);
        toggle.setStatusTip(QStringLiteral("Offline"));
        QVERIFY(!strip.playButton()->isEnabled());
        QCOMPARE(ui::tipName(strip.playButton()), QStringLiteral("Pause · Offline"));
        // The loop is the strip's own, so it works while its command waits,
        // without claiming the command's key.
        QVERIFY(strip.loopButton()->isEnabled());
        QVERIFY(ui::tipShortcut(strip.loopButton()).isEmpty());
        QCOMPARE(ui::tipName(strip.loopButton()), QStringLiteral("Loop playback"));
    }

    // With the loop on, playback reaching the out mark goes back to the in
    // mark and plays on; playback that ends at the program's end starts
    // again from the start. Without the loop, or paused mid-way, nothing.
    void loopWrapsAtTheOutMark() {
        Harness h;
        ScrubStrip strip(nullptr, &h.bus);
        strip.resize(816, 56);
        QSignalSpy marks(&strip, &ScrubStrip::marksChanged);
        h.bus.setPlayhead(5.0);
        strip.markInButton()->click();
        h.bus.setPlayhead(10.0);
        strip.markOutButton()->click();
        QCOMPARE(marks.count(), 2);
        QCOMPARE(strip.inPoint(), 5.0);
        QCOMPARE(strip.outPoint(), 10.0);
        QCOMPARE(strip.loopStart(), 5.0);
        QCOMPARE(strip.loopEnd(), 10.0);

        strip.loopButton()->click();
        QVERIFY(strip.loop() && strip.loopButton()->isChecked());
        h.sent.clear();
        h.bus.setPlayhead(6.0);
        h.bus.setPlaying(true);
        h.bus.setPlayhead(9.9);
        QVERIFY(h.sent.isEmpty());
        h.bus.setPlayhead(10.0);
        QCOMPARE(h.sent.size(), 1);
        QCOMPARE(h.last(), (QJsonObject{{"type", "seek"}, {"position", 5.0}, {"scrub", false}}));
        // Once per pass: a tick still past the mark sends nothing more.
        h.bus.setPlayhead(10.04);
        QCOMPARE(h.sent.size(), 1);
        h.bus.setPlayhead(5.0);
        h.bus.setPlayhead(10.0);
        QCOMPARE(h.sent.size(), 2);
        // Still playing: no play command.
        QVERIFY(!h.ran.contains(QStringLiteral("playback.toggle")));

        // The program's end: playback stops there by itself, and starts again.
        strip.clearMarks();
        h.sent.clear();
        h.bus.setPlayhead(29.98);
        QVERIFY(h.sent.isEmpty());
        h.bus.setPlaying(false);
        QCOMPARE(h.last(), (QJsonObject{{"type", "seek"}, {"position", 0.0}, {"scrub", false}}));
        QCOMPARE(h.ran, QStringList{QStringLiteral("playback.toggle")});
        // Paused mid-way, the loop leaves it paused.
        h.sent.clear();
        h.ran.clear();
        h.bus.setPlaying(true);
        h.bus.setPlayhead(12.0);
        h.bus.setPlaying(false);
        QVERIFY(h.sent.isEmpty() && h.ran.isEmpty());
        // Loop off: the end is the end.
        strip.loopButton()->click();
        QVERIFY(!strip.loop());
        h.bus.setPlaying(true);
        h.bus.setPlayhead(30.0);
        h.bus.setPlaying(false);
        QVERIFY(h.sent.isEmpty() && h.ran.isEmpty());
        // An out mark before the in mark clears the in mark.
        h.bus.setPlayhead(20.0);
        strip.markIn();
        h.bus.setPlayhead(8.0);
        strip.markOut();
        QVERIFY(!strip.hasIn() && strip.outPoint() == 8.0);
    }

    // ---- the rail ----------------------------------------------------------

    // The rail at the design's 548: the meter at (0, 8, 44, 350) with bars at
    // x 6 and 14, L/R at 362, LUFS M at 390, the device at (6, 438) and the
    // comments at (6, 498), their labels at 472 and 532. A shorter rail keeps
    // the lower group at its foot and gives the meter what is left.
    void railKeepsTheDesignsGeometry() {
        StageRail rail;
        rail.resize(StageRail::kWidth, StageRail::kHeight);
        rail.show();
        QVERIFY(QTest::qWaitForWindowExposed(&rail));
        QCOMPARE(rail.meterRect(), QRect(0, 8, 44, 350));
        QCOMPARE(rail.levelMeter()->geometry(), QRect(0, 8, 44, 350));
        QCOMPARE(rail.barRect(0), QRect(6, 8, 6, 350));
        QCOMPARE(rail.barRect(1), QRect(14, 8, 6, 350));
        QCOMPARE(rail.channelLabelRect(0), QRect(6, 362, 6, 16));
        QCOMPARE(rail.lufsRect(), QRect(0, 390, 44, 32));
        QCOMPARE(rail.lufsReadout()->geometry(), QRect(0, 390, 44, 32));
        QCOMPARE(rail.deviceButton()->geometry(), QRect(6, 438, 32, 32));
        QCOMPARE(rail.chatButton()->geometry(), QRect(6, 498, 32, 32));
        QCOMPARE(rail.deviceLabelRect(), QRect(-8, 472, 60, 16));
        QCOMPARE(rail.commentsLabelRect(), QRect(-8, 532, 60, 16));
        // The scale: 0 6 12 18 24 40 60 at Y(-v) - 8, right-aligned in 24-44.
        QCOMPARE(rail.shownScaleMarks(), (QList<int>{0, 6, 12, 18, 24, 40, 60}));
        QCOMPARE(rail.scaleLabelRect(0), QRect(24, 0, 20, 16));
        QCOMPARE(rail.scaleLabelRect(18), QRect(24, 8 + 105 - 8, 20, 16));
        QCOMPARE(rail.scaleLabelRect(60), QRect(24, 350, 20, 16));
        // The safe zones moved to the Program strip: that button never shows.
        rail.safeZonesButton()->setVisible(true);
        QVERIFY(rail.safeZonesButton()->isHidden());

        rail.resize(StageRail::kWidth, 400);
        QCOMPARE(rail.meterHeight(), 202);
        QCOMPARE(rail.chatButton()->geometry(), QRect(6, 350, 32, 32));
        QCOMPARE(rail.commentsLabelRect().bottom(), 399);
        QVERIFY(!rail.levelMeter()->isHidden());
        rail.resize(StageRail::kWidth, 250);
        QVERIFY(rail.levelMeter()->isHidden());

        // The labels, 60 wide, sit beside the rail in its parent.
        QWidget host;
        host.resize(300, 600);
        auto *placed = new StageRail(&host);
        placed->setGeometry(100, 8, StageRail::kWidth, StageRail::kHeight);
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));
        QCOMPARE(placed->deviceLabelWidget()->parentWidget(), &host);
        QCOMPARE(placed->deviceLabelWidget()->geometry(), QRect(92, 480, 60, 16));
        QCOMPARE(placed->commentsLabelWidget()->geometry(), QRect(92, 540, 60, 16));
        QVERIFY(placed->commentsLabelWidget()->isVisible());
        placed->move(120, 8);
        QCOMPARE(placed->commentsLabelWidget()->geometry().left(), 112);
        placed->hide();
        QVERIFY(!placed->commentsLabelWidget()->isVisible());
        delete placed;
        QVERIFY(host.findChildren<QWidget *>(QStringLiteral("railCommentsLabel")).isEmpty());
    }

    // The meter shows `meter` events only: the RMS level as the bar
    // (track2 to -18, warn to -6, neg above), the peak as a 2-point hold
    // line. −6.4/−3.8 on the left: warn from Y 37, the hold at Y 22.
    void railMeterShowsMeterEvents() {
        Harness h;
        StageRail rail(nullptr, &h.bus);
        rail.resize(StageRail::kWidth, StageRail::kHeight);
        rail.show();
        QVERIFY(QTest::qWaitForWindowExposed(&rail));
        rail.programMeter()->setAutoAdvance(false);
        QWidget *level = rail.levelMeter();
        // No events: the empty track, never an estimate, and no readout.
        QVERIFY(!rail.programMeter()->hasData());
        QCOMPARE(rail.lufsText(), QStringLiteral("—"));
        for (const double y : {20.0, 60.0, 200.0, 340.0})
            QVERIFY(sameColor(pixel(level, QPointF(8, y)), tokens::raised));

        h.bus.dispatch(meterEvent(-3.8, -5.2, -6.4, -8.1, -17.8));
        const ProgramMeter *meter = rail.programMeter();
        QVERIFY(meter->hasData());
        QCOMPARE(meter->level(0), -6.4);
        QCOMPARE(meter->hold(0), -3.8);
        QCOMPARE(rail.lufsText(), QStringLiteral("−17.8"));
        QCOMPARE(ProgramMeter::y(-6.4), 37.0);
        QCOMPARE(ProgramMeter::y(-3.8), 22.0);
        const QList<ProgramMeter::Segment> left = ProgramMeter::segments(-6.4);
        QCOMPARE(left.size(), 2);
        QCOMPARE(left.at(0).top, 105.0);
        QCOMPARE(left.at(0).bottom, 350.0);
        QCOMPARE(left.at(0).color, tokens::track2);
        QCOMPARE(left.at(1).top, 37.0);
        QCOMPARE(left.at(1).bottom, 105.0);
        QCOMPARE(left.at(1).color, tokens::warn);
        QCOMPARE(ProgramMeter::segments(-2.0).last().color, tokens::neg);
        QCOMPARE(ProgramMeter::segments(-2.0).last().top, 12.0);
        QVERIFY(ProgramMeter::segments(-60.0).isEmpty());
        QVERIFY(ProgramMeter::segments(-90.0).isEmpty());

        // Pixels: the left bar's runs and hold, the right bar's at −8.1/−5.2.
        QVERIFY(sameColor(pixel(level, QPointF(8, 200)), tokens::track2));
        QVERIFY(sameColor(pixel(level, QPointF(8, 60)), tokens::warn));
        QVERIFY(sameColor(pixel(level, QPointF(8, 30)), tokens::raised));
        QVERIFY(sameColor(pixel(level, QPointF(8, 23)), tokens::ink));
        QVERIFY(sameColor(pixel(level, QPointF(16, 40)), tokens::raised));
        QVERIFY(sameColor(pixel(level, QPointF(16, 50)), tokens::warn));
        QVERIFY(sameColor(pixel(level, QPointF(16, 31)), tokens::ink));
        // Between the bars, nothing is drawn.
        QVERIFY(!sameColor(pixel(level, QPointF(13, 200)), tokens::track2));
    }

    // The hold stays 1.5 s, then falls 20 dB a second until it meets the
    // peak playing now; with the null reading of a pause the bars empty and
    // the hold falls off the scale.
    void railPeakHoldDecays() {
        Harness h;
        StageRail rail(nullptr, &h.bus);
        ProgramMeter *meter = rail.programMeter();
        meter->setAutoAdvance(false);
        QVERIFY(!meter->ticking());
        h.bus.dispatch(meterEvent(-3.8, -5.2, -6.4, -8.1, -17.8));
        h.bus.dispatch(meterEvent(-20.0, -20.0, -24.0, -24.0, -21.0));
        QCOMPARE(meter->level(0), -24.0);
        QCOMPARE(meter->hold(0), -3.8);
        meter->advance(1.0);
        QCOMPARE(meter->hold(0), -3.8);
        meter->advance(1.0);
        QVERIFY(std::abs(meter->hold(0) - (-13.8)) < 1e-9);
        meter->advance(0.3);
        QVERIFY(std::abs(meter->hold(0) - (-19.8)) < 1e-9);
        meter->advance(0.1);
        QCOMPARE(meter->hold(0), -20.0);
        // A louder peak raises it again.
        h.bus.dispatch(meterEvent(-1.0, -20.0, -9.0, -24.0, -14.0));
        QCOMPARE(meter->hold(0), -1.0);

        // Paused: one reading of nulls.
        h.bus.dispatch(meterEvent(QJsonValue(), QJsonValue(), QJsonValue(), QJsonValue(), QJsonValue()));
        QVERIFY(!meter->hasData() && !meter->hasLevel(0));
        QCOMPARE(rail.lufsText(), QStringLiteral("—"));
        QVERIFY(meter->hasHold(0));
        meter->advance(1.5 + 3.0);
        QVERIFY(!meter->hasHold(0) && !meter->hasHold(1));
        rail.resize(StageRail::kWidth, StageRail::kHeight);
        for (const double y : {10.0, 60.0, 200.0})
            QVERIFY(sameColor(pixel(rail.levelMeter(), QPointF(8, y)), tokens::raised));
        // Its own timer runs only while a hold is up.
        meter->setAutoAdvance(true);
        QVERIFY(!meter->ticking());
        h.bus.dispatch(meterEvent(-6.0, -6.0, -9.0, -9.0, -14.0));
        QVERIFY(meter->ticking());
        QVERIFY(meter->apply(QJsonObject{{"type", "meter"}}));
        QVERIFY(!meter->apply(QJsonObject{{"type", "clock"}}));
    }

    // Clicking the meter or LUFS M runs workspace.finish and asks for the
    // loudness pane; in the editor, the Finish workspace comes forward.
    void railMeterClickOpensTheLoudnessPane() {
        {
            Harness h;
            StageRail rail(nullptr, &h.bus);
            rail.resize(StageRail::kWidth, StageRail::kHeight);
            QSignalSpy highlighted(&h.bus, &EditorBus::highlightPane);
            QSignalSpy requested(&rail, &StageRail::loudnessRequested);
            QTest::mouseClick(rail.levelMeter(), Qt::LeftButton, {}, QPoint(10, 100));
            QCOMPARE(h.ran, QStringList{QStringLiteral("workspace.finish")});
            QCOMPARE(highlighted.count(), 1);
            QCOMPARE(highlighted.first().first().toString(), QStringLiteral("loudness"));
            QTest::mouseClick(rail.lufsReadout(), Qt::LeftButton, {}, QPoint(22, 8));
            QCOMPARE(h.ran.size(), 2);
            QCOMPARE(requested.count(), 2);
        }
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        StageRail *rail = window.m_rail;
        // Placed in the window, the rail hears its bus.
        QCOMPARE(rail->bus(), window.bus());
        window.handleEvent(meterEvent(-3.8, -5.2, -6.4, -8.1, -17.8));
        QCOMPARE(rail->lufsText(), QStringLiteral("−17.8"));
        QSignalSpy highlighted(window.bus(), &EditorBus::highlightPane);
        QTest::mouseClick(rail->lufsReadout(), Qt::LeftButton, {}, QPoint(22, 8));
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Finish"));
        QCOMPARE(highlighted.count(), 1);
        window.close();
    }

    // The badge counts the review's open threads and follows the model;
    // the tip reads "Comments · N open" with the panel's shortcut.
    void railBadgeFollowsTheReview() {
        Harness h;
        StageRail rail(nullptr, &h.bus);
        rail.resize(StageRail::kWidth, StageRail::kHeight);
        QVERIFY(rail.commentsBadge()->isHidden());
        QCOMPARE(rail.openCount(), -1);
        QCOMPARE(ui::tipName(rail.chatButton()), QStringLiteral("Comments"));
        h.bus.setReview(&h.review);
        QCOMPARE(rail.openCount(), 0);
        QVERIFY(rail.commentsBadge()->isHidden());
        h.review.setComments(laneComments());
        QCOMPARE(rail.openCount(), 4);
        QVERIFY(!rail.commentsBadge()->isHidden());
        QCOMPARE(rail.commentsBadge()->text(), QStringLiteral("4"));
        QCOMPARE(ui::tipName(rail.chatButton()), QStringLiteral("Comments · 4 open"));
        // At the button's top right, 6 out, ringed.
        const QRectF badge = rail.commentsBadge()->badgeRect().translated(rail.commentsBadge()->pos());
        QCOMPARE(badge.right(), 6.0 + 32 + 6);
        QCOMPARE(badge.top(), 498.0 - 6);
        QJsonArray items = laneComments();
        QJsonObject tom = items.at(1).toObject();
        tom.insert("resolved", true);
        items.replace(1, tom);
        h.review.setComments(items);
        QCOMPARE(rail.commentsBadge()->text(), QStringLiteral("3"));
        QAction comments(QStringLiteral("Kerf Assistant"));
        comments.setShortcut(QKeySequence(QStringLiteral("Ctrl+Shift+J")));
        rail.setActions(&comments, nullptr);
        QCOMPARE(ui::tipShortcut(rail.chatButton()), comments.shortcut().toString(QKeySequence::NativeText));
        // On while the panel is open (its owner checks it).
        rail.chatButton()->setChecked(true);
        QVERIFY(rail.chatButton()->drawsOn());
        h.bus.setReview(nullptr);
        QVERIFY(rail.commentsBadge()->isHidden());
        QCOMPARE(ui::tipName(rail.chatButton()), QStringLiteral("Comments"));
    }

    // The device button's menu: a header naming the format, then the
    // laptop for 16:9 or the phone's choices (Phone · Feed for 1:1 and 4:5)
    // and Off, each with its glyph; choosing one asks the stage for it. The
    // label under the button reads Laptop, Phone, Feed or Off.
    void railDeviceMenuOffersEachFormatsChoices() {
        Harness h;
        StageRail rail(nullptr, &h.bus);
        rail.resize(StageRail::kWidth, StageRail::kHeight);
        struct Case {
            QSize frame;
            QString header, first, label, icon;
        };
        const QList<Case> cases{
            {QSize(1920, 1080), "Device preview · 16:9", "Laptop", "Laptop", "g-laptop"},
            {QSize(1080, 1920), "Device preview · 9:16", "Phone", "Phone", "g-phone"},
            {QSize(1080, 1080), "Device preview · 1:1", "Phone · Feed", "Feed", "g-feed"},
            {QSize(1080, 1350), "Device preview · 4:5", "Phone · Feed", "Feed", "g-feed"},
        };
        for (const Case &c : cases) {
            h.bus.setSnapshot(QJsonObject{}, QJsonObject{}, 25, c.frame);
            const QList<StageRail::DeviceChoice> choices = rail.deviceChoices();
            QCOMPARE(rail.deviceMenuHeader(), c.header);
            QCOMPARE(choices.size(), 2);
            QCOMPARE(choices.at(0).name, c.first);
            QCOMPARE(choices.at(0).icon, c.icon);
            QVERIFY(choices.at(0).on && choices.at(0).checked);
            QCOMPARE(choices.at(1).name, QStringLiteral("Off"));
            QCOMPARE(choices.at(1).icon, QStringLiteral("g-devoff"));
            QCOMPARE(rail.deviceLabel(), c.label);
            QCOMPARE(rail.deviceButton()->iconName(), c.icon);
        }
        // Set by the stage: off reads Off with the slashed glyph.
        rail.setDevicePreview(16.0 / 9.0, false);
        QCOMPARE(rail.deviceLabel(), QStringLiteral("Off"));
        QCOMPARE(rail.deviceButton()->iconName(), QStringLiteral("g-devoff"));
        QVERIFY(!rail.deviceChoices().at(0).checked && rail.deviceChoices().at(1).checked);
        QCOMPARE(ui::tipName(rail.deviceButton()), QStringLiteral("Device preview: Off"));
        rail.setDevicePreview(16.0 / 9.0, true);
        QCOMPARE(ui::tipName(rail.deviceButton()), QStringLiteral("Device preview: Laptop"));

        QSignalSpy requested(&rail, &StageRail::devicePreviewRequested);
        QMenu *menu = rail.createDeviceMenu();
        const QList<QAction *> actions = menu->actions();
        QCOMPARE(actions.size(), 3);
        QCOMPARE(actions.at(0)->text(), QStringLiteral("Device preview · 16:9"));
        QCOMPARE(ui::menuRole(actions.at(0)), ui::MenuRole::Header);
        QVERIFY(actions.at(1)->isChecked() && !actions.at(2)->isChecked());
        actions.at(2)->trigger();
        QCOMPARE(requested.count(), 1);
        QCOMPARE(requested.first().first().toBool(), false);
        delete menu;

        // A click opens it, held on while it is open.
        QWidget host;
        host.resize(300, 600);
        rail.setParent(&host);
        rail.setGeometry(100, 8, StageRail::kWidth, StageRail::kHeight);
        rail.show();
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));
        QTest::mouseClick(rail.deviceButton(), Qt::LeftButton, {}, QPoint(16, 16));
        QTRY_VERIFY(qobject_cast<QMenu *>(QApplication::activePopupWidget()));
        QVERIFY(rail.deviceButton()->drawsOn());
        QApplication::activePopupWidget()->close();
        QTRY_VERIFY(!rail.deviceButton()->drawsOn());
        rail.setParent(nullptr);
    }

    // ---- the agent HUD -------------------------------------------------------

    // The HUD shows while a preview renders: "Rendering preview · v2" (the
    // proposal's version), its first change and count, the progress and
    // percent; Stop (or Esc) sends preview_stop and hides it until the next
    // render; any other status hides it. Phones get the 32-tall strip.
    void hudShowsThePreviewRender() {
        Harness h;
        const QJsonObject snapshot = proposalSnapshot();
        h.bus.setSnapshot(snapshot, snapshot.value("timeline").toObject(), 25, QSize(1920, 1080));
        h.review.setSnapshot(snapshot);
        QWidget host;
        host.resize(700, 400);
        AgentHud hud(&host, &h.bus);
        hud.setGeometry(AgentHud::cardRect(QSizeF(671.8, 377.9)).toRect());
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));
        QVERIFY(!hud.active() && hud.isHidden());
        QCOMPARE(AgentHud::cardRect(QSizeF(671.8, 377.9)), QRectF(671.8 - 34 - 8 - 288, 19 + 8, 288, 72));
        QCOMPARE(AgentHud::cardRect(QSizeF(200, 120)).width(), 200 - 20 - 16.0);

        QSignalSpy active(&hud, &AgentHud::activeChanged);
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.62}});
        QVERIFY(hud.active() && hud.isVisible());
        QCOMPARE(active.count(), 1);
        QCOMPARE(hud.text(), QStringLiteral("Rendering preview · v2"));
        QCOMPARE(hud.percentText(), QStringLiteral("62%"));
        // Without the review model's diff: the event's message, else nothing.
        QVERIFY(hud.detail().isEmpty());
        h.bus.setReview(&h.review);
        QCOMPARE(hud.detail(), QStringLiteral("Logo_reveal → 00:00:25:20 · 2 changes"));
        // The version of the proposal previewed, by any of its ids.
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc1"}, {"progress", 0.1}});
        QCOMPARE(hud.text(), QStringLiteral("Rendering preview · v1"));
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"progress", 0.3}, {"message", "Program  render"}});
        QCOMPARE(hud.text(), QStringLiteral("Rendering preview"));
        QCOMPARE(hud.detail(), QStringLiteral("Program render"));
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.62}});

        // Stop: preview_stop, hidden; the same run's progress keeps it hidden.
        h.sent.clear();
        hud.stopButton()->click();
        QCOMPARE(h.sent, (QList<QJsonObject>{QJsonObject{{"type", "preview_stop"}}}));
        QVERIFY(!hud.active() && hud.isHidden());
        QCOMPARE(active.count(), 2);
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.7}});
        QVERIFY(hud.isHidden());
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "stopped"}});
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.05}});
        QVERIFY(hud.isVisible());

        // Esc stops it too, ahead of the window's own Esc.
        h.sent.clear();
        QTest::keyClick(&host, Qt::Key_Escape);
        QCOMPARE(h.sent, (QList<QJsonObject>{QJsonObject{{"type", "preview_stop"}}}));
        QVERIFY(hud.isHidden());
        // Once hidden, Esc is the window's again.
        h.sent.clear();
        QTest::keyClick(&host, Qt::Key_Escape);
        QVERIFY(h.sent.isEmpty());

        // Ready (or any other status) hides it.
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "failed"}});
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.5}});
        QVERIFY(hud.isVisible());
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        QVERIFY(!hud.active() && hud.isHidden() && hud.text().isEmpty());

        // The phone strip: 32 tall, the bar 120 wide at the right with Stop.
        hud.setDocked(true);
        hud.setGeometry(16, 8, 400, hud.sizeHint().height());
        h.bus.dispatch(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "proposal-cc2"}, {"progress", 0.5}});
        QCOMPARE(hud.height(), AgentHud::kStripHeight);
        QWidget *bar = hud.findChild<QWidget *>(QStringLiteral("agentHudProgress"));
        QVERIFY(bar);
        QCOMPARE(bar->width(), AgentHud::kStripBar);
        QCOMPARE(hud.stopButton()->geometry().right(), 400 - 4 - 1);
        QVERIFY(bar->geometry().right() < hud.stopButton()->x());
    }

    // ---- deprecated, removed in W3 with programPeak and vuNeedle ----------------

    // The old level guess still reads the waveform peaks under the playhead
    // for its last caller (MainWindow::updateProgramLevel).
    void programLevelReadsTheSoundUnderThePlayhead() {
        MediaArtwork artwork;
        AudioPeaks peaks;
        peaks.levels.append(QVector<AudioPeaks::Bucket>(4 * AudioPeaks::kBucketsPerSecond));
        for (int index = 0; index < AudioPeaks::kBucketsPerSecond; ++index) peaks.levels[0][index] = {-0.25f, 0.2f, 0.1f};
        peaks.buildLevels();
        artwork.peaks.insert(QStringLiteral("tone"), peaks);
        const auto clip = [](const char *id, double start, double in, const QJsonObject &props = {}) {
            return QJsonObject{{"id", id}, {"sourceId", "tone"}, {"timelineStart", start}, {"duration", 2.0},
                               {"in", in}, {"out", in + 2.0}, {"props", props}};
        };
        const auto timelineOf = [](const QJsonArray &tracks) { return QJsonObject{{"tracks", tracks}}; };
        const auto track = [](const char *type, const QJsonArray &clips, const QJsonObject &extra = {}) {
            QJsonObject out{{"id", type}, {"type", type}, {"clips", clips}};
            for (auto it = extra.begin(); it != extra.end(); ++it) out.insert(it.key(), it.value());
            return out;
        };
        const QJsonObject plain = timelineOf({track("audio", {clip("a", 2.0, 0.0)})});
        QCOMPARE(programPeak(plain, &artwork, 2.5), 0.25);
        QCOMPARE(programPeak(plain, &artwork, 1.0), 0.0);
        QCOMPARE(programPeak(plain, nullptr, 2.5), 0.0);
        QCOMPARE(programPeak(timelineOf({track("audio", {clip("a", 2.0, 0.0)}, {{"muted", true}})}), &artwork, 2.5), 0.0);
        QCOMPARE(vuNeedle(0.0), 0.0);
        QVERIFY(std::abs(vuNeedle(std::pow(10.0, -18.0 / 20.0)) - 0.75) < 1e-9);
        // The VuMeter the rail still hands out is never shown.
        StageRail rail;
        QVERIFY(rail.meter() && rail.meter()->isHidden());
        rail.meter()->setTarget(2.0);
        QCOMPARE(rail.meter()->needle(), 1.0);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ScrubRailTest)

#include "scrub_rail_test.moc"
