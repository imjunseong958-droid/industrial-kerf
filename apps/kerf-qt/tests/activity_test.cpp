// Tests for what the editor shows of what it is doing: what a making's line
// says, the light round the picture and its chip as events come and go, the
// clips of media being made, queued, not made or offline, and the Media sheet's
// line under each tile.

#include "fixtures.h"
#include "test_registry.h"

#include "design_tokens.h"
#include "main_window.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "ui/activity.h"
#include "ui/agent_hud.h"
#include "ui/components.h"
#include "ui/stage_motion.h"
#include "ui/wiring/stage_wiring.h"
#include "ui/wiring/toolbar_wiring.h"
#include "viewer_widget.h"

#include <QDir>
#include <QJsonArray>
#include <QJsonObject>
#include <QListWidget>
#include <QScopeGuard>
#include <QSettings>
#include <QTest>

namespace kerf {

using namespace test;

namespace {

using State = activity::MediaActivity::State;
using Phase = activity::MakingLine::Phase;

// The editor's greys are neutral: what leans to amber is the activity's.
bool isAmberInk(const QColor &pixel) { return pixel.red() > 48 && pixel.red() > pixel.blue() + 24 && pixel.green() > pixel.blue() + 8; }

// With KERF_SCREENSHOT_DIR set, what is shown is saved, to look at.
void keep(QWidget &widget, const QString &name) {
    if (const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR"); !shots.isEmpty())
        widget.grab().save(QDir(shots).filePath(name + QStringLiteral(".png")));
}
bool isBlueInk(const QColor &pixel) { return pixel.blue() > 150 && pixel.blue() > pixel.red() + 40; }

// How many of `area`'s pixels in `image` (logical points) are amber.
int amberIn(const QImage &image, const QRect &area) {
    const qreal ratio = image.devicePixelRatio();
    int count = 0;
    for (int y = area.top(); y <= area.bottom(); ++y)
        for (int x = area.left(); x <= area.right(); ++x)
            if (isAmberInk(image.pixelColor((QPointF(x, y) * ratio).toPoint()))) ++count;
    return count;
}

QJsonObject stageSnapshot() {
    QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("width"), 1920);
    timeline.insert(QStringLiteral("height"), 1080);
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.remove(QStringLiteral("pendingProposal"));
    snapshot.remove(QStringLiteral("proposalTimeline"));
    return snapshot;
}

} // namespace

class ActivityTest final : public QObject {
    Q_OBJECT

private slots:
    // kerf-comfy's and the recipe's steps say where a making is; one small
    // reading of them says whether it waits its turn or runs.
    void makingLinesSayWhereAMakingIs() {
        activity::MakingLine line = activity::parseMakingLine(QString());
        QCOMPARE(line.phase, Phase::Starting);
        QVERIFY(!line.queued());
        line = activity::parseMakingLine(QStringLiteral("asked the server for 33 frames…: job 4f2a-77"));
        QCOMPARE(line.phase, Phase::Asked);
        QVERIFY(line.queued());
        line = activity::parseMakingLine(QStringLiteral("waiting, 1 ahead, 4 s"));
        QCOMPARE(line.phase, Phase::Waiting);
        QCOMPARE(line.ahead, 1);
        QCOMPARE(line.seconds, 4);
        QVERIFY(line.queued());
        line = activity::parseMakingLine(QStringLiteral("waiting, 9 s"));
        QCOMPARE(line.phase, Phase::Waiting);
        QCOMPARE(line.ahead, -1);
        QCOMPARE(line.seconds, 9);
        line = activity::parseMakingLine(QStringLiteral("running, 12 s"));
        QCOMPARE(line.phase, Phase::Running);
        QCOMPARE(line.seconds, 12);
        QVERIFY(!line.queued());
        line = activity::parseMakingLine(QStringLiteral("Running, 3 s"));
        QCOMPARE(line.phase, Phase::Running);
        QCOMPARE(line.seconds, 3);
        line = activity::parseMakingLine(QStringLiteral("rendering bridge: 62%"));
        QCOMPARE(line.phase, Phase::Rendering);
        QCOMPARE(line.percent, 62);
        line = activity::parseMakingLine(QStringLiteral("rendering s2: 40.5%"));
        QCOMPARE(line.percent, 40);
        line = activity::parseMakingLine(QStringLiteral("made in 13 s"));
        QCOMPARE(line.phase, Phase::Made);
        QCOMPARE(line.seconds, 13);
        line = activity::parseMakingLine(QStringLiteral("warming up"));
        QCOMPARE(line.phase, Phase::Other);
        QVERIFY(!line.queued());

        QCOMPARE(activity::elapsedText(0), QStringLiteral("0:00"));
        QCOMPARE(activity::elapsedText(-500), QStringLiteral("0:00"));
        QCOMPARE(activity::elapsedText(12'900), QStringLiteral("0:12"));
        QCOMPARE(activity::elapsedText(72'000), QStringLiteral("1:12"));
        QCOMPARE(activity::elapsedText(3'723'000), QStringLiteral("1:02:03"));

        activity::MediaActivity making{State::Making, 1'000};
        QCOMPARE(activity::clipLabel(making, 13'500), QStringLiteral("Making 0:12"));
        QCOMPARE(activity::clipLabel({State::Queued}, 0), QStringLiteral("Queued"));
        QCOMPARE(activity::clipLabel({State::NotMade}, 0), QStringLiteral("Not made"));
        QCOMPARE(activity::clipLabel({State::Offline}, 0), QStringLiteral("Offline"));
        QCOMPARE(activity::clipLabel({}, 0), QString());
    }

    // What the stage says: what the computer makes, else what Kerf does.
    void theStageSaysWhatRunsAmberOverBlue() {
        activity::Work work;
        activity::Stage stage = activity::stageFor(work, 0);
        QCOMPARE(stage.hue, activity::Hue::None);
        QVERIFY(stage.chip.isEmpty());
        work.agentBusy = true;
        stage = activity::stageFor(work, 0);
        QCOMPARE(stage.hue, activity::Hue::Kerf);
        QCOMPARE(stage.chip, QStringLiteral("Kerf is looking"));
        work.cutting = true;
        QCOMPARE(activity::stageFor(work, 0).chip, QStringLiteral("Kerf is cutting"));
        work.render = QStringLiteral("Rendering preview");
        work.fraction = 0.629;
        stage = activity::stageFor(work, 0);
        QCOMPARE(stage.hue, activity::Hue::Gpu);
        QCOMPARE(stage.chip, QStringLiteral("Rendering preview 62%"));
        work.makings.append({QStringLiteral("bridge-low"), activity::Making{1, QStringLiteral("running, 12 s"), 0}});
        stage = activity::stageFor(work, 12'000);
        QCOMPARE(stage.hue, activity::Hue::Gpu);
        QCOMPARE(stage.chip, QStringLiteral("Making bridge-low · 0:12"));
        // The one running is named ahead of one waiting its turn, and the rest counted.
        work.makings.prepend({QStringLiteral("harbor"), activity::Making{2, QStringLiteral("waiting, 2 ahead, 3 s"), 0}});
        QCOMPARE(activity::stageFor(work, 12'000).chip, QStringLiteral("Making bridge-low · 0:12 +1"));
        work.makings.removeLast();
        QCOMPARE(activity::stageFor(work, 12'000).chip, QStringLiteral("Making harbor · queued, 2 ahead"));
        QCOMPARE(activity::hueColor(activity::Hue::Kerf), tokens::activity::kerf);
        QCOMPARE(activity::hueColor(activity::Hue::Gpu), tokens::activity::gpu);
    }

    // The light round the picture: just outside its edge, in its hue; under
    // Reduce Motion a still ring that draws no frames, else a running light
    // that comes and goes and then stops.
    void theEdgeLightComesAndGoes() {
        activity::EdgeLight light;
        light.resize(260, 170);
        light.setFrame(QRectF(30, 30, 200, 112), tokens::layout::pictureRadius);
        light.show();
        QVERIFY(QTest::qWaitForWindowExposed(&light));
        const QPoint onRing(130, 30 - 2);
        const auto at = [&light](const QPoint &point) {
            const QImage image = light.grab().toImage();
            return image.pixelColor((QPointF(point) * image.devicePixelRatio()).toPoint());
        };
        const QColor unlit = at(onRing);
        QVERIFY(!light.isLit());
        light.setHue(activity::Hue::Gpu);
        QCOMPARE(light.level(), 1.0);
        QVERIFY(light.isLit());
        QVERIFY(!light.isTicking());
        QVERIFY2(isAmberInk(at(onRing)), qPrintable(at(onRing).name()));
        // Not over the picture: its colours are its own.
        QCOMPARE(at(QPoint(130, 40)), unlit);
        light.setHue(activity::Hue::Kerf);
        QVERIFY2(isBlueInk(at(onRing)), qPrintable(at(onRing).name()));
        // Only the ring is drawn again for it.
        QVERIFY(!light.band().contains(QPoint(130, 86)));
        QVERIFY(light.band().contains(onRing));
        light.setHue(activity::Hue::None);
        QVERIFY(!light.isLit());
        QCOMPARE(at(onRing), unlit);

        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        light.setHue(activity::Hue::Gpu);
        QVERIFY(light.isTicking());
        QVERIFY(light.level() < 1.0);
        QTRY_COMPARE_WITH_TIMEOUT(light.level(), 1.0, 2000);
        // Running, it marks where the light was and is, not the ring.
        QTRY_VERIFY_WITH_TIMEOUT(!light.m_lit.isEmpty(), 1000);
        keep(light, QStringLiteral("activity-edge"));
        QVERIFY(light.m_lit.boundingRect().width() * light.m_lit.boundingRect().height() < light.band().boundingRect().width() * 40);
        light.hide();
        QVERIFY(!light.isTicking());
        light.show();
        QVERIFY(light.isTicking());
        light.setHue(activity::Hue::None);
        QVERIFY(light.isLit());
        QTRY_VERIFY_WITH_TIMEOUT(!light.isLit(), 2000);
        QTRY_VERIFY_WITH_TIMEOUT(!light.isTicking(), 1000);
    }

    // As Kerf works, a model makes a shot, or a render runs, the light round the
    // picture and its chip say so, in blue or amber, and both go when it ends.
    void theStageShowsWhatRunsAndLetsItGo() {
        QSettings().remove(QStringLiteral("layout"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(stageSnapshot());
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        activity::EdgeLight *light = window.m_edgeLight;
        WorkChip *chip = window.m_workChip;
        QVERIFY(light && chip);
        QCOMPARE(light->parentWidget(), window.m_viewer);
        QCOMPARE(light->frame(), window.m_viewer->videoRect());
        QCOMPARE(light->hue(), activity::Hue::None);
        QVERIFY(!chip->isShown());

        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QCOMPARE(light->hue(), activity::Hue::Kerf);
        QVERIFY(chip->isShown());
        QCOMPARE(chip->text(), QStringLiteral("Kerf is looking"));
        keep(window, QStringLiteral("activity-kerf"));
        QCOMPARE(chip->dot(), tokens::activity::kerf);
        // At the picture's top left.
        const QRect picture = window.m_viewer->videoRect().toRect();
        QCOMPARE(chip->pos(), picture.topLeft() + QPoint(ViewerWidget::kChipInset, ViewerWidget::kChipInset));

        // A model makes a shot: amber, with its name and its clock.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "drone"}, {"take", 1}, {"line", ""}});
        QCOMPARE(light->hue(), activity::Hue::Gpu);
        QCOMPARE(chip->dot(), tokens::activity::gpu);
        QCOMPARE(chip->text(), QStringLiteral("Making drone-pass · 0:00"));
        QVERIFY(window.m_activityClock.isActive());
        window.m_makingState[QStringLiteral("drone")].since = activity::clock() - 12'000;
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "drone"}, {"take", 1}, {"line", "running, 12 s"}});
        QCOMPARE(chip->text(), QStringLiteral("Making drone-pass · 0:12"));
        // Its clip and its tile say so too.
        QCOMPARE(window.m_timeline->clipActivity(QStringLiteral("v1c")).state, State::Making);
        QCOMPARE(window.m_timeline->clipActivityLabel(QStringLiteral("v1c")), QStringLiteral("Making 0:12"));
        QCOMPARE(window.m_projectPanel->status(QStringLiteral("drone")), QStringLiteral("Making take 1 · 0:12"));
        keep(window, QStringLiteral("activity-making"));
        // Waiting its turn on the server, it is queued.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "drone"}, {"take", 1}, {"line", "waiting, 2 ahead, 3 s"}});
        QCOMPARE(window.m_timeline->clipActivityLabel(QStringLiteral("v1c")), QStringLiteral("Queued"));
        QCOMPARE(chip->text(), QStringLiteral("Making drone-pass · queued, 2 ahead"));
        QCOMPARE(window.m_projectPanel->status(QStringLiteral("drone")), QStringLiteral("Queued · 2 ahead"));
        // A new take starts its clock again.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "drone"}, {"take", 2}, {"line", "running, 1 s"}});
        QVERIFY(activity::clock() - window.m_makingState.value(QStringLiteral("drone")).since < 1000);
        // Made, Kerf's own light is back, and the clock stops.
        window.handleEvent(QJsonObject{{"type", "media_made"}, {"media", "drone"}, {"take", 2}});
        QCOMPARE(light->hue(), activity::Hue::Kerf);
        QCOMPARE(chip->text(), QStringLiteral("Kerf is looking"));
        QVERIFY(!window.m_activityClock.isActive());
        QCOMPARE(window.m_timeline->clipActivityLabel(QStringLiteral("v1c")), QString());
        // A making that fails is gone from the stage too.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "drone"}, {"take", 3}, {"line", ""}});
        QCOMPARE(light->hue(), activity::Hue::Gpu);
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", "cannot make drone: it failed"}, {"command", "media_make"}, {"media", "drone"}});
        QCOMPARE(light->hue(), activity::Hue::Kerf);

        // A render: amber while it runs.
        window.handleEvent(QJsonObject{{"type", "export"}, {"kind", "render"}, {"jobId", "j1"}, {"quality", "master"},
                                       {"status", "running"}, {"progress", 0.4}});
        double fraction = 0;
        QCOMPARE(window.m_toolbarWiring->runningRender(&fraction), QStringLiteral("Exporting master"));
        QCOMPARE(fraction, 0.4);
        QCOMPARE(light->hue(), activity::Hue::Gpu);
        QCOMPARE(chip->text(), QStringLiteral("Exporting master 40%"));
        window.handleEvent(QJsonObject{{"type", "export"}, {"kind", "render"}, {"jobId", "j1"}, {"status", "done"}});
        QCOMPARE(light->hue(), activity::Hue::Kerf);
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}, {"progress", 0.62}});
        QCOMPARE(window.m_toolbarWiring->runningRender(&fraction), QStringLiteral("Rendering preview"));
        QCOMPARE(light->hue(), activity::Hue::Gpu);
        // The preview's own card says what it renders; else the chip does.
        if (window.m_stageWiring->hud()->isVisible()) QVERIFY(!chip->isShown());
        else QCOMPARE(chip->text(), QStringLiteral("Rendering preview 62%"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "done"}});

        // Work ends: the light and the chip go (at once, under Reduce Motion).
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QCOMPARE(light->hue(), activity::Hue::None);
        QVERIFY(!light->isLit());
        QVERIFY(!chip->isShown());
        QVERIFY(chip->isHidden());

        // A file that is gone shows on its clip and its tile.
        window.handleEvent(QJsonObject{{"type", "media_status"}, {"missing", QJsonArray{"drone"}}, {"pointers", QJsonArray{}},
                                       {"external", QJsonArray{}}});
        QCOMPARE(window.m_timeline->clipActivityLabel(QStringLiteral("v1c")), QStringLiteral("Offline"));
        QCOMPARE(window.m_projectPanel->status(QStringLiteral("drone")), QStringLiteral("Offline · file missing"));
        QCOMPARE(light->hue(), activity::Hue::None);
        QSettings().remove(QStringLiteral("layout"));
        window.close();
    }

    // A clip of media being made has amber stripes and its clock; queued, a
    // dashed amber outline; with no file, a dashed hole saying why. Only those
    // being made draw frames, and only while there are some.
    void clipsShowTheirMediasMaking() {
        TimelineWidget timeline;
        timeline.resize(900, 240);
        const auto clip = [](const char *id, const char *source, double start) {
            return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", source}, {"timelineStart", start}, {"duration", 3.0}};
        };
        timeline.setProject(QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                                            clip("a", "m1", 0.0), clip("b", "m2", 3.0), clip("c", "m3", 6.0), clip("d", "m4", 9.0),
                                            clip("e", "m5", 12.0)}}}}}},
                            16, 24);
        timeline.show();
        QVERIFY(QTest::qWaitForWindowExposed(&timeline));
        QVERIFY(!timeline.makingTicks());
        const qint64 now = activity::clock();
        timeline.setMediaActivity({{QStringLiteral("m1"), {State::Making, now - 12'000, 1}},
                                   {QStringLiteral("m2"), {State::Queued, now, 1}},
                                   {QStringLiteral("m3"), {State::NotMade}},
                                   {QStringLiteral("m4"), {State::Offline}}});
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("a")), QStringLiteral("Making 0:12"));
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("b")), QStringLiteral("Queued"));
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("c")), QStringLiteral("Not made"));
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("d")), QStringLiteral("Offline"));
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("e")), QString());
        QVERIFY(timeline.makingTicks());
        // Under Reduce Motion the stripes hold still: only the clock goes on.
        QCOMPARE(timeline.m_makingTicker->interval(), tokens::motion::clockFrame);
        QCOMPARE(timeline.makingClipRects().size(), 1);
        QCOMPARE(timeline.makingClipRects().constFirst().toAlignedRect(), timeline.clipBounds(QStringLiteral("a")));
        keep(timeline, QStringLiteral("activity-timeline"));
        const QImage image = timeline.grab().toImage();
        const auto body = [&timeline](const char *id) { return timeline.clipBounds(QString::fromLatin1(id)).adjusted(2, 18, -2, -2); };
        const auto edge = [&timeline](const char *id) {
            const QRect bounds = timeline.clipBounds(QString::fromLatin1(id));
            return QRect(bounds.left() + 6, bounds.bottom(), bounds.width() - 12, 1);
        };
        QVERIFY2(amberIn(image, body("a")) > 40, qPrintable(QString::number(amberIn(image, body("a")))));
        QVERIFY(amberIn(image, edge("b")) > 4);
        QCOMPARE(amberIn(image, body("c")), 0);
        QCOMPARE(amberIn(image, body("e")), 0);
        // With no file, a hole in the lane: the well, not a clip's card.
        const qreal ratio = image.devicePixelRatio();
        const QRect hole = body("c");
        QCOMPARE(image.pixelColor((QPointF(hole.right() - 4, hole.bottom() - 2) * ratio).toPoint()).rgb(), tokens::well);
        timeline.setMediaActivity({});
        QVERIFY(!timeline.makingTicks());
        QCOMPARE(timeline.clipActivityLabel(QStringLiteral("a")), QString());
        QCOMPARE(amberIn(timeline.grab().toImage(), body("a")), 0);
        // Hidden, nothing ticks.
        timeline.setMediaActivity({{QStringLiteral("m1"), {State::Making, now, 1}}});
        QVERIFY(timeline.makingTicks());
        timeline.hide();
        QVERIFY(!timeline.makingTicks());
    }

    // Under each tile its whole name and what it is doing.
    void mediaTilesSayWhatTheyAreDoing() {
        const QJsonObject shot{{"id", "m5"}, {"path", "shots/boat.takes/002.mp4"}, {"width", 320}, {"duration", 4}, {"recipe", "shots/boat.kerfrecipe"},
                               {"takes", QJsonArray{QJsonObject{{"number", 1}, {"path", "shots/boat.takes/001.mp4"}, {"chosen", false}},
                                                    QJsonObject{{"number", 2}, {"path", "shots/boat.takes/002.mp4"}, {"chosen", true}}}}};
        const QJsonObject fresh{{"id", "m6"}, {"path", "shots/dawn.mp4"}, {"width", 320}, {"duration", 4}, {"recipe", "shots/dawn.kerfrecipe"}};
        const QJsonObject video{{"id", "m1"}, {"path", "media/a.mov"}, {"width", 1920}, {"duration", 4}};
        const QJsonObject sound{{"id", "m2"}, {"path", "media/voice.wav"}, {"duration", 4}, {"hasAudio", true}};
        const auto text = [](const QJsonObject &source, const activity::MediaActivity &doing = {}, const QString &badge = {},
                             const QString &wants = {}, const QStringList &on = {}) {
            return ProjectPanel::statusText(source, doing, badge, wants, on, 41'000);
        };
        QCOMPARE(text(shot, {}, {}, {}, {"V1"}), QStringLiteral("Made · shot · take 2 of 2 · on V1"));
        QCOMPARE(text(fresh), QStringLiteral("Made · shot · take 1 of 1"));
        QCOMPARE(text(video, {}, {}, {}, {"V1", "V2"}), QStringLiteral("Video · on V1 V2"));
        QCOMPARE(text(sound), QStringLiteral("Sound"));
        QCOMPARE(text(video, {}, QStringLiteral("External"), {}, {"V1"}), QStringLiteral("Video · outside the project · on V1"));
        QCOMPARE(text(video, {}, QStringLiteral("Offline")), QStringLiteral("Offline · file missing"));
        QCOMPARE(text(fresh, {}, QStringLiteral("Offline")), QStringLiteral("Not made yet"));
        QCOMPARE(text(fresh, {State::NotMade}), QStringLiteral("Not made yet"));
        QCOMPARE(text(shot, {}, {}, QStringLiteral("Ada's words changed")), QStringLiteral("Wants making · Ada's words changed"));
        QCOMPARE(text(shot, {State::Making, 0, 3}), QStringLiteral("Making take 3 · 0:41"));
        QCOMPARE(text(shot, {State::Queued, 0, 3, 1}), QStringLiteral("Queued · 1 ahead"));
        QCOMPARE(text(shot, {State::Queued, 0, 3}), QStringLiteral("Queued for making"));
        QCOMPARE(text(video, {State::Offline}), QStringLiteral("Offline · file missing"));

        ProjectPanel bin(nullptr);
        bin.setSources(QJsonArray{video, sound, shot, fresh});
        bin.setTracks(QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{
                                     QJsonObject{{"id", "c1"}, {"sourceId", "m5"}}, QJsonObject{{"id", "c2"}, {"sourceId", "m1"}}}}},
                                 QJsonObject{{"id", "V2"}, {"type", "video"}, {"clips", QJsonArray{QJsonObject{{"id", "c3"}, {"sourceId", "m1"}}}}}});
        QCOMPARE(bin.status(QStringLiteral("m5")), QStringLiteral("Made · shot · take 2 of 2 · on V1"));
        QCOMPARE(bin.status(QStringLiteral("m1")), QStringLiteral("Video · on V1 V2"));
        bin.setMediaStatus(QJsonObject{{"missing", QJsonArray{"m6"}}});
        QCOMPARE(bin.status(QStringLiteral("m6")), QStringLiteral("Not made yet"));
        // A shot not made is a dashed hole on the well, saying so.
        const QImage hole = bin.m_list->item(3)->icon().pixmap(ProjectPanel::kThumbnail).toImage();
        QCOMPARE(hole.pixelColor(QPoint(10, 50) * hole.devicePixelRatio()).rgb(), tokens::well);
        bin.setWantsMaking({{QStringLiteral("m5"), QStringLiteral("Ada's words changed")}});
        QCOMPARE(bin.status(QStringLiteral("m5")), QStringLiteral("Wants making · Ada's words changed"));
        bin.setWantsMaking({});

        bin.resize(tokens::layout::bin - 1, 520);
        bin.show();
        QVERIFY(QTest::qWaitForWindowExposed(&bin));
        QVERIFY(!bin.makingTicks());
        bin.setMediaActivity({{QStringLiteral("m5"), {State::Making, activity::clock() - 41'000, 3}}});
        QCOMPARE(bin.status(QStringLiteral("m5")), QStringLiteral("Making take 3 · 0:41"));
        QVERIFY(bin.makingTicks());
        // Stripes over its thumb, none over the others'.
        const auto thumb = [&bin](int row) {
            const QRect cell = bin.m_list->visualItemRect(bin.m_list->item(row));
            return QRect(bin.m_list->viewport()->mapTo(&bin, cell.topLeft()), ProjectPanel::kThumbnail).adjusted(2, 2, -40, -2);
        };
        QCoreApplication::processEvents();
        keep(bin, QStringLiteral("activity-media"));
        const QImage tiles = bin.grab().toImage();
        QVERIFY2(amberIn(tiles, thumb(2)) > 40, qPrintable(QString::number(amberIn(tiles, thumb(2)))));
        QCOMPARE(amberIn(tiles, thumb(0)), 0);
        // The list has no stripes to move.
        bin.setListView(true);
        QVERIFY(!bin.makingTicks());
        bin.setListView(false);
        QVERIFY(bin.makingTicks());
        bin.setMediaActivity({});
        QVERIFY(!bin.makingTicks());
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ActivityTest)

#include "activity_test.moc"
