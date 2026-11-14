// Motion: what each scene costs a frame (a measurement, run on request), and what
// is left when motion is cut short.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "main_window.h"
#include "proposal_card.h"
#include "ui/wiring/review_wiring.h"
#include "timeline_widget.h"
#include "ui/components.h"
#include "ui/stage_motion.h"
#include "ui/stage_shell.h"
#include "gpu_picture.h"
#include "viewer_widget.h"

#include <QElapsedTimer>
#include <QEventLoop>
#include <QGraphicsOpacityEffect>
#include <QGuiApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QPainter>
#include <QScopeGuard>
#include <QScreen>
#include <QSignalSpy>
#include <QTimer>
#include <QVBoxLayout>
#include <QTest>

#include <algorithm>
#include <cstdio>
#include <ctime>
#include <functional>

namespace kerf {

using namespace test;

namespace {

QJsonObject snapshotOf(const QSize &canvas) {
    QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("width"), canvas.width());
    timeline.insert(QStringLiteral("height"), canvas.height());
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.remove(QStringLiteral("pendingProposal"));
    snapshot.remove(QStringLiteral("proposalTimeline"));
    return snapshot;
}

// A busy picture, as footage is, in the format the bridge sends.
QImage footageFrame(const QSize &size, int seed = 0) {
    QImage image(size, QImage::Format_RGBA8888);
    image.fill(Qt::black);
    QPainter p(&image);
    QLinearGradient sky(0, 0, size.width(), size.height());
    sky.setColorAt(0.0, QColor::fromHsv((seed * 37) % 360, 160, 200));
    sky.setColorAt(1.0, QColor::fromHsv((seed * 37 + 80) % 360, 200, 90));
    p.fillRect(image.rect(), sky);
    for (int i = 0; i < 60; ++i) p.fillRect((i * 97 + seed) % size.width(), (i * 53) % size.height(), 90, 60, QColor::fromHsv((i * 29) % 360, 120, 180, 120));
    return image;
}

// What the main thread did over a stretch: its time spent working, and each
// frame's repaint.
class Meter final : public QObject {
public:
    explicit Meter(QWidget *window) : m_window(window) {
        window->installEventFilter(this);
        m_clock.start();
        m_cpu = std::clock();
    }
    void report(const char *scene) {
        const double total = m_clock.nsecsElapsed() / 1e6;
        std::sort(m_paints.begin(), m_paints.end());
        double worstGap = 0.0;
        for (int i = 1; i < m_starts.size(); ++i) worstGap = std::max(worstGap, m_starts.at(i) - m_starts.at(i - 1));
        const auto at = [this](double share) { return m_paints.isEmpty() ? 0.0 : m_paints.at(std::min<qsizetype>(m_paints.size() - 1, qsizetype(m_paints.size() * share))); };
        std::printf("%-28s cpu %5.1f%%  frames %4d (%5.1f/s)  paint p50 %6.2f p95 %6.2f ms  worst gap %6.1f ms\n", scene, 100.0 * (double(std::clock() - m_cpu) / CLOCKS_PER_SEC * 1000.0) / total,
                    int(m_paints.size()), m_paints.size() / (total / 1000.0), at(0.5), at(0.95), worstGap);
        std::fflush(stdout);
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        if (watched != m_window || event->type() != QEvent::UpdateRequest || m_inside) return false;
        m_inside = true;
        QElapsedTimer paint;
        paint.start();
        m_starts << m_clock.nsecsElapsed() / 1e6;
        QCoreApplication::sendEvent(m_window, event);
        m_paints << paint.nsecsElapsed() / 1e6;
        m_inside = false;
        return true;
    }

private:
    QWidget *m_window;
    QElapsedTimer m_clock;
    std::clock_t m_cpu = 0;
    bool m_inside = false;
    QList<double> m_paints;
    QList<double> m_starts;
};

} // namespace

class MotionTest final : public QObject {
    Q_OBJECT

private slots:
    // Playing, a frame draws again only the picture: the room around it, and what
    // is under the picture, are not drawn, and the screen is as a whole drawing
    // of the window would make it.
    void playingDrawsOnlyThePicture() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        window.m_shell->setPanes(open);
        QTest::qWait(200);
        // The room fills all it repaints: what is under it is not drawn first.
        QVERIFY(window.m_shell->testAttribute(Qt::WA_OpaquePaintEvent));
        const QRect shown = window.m_viewer->shownRect();
        QVERIFY(!shown.isEmpty() && shown != window.m_viewer->rect());
        QVERIFY(window.m_viewer->rect().contains(window.m_viewer->coveredRegion().boundingRect()));
        for (int i = 1; i <= 6; ++i) {
            window.m_viewer->setFrame(footageFrame(QSize(1920, 1080), i), i / 24.0);
            QCoreApplication::processEvents();
        }
        QTest::qWait(50);
        // What is on the screen, read back from it, against the window drawn whole
        // afresh: the second is the same only at one pixel a point.
        if (window.devicePixelRatio() != 1.0) QSKIP("The offscreen screen is read back at one pixel a point");
        QCOMPARE(window.screen()->grabWindow(window.winId()).toImage().convertToFormat(QImage::Format_RGB32),
                 window.grab().toImage().convertToFormat(QImage::Format_RGB32));
    }

    // The GPU draws the picture only where nothing else does: a frame at Fit,
    // between its rounded top and bottom, with nothing compared or wiped. The
    // rest goes back to the CPU as it changes.
    void theGpuDrawsOnlyAFrameAtFitBetweenItsRoundedEdges() {
#ifdef KERF_GPU_PICTURE
        // Never shown, so never drawn: whether this machine has a GPU is not asked.
        GpuPicture::s_availableForTests = 1;
        const auto restore = qScopeGuard([] { GpuPicture::s_availableForTests = -1; });
        ViewerWidget viewer;
        viewer.resize(960, 540);
        viewer.setGpuPicture(true);
        QVERIFY(viewer.gpuPicture());
        QVERIFY(viewer.gpuRect().isEmpty());
        const QImage frame = footageFrame(QSize(1920, 1080), 2);
        viewer.setFrame(frame, 0.0);
        const QRect area = viewer.gpuRect();
        const QRectF card = viewer.videoRect();
        const int round = tokens::layout::pictureRadius;
        QVERIFY(!area.isEmpty());
        QVERIFY(card.contains(QRectF(area)));
        QVERIFY(area.top() >= card.top() + round && area.bottom() < card.bottom() - round);
        QVERIFY(area.width() >= card.width() - 2);
        // It shows the part of the frame it covers.
        auto *gpu = static_cast<GpuPicture *>(viewer.m_gpu);
        QCOMPARE(gpu->geometry(), area);
        QVERIFY(qAbs(gpu->source().top() - (area.top() - card.top()) / card.height()) < 1e-9);
        QVERIFY(qAbs(gpu->source().height() - area.height() / card.height()) < 1e-9);
        viewer.setCompareFrame(frame);
        QVERIFY(viewer.gpuRect().isEmpty());
        QVERIFY(gpu->isHidden());
        viewer.setCompareFrame(QImage());
        QCOMPARE(viewer.gpuRect(), area);
        viewer.setZoom(1.0);
        QVERIFY(viewer.gpuRect().isEmpty());
        viewer.setZoom(0.0);
        QCOMPARE(viewer.gpuRect(), area);
        // Hidden, it has its new size only when shown, and is told of it then.
        viewer.resize(800, 600);
        QResizeEvent resized(viewer.size(), QSize(960, 540));
        QCoreApplication::sendEvent(&viewer, &resized);
        QVERIFY(viewer.gpuRect() != area && !viewer.gpuRect().isEmpty());
        viewer.clearFrame();
        QVERIFY(viewer.gpuRect().isEmpty());
        viewer.setFrame(frame, 0.0);
        viewer.setGpuPicture(false);
        QVERIFY(!viewer.gpuPicture());
        QVERIFY(viewer.gpuRect().isEmpty());
#else
        QSKIP("The GPU picture needs Qt 6.7");
#endif
    }

    // Where no GPU starts, as on the offscreen platform, none is asked to draw:
    // the window stays as it was, and the viewer draws all of the picture.
    void aPictureNoGpuCanDrawIsDrawnHere() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.m_viewer->setGpuPicture(true);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        window.m_shell->setPanes(open);
        for (int i = 1; i <= 3; ++i) {
            window.m_viewer->setFrame(footageFrame(QSize(1920, 1080), i), i / 24.0);
            QCoreApplication::processEvents();
        }
#ifdef KERF_GPU_PICTURE
        // Where one does start, it draws.
        if (GpuPicture::available()) {
            QVERIFY(window.m_viewer->gpuPicture());
            QTRY_VERIFY_WITH_TIMEOUT(static_cast<GpuPicture *>(window.m_viewer->m_gpu)->uploads() > 0, 3000);
            return;
        }
#endif
        QVERIFY(!window.m_viewer->gpuPicture());
        QVERIFY(window.m_viewer->gpuRect().isEmpty());
        QTest::qWait(50);
        if (window.devicePixelRatio() != 1.0) return;
        QCOMPARE(window.screen()->grabWindow(window.winId()).toImage().convertToFormat(QImage::Format_RGB32),
                 window.grab().toImage().convertToFormat(QImage::Format_RGB32));
    }

    // KERF_MOTION_COSTS=1 QT_SCALE_FACTOR=2 kerf-qt-tests -platform offscreen whatEachSceneCosts
    void whatEachSceneCosts() {
        if (qEnvironmentVariableIsEmpty("KERF_MOTION_COSTS")) QSKIP("A measurement, run on request");
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        for (int i = 0; i < 12; ++i) {
            window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"},
                                           {"text", QStringLiteral("Trimmed the intro and tightened the second interview; the music now ducks under the voice and the title lands on the beat. Round %1.").arg(i)}});
        }
        const QImage frame = footageFrame(QSize(1920, 1080));
        window.m_viewer->setFrame(frame, 0.0);
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QTest::qWait(300);
        const auto run = [&](const char *scene, int ms, const std::function<void()> &start) {
            QTest::qWait(150);
            Meter meter(&window);
            start();
            // Not qWait: its sleeps would show as gaps between frames.
            QEventLoop loop;
            QTimer::singleShot(ms, &loop, &QEventLoop::quit);
            loop.exec();
            meter.report(scene);
        };
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        stage::Panes closed = open;
        closed.picture = false;
        closed.timeline = false;
        window.m_shell->setPanes(open);
        QTest::qWait(1000);
        run("idle", 500, [] {});
        const QRect picture = window.m_shell->rects().picture;
        run("stage: panel closes", 850, [&] { window.m_shell->setPanes(closed, QRect(20, 20, 44, 24)); });
        run("stage: panel opens", 850, [&] { window.m_shell->setPanes(open, QRect(300, 600, 128, 72)); });
        QTest::qWait(300);
        run("flights: three tiles", 1300, [&] {
            for (int i = 0; i < 3; ++i) {
                StageMotion::Flight flight;
                flight.image = frame;
                flight.from = QRectF(320 + i * 40, 700, 128, 72);
                flight.to = QRectF(700 + i * 200, 640, 180, 44);
                flight.delay = i * tokens::motion::flightStagger;
                flight.duration = tokens::motion::flight;
                flight.tail = tokens::motion::flightLand;
                flight.ring = true;
                flight.label = QStringLiteral("Interview");
                window.m_motion->fly(flight);
            }
        });
        run("cover: fades in, then holds", 1500, [&] {
            StageMotion::Flight cover;
            cover.image = frame;
            cover.from = picture;
            cover.to = picture;
            cover.fromOpacity = 0.0;
            cover.duration = tokens::motion::appear;
            cover.lively = false;
            cover.stays = true;
            cover.fit = true;
            cover.radius = tokens::layout::pictureRadius;
            window.m_motion->fly(cover);
        });
        window.m_motion->releaseStaying();
        QTest::qWait(400);
        run("work chip", 1500, [&] { window.m_workChip->setVisible(true); });
        window.m_workChip->setVisible(false);
        run("before/after wipe", 1500, [&] { window.m_viewer->setCompareFrame(footageFrame(QSize(960, 540), 3)); });
        window.m_viewer->setCompareFrame(QImage());
        // A first cut at work, as its card in the conversation: the ring, a clip
        // turning, and the finish.
        window.m_shell->setPanes(closed);
        QTest::qWait(1000);
        window.m_chat->setBuildingProposal(QStringLiteral("p7"));
        QTest::qWait(400);
        if (ProposalCard *card = window.m_chat->buildingCard()) {
            CutPicture cut;
            cut.duration = 18.0;
            cut.frames << CutPicture::Frame{0.0, frame} << CutPicture::Frame{6.0, footageFrame(QSize(1920, 1080), 3)};
            card->setPicture(cut);
            QTest::qWait(800);
            run("card at work: the ring", 1500, [] {});
            run("card at work: a clip turns", 800, [&] {
                cut.frames << CutPicture::Frame{12.0, footageFrame(QSize(1920, 1080), 5)};
                card->setPicture(cut);
            });
            run("card: done", 2600, [&] {
                card->picture()->setWorking(false);
                card->picture()->setFinished(0);
            });
            run("card: done, at rest", 500, [] {});
        }
        window.m_chat->setBuildingProposal(QString());
        window.m_shell->setPanes(open);
        QTest::qWait(1000);
        run("playback: 24 fps of frames", 1500, [&] {
            auto *timer = new QTimer(&window);
            int seed = 1;
            connect(timer, &QTimer::timeout, &window, [&window, seed, timer]() mutable {
                static const QList<QImage> frames = [] {
                    QList<QImage> out;
                    // One shot: the picture moves about and its colour hardly does.
                    for (int i = 0; i < 8; ++i) {
                        QImage shot = footageFrame(QSize(1920, 1080));
                        QPainter(&shot).fillRect((i * 211) % 1600, (i * 131) % 800, 300, 200, QColor(255, 255, 255, 40));
                        out << shot;
                    }
                    return out;
                }();
                ++seed;
                window.m_viewer->setFrame(frames.at(seed % 8), seed / 24.0);
                if (seed > 36) timer->deleteLater();
            });
            timer->start(41);
        });
        QTest::qWait(1000);
        window.close();
    }

    // What the window's own parts cost as a conversation grows: the conversation
    // made again, a snapshot taken in, one comment arriving, and the timeline drawn.
    void whatAConversationCosts() {
        if (qEnvironmentVariableIsEmpty("KERF_MOTION_COSTS")) QSKIP("A measurement, run on request");
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = snapshotOf(QSize(1920, 1080));
        window.applySnapshot(snapshot);
        window.m_reviewWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", snapshot}});
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const auto settle = [] { for (int i = 0; i < 4; ++i) QCoreApplication::processEvents(); };
        const auto ms = [](const std::function<void()> &work, int times) {
            QList<double> took;
            for (int i = 0; i < times; ++i) {
                QElapsedTimer clock;
                clock.start();
                work();
                took << clock.nsecsElapsed() / 1e6;
            }
            std::sort(took.begin(), took.end());
            return took.at(took.size() / 2);
        };
        const QJsonObject you{{"kind", "person"}, {"name", "Ana Ruiz"}, {"id", "ana@kerf.test"}};
        const QJsonObject kerf{{"kind", "kerf"}, {"name", "Kerf"}};
        const auto record = [](const QString &id, const QString &parent, const QJsonObject &author, int minute, const QString &body) {
            return QJsonObject{{"id", id}, {"parent", parent.isEmpty() ? QJsonValue() : QJsonValue(parent)}, {"author", author}, {"body", body},
                               {"tags", QJsonArray{}}, {"blocking", false}, {"resolved", false},
                               {"created", QStringLiteral("2026-11-14T%1:%2:00Z").arg(8 + minute / 60, 2, 10, QLatin1Char('0')).arg(minute % 60, 2, 10, QLatin1Char('0'))},
                               {"revision", 22}};
        };
        for (const int turns : {10, 50, 150}) {
            QJsonArray items;
            for (int i = 0; i < turns; ++i) {
                items << record(QStringLiteral("c%1").arg(i), QString(), you, i * 2, QStringLiteral("Tighten the interview at the harbor and let the music breathe, round %1").arg(i));
                items << record(QStringLiteral("c%1r").arg(i), QStringLiteral("c%1").arg(i), kerf, i * 2 + 1,
                                QStringLiteral("Trimmed the intro and tightened the second interview; the music now ducks under the voice and the title lands on the beat. Round %1.").arg(i));
            }
            window.m_reviewWiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", items}});
            settle();
            const double rebuild = ms([&] { window.m_chat->rebuild(); }, 5);
            QJsonArray more = items;
            more << record(QStringLiteral("new"), QString(), you, turns * 2 + 1, QStringLiteral("One more note"));
            const double arrives = ms([&] {
                static int flip = 0;
                window.m_reviewWiring->handleEvent(QStringLiteral("comments"), QJsonObject{{"type", "comments"}, {"items", (flip++ % 2) ? items : more}});
                settle();
            }, 6);
            const double takesIn = ms([&] {
                static int revision = 40;
                QJsonObject changed = snapshot;
                changed.insert(QStringLiteral("revision"), ++revision);
                QJsonObject proposal = changed.value(QStringLiteral("pendingProposal")).toObject();
                proposal.insert(QStringLiteral("script"), QStringLiteral("trim v1b out -%1f\n").arg(revision));
                changed.insert(QStringLiteral("pendingProposal"), proposal);
                QJsonArray proposals = changed.value(QStringLiteral("proposals")).toArray();
                QJsonObject listed = proposals.at(0).toObject();
                listed.insert(QStringLiteral("script"), proposal.value(QStringLiteral("script")));
                proposals.replace(0, listed);
                changed.insert(QStringLiteral("proposals"), proposals);
                window.applySnapshot(changed);
                window.m_reviewWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", changed}});
                settle();
            }, 6);
            const double same = ms([&] { window.applySnapshot(snapshot); settle(); }, 6);
            std::printf("%3d turns: conversation made again %6.1f ms   a comment arrives %6.1f ms   a changed snapshot %6.1f ms   the same snapshot %5.1f ms   rows %d\n",
                        turns, rebuild, arrives, takesIn, same, int(window.m_chat->rows().size()));
            std::fflush(stdout);
        }
        stage::Panes open = window.m_shell->panes();
        open.picture = open.timeline = true;
        window.m_shell->setPanes(open);
        QTest::qWait(1200);
        const double timeline = ms([&] { window.m_timeline->repaint(); }, 9);
        const double viewer = ms([&] { window.m_viewer->repaint(); }, 9);
        const double chat = ms([&] { window.m_chat->repaint(); }, 9);
        const double whole = ms([&] { window.repaint(); }, 5);
        std::printf("drawn whole: timeline %.2f ms   picture %.2f ms   conversation %.2f ms   window %.2f ms\n", timeline, viewer, chat, whole);
        std::fflush(stdout);
        window.close();
    }

    // What is marked to be drawn is all that changed: nothing is left behind on the
    // way, or when it is over.
    void motionLeavesNothingBehind() {
        if (QGuiApplication::platformName() != QLatin1String("offscreen")) QSKIP("Reads the offscreen platform's backing store");
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1400, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const QImage frame = footageFrame(QSize(1920, 1080));
        window.m_viewer->setFrame(frame, 0.0);
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        window.m_shell->setPanes(open);
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        QTest::qWait(tokens::motion::stage);
        const auto differing = [&window] {
            // What is on screen against all of it drawn anew.
            QTest::qWait(40);
            const QImage shown = window.screen()->grabWindow(window.winId()).toImage().convertToFormat(QImage::Format_RGB32);
            const QImage drawn = window.grab().toImage().convertToFormat(QImage::Format_RGB32);
            if (shown.size() != drawn.size()) return -1;
            int count = 0;
            for (int y = 0; y < shown.height(); ++y) {
                const auto *a = reinterpret_cast<const QRgb *>(shown.constScanLine(y));
                const auto *b = reinterpret_cast<const QRgb *>(drawn.constScanLine(y));
                for (int x = 0; x < shown.width(); ++x)
                    if (std::abs(qRed(a[x]) - qRed(b[x])) > 2 || std::abs(qGreen(a[x]) - qGreen(b[x])) > 2 || std::abs(qBlue(a[x]) - qBlue(b[x])) > 2) ++count;
            }
            return count;
        };
        QCOMPARE(differing(), 0);

        // A flight, stopped half way, and then gone.
        StageMotion::Flight flight;
        flight.image = frame;
        flight.from = QRectF(100, 500, 128, 72);
        flight.to = QRectF(800, 600, 180, 44);
        flight.duration = 400;
        flight.ring = true;
        flight.label = QStringLiteral("Interview");
        QVERIFY(window.m_motion->fly(flight));
        QTest::qWait(70);
        window.m_motion->m_ticker.pause();
        QCOMPARE(differing(), 0);
        window.m_motion->m_ticker.resume();
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_motion->isFlying(), 2000);
        QCOMPARE(differing(), 0);

        // The regions, stopped half way there and half way back.
        stage::Panes closed = open;
        closed.picture = false;
        closed.timeline = false;
        const auto halfWay = [&](const stage::Panes &panes, const QRect &origin) {
            window.m_shell->setPanes(panes, origin);
            QTest::qWait(60);
            QVERIFY(window.m_shell->isMoving());
            window.m_shell->m_moves->pause();
            QCOMPARE(differing(), 0);
            window.m_shell->m_moves->resume();
            QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
            QCOMPARE(differing(), 0);
        };
        halfWay(closed, QRect(20, 20, 44, 24));
        halfWay(open, QRect(300, 600, 128, 72));
        window.close();
    }

    // Each start has its end, or the picture stays coarse and flights wait for ever.
    void aMoveCutShortStillEnds() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1400, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        stage::Panes panes = window.m_shell->panes();
        panes.picture = true;
        panes.timeline = false;
        window.m_shell->setPanes(panes);
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        panes.timeline = true;
        QSignalSpy ended(window.m_shell, &StageShell::transitionFinished);
        // By the window changing size.
        window.m_shell->setPanes(panes);
        QVERIFY(window.m_shell->isMoving());
        QVERIFY(window.m_viewer->m_stageMoving);
        window.resize(1300, 860);
        QTRY_VERIFY(!window.m_shell->isMoving());
        QVERIFY(ended.size() >= 1);
        QVERIFY(!window.m_viewer->m_stageMoving);
        // By going back to where it started, with nothing left to move.
        ended.clear();
        panes.timeline = false;
        window.m_shell->setPanes(panes);
        QVERIFY(window.m_shell->isMoving());
        panes.timeline = true;
        window.m_shell->setPanes(panes);
        panes.timeline = false;
        window.m_shell->setPanes(panes);
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        QTRY_VERIFY(!ended.isEmpty());
        QVERIFY(!window.m_viewer->m_stageMoving);
        window.close();
    }

    // Going home, cut short by a new cut: the timeline comes back seen.
    void theVeilDoesNotOutliveAMoveCutShort() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1400, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        window.m_shell->setPanes(open);
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        stage::Panes closed = open;
        closed.picture = false;
        closed.timeline = false;
        window.m_shell->veilTimeline(1.0);
        window.m_shell->setPanes(closed);
        QTRY_VERIFY_WITH_TIMEOUT(window.m_shell->timelineVeil() > 0.0, 2000);
        window.m_shell->setPanes(open);
        QTRY_COMPARE_WITH_TIMEOUT(window.m_shell->timelineVeil(), 0.0, 2000);
        // And gone with the timeline when the window's size cuts it short.
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        window.m_shell->veilTimeline(1.0);
        window.m_shell->setPanes(closed);
        QTRY_VERIFY_WITH_TIMEOUT(window.m_shell->timelineVeil() > 0.0, 2000);
        window.resize(1300, 860);
        QTRY_COMPARE_WITH_TIMEOUT(window.m_shell->timelineVeil(), 0.0, 1000);
        window.close();
    }

    void aStandInHoldingStillDrawsNoFrames() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        StageMotion layer;
        layer.resize(800, 600);
        layer.show();
        StageMotion::Flight flight;
        flight.image = footageFrame(QSize(320, 180));
        flight.from = QRectF(0, 0, 320, 180);
        flight.to = flight.from;
        flight.fromOpacity = 0.0;
        flight.duration = 60;
        flight.stays = true;
        flight.fit = true;
        QVERIFY(layer.fly(flight));
        QVERIFY(layer.isTicking());
        QTRY_VERIFY(!layer.isTicking());
        QVERIFY(layer.isFlying());
        layer.releaseStaying();
        QVERIFY(layer.isTicking());
        QTRY_VERIFY(!layer.isFlying());
        QVERIFY(layer.isHidden());
    }

    // Let go on its way or as it comes in, it neither jumps nor flashes.
    void aStandInLetGoEarlyGoesOnFromWhereItIs() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        StageMotion layer;
        layer.resize(800, 600);
        layer.show();
        StageMotion::Flight flight;
        flight.image = footageFrame(QSize(320, 180));
        flight.from = QRectF(0, 0, 100, 60);
        flight.to = QRectF(400, 300, 300, 180);
        flight.fromOpacity = 0.0;
        flight.duration = 3000;
        flight.lively = false;
        flight.stays = true;
        flight.fit = true;
        QVERIFY(layer.fly(flight));
        QTest::qWait(30);
        layer.m_ticker.pause();
        const QRectF box = layer.boxOf(layer.m_flights.first());
        const qreal opacity = layer.opacityOf(layer.m_flights.first());
        QVERIFY(opacity < 1.0);
        const qint64 then = layer.m_now;
        layer.releaseStaying();
        // Seen at the same moment, it is where it was.
        layer.m_now = then;
        QVERIFY(!layer.m_flights.first().flight.stays);
        QCOMPARE(layer.boxOf(layer.m_flights.first()), box);
        QCOMPARE(layer.opacityOf(layer.m_flights.first()), opacity);
        layer.m_ticker.resume();
        QTRY_VERIFY_WITH_TIMEOUT(!layer.isFlying(), 6000);
    }

    void aStandInGoesWithThePicture() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        StageMotion layer;
        layer.resize(800, 600);
        layer.show();
        StageMotion::Flight flight;
        flight.from = QRectF(0, 0, 100, 60);
        flight.to = flight.from;
        flight.duration = 20;
        flight.stays = true;
        flight.fit = true;
        QVERIFY(layer.fly(flight));
        QTRY_VERIFY(!layer.isTicking());
        const QRect picture(200, 100, 300, 180);
        layer.followPicture(picture);
        QVERIFY(layer.isTicking());
        const QRectF start = layer.boxOf(layer.m_flights.first());
        QVERIFY(start.width() < 110);
        QTRY_VERIFY(!layer.isTicking());
        QCOMPARE(layer.boxOf(layer.m_flights.first()), QRectF(picture));
        // With no picture left there is nothing to stand in for.
        layer.followPicture(QRect());
        QTRY_VERIFY(!layer.isFlying());
    }

    // Two dims at once would fight, and the first to end would take the second's
    // effect with it.
    void theLastDimIsTheOneThatStays() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QWidget widget;
        widget.resize(100, 100);
        widget.show();
        moves::dim(&widget, 0.3, 200);
        QTest::qWait(50);
        moves::dim(&widget, 1.0, 100);
        QTest::qWait(30);
        moves::dim(&widget, 0.3, 300);
        QTest::qWait(450);
        auto *effect = qobject_cast<QGraphicsOpacityEffect *>(widget.graphicsEffect());
        QVERIFY(effect);
        QCOMPARE(effect->opacity(), 0.3);
        moves::dim(&widget, 1.0, 50);
        QTRY_VERIFY(!widget.graphicsEffect());
    }

    // It is drawn rising: the layout it is in keeps it, whatever happens meanwhile.
    void aRiseLeavesTheLayoutAlone() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QWidget host;
        auto *layout = new QVBoxLayout(&host);
        auto *above = new QLabel(QStringLiteral("Above"));
        auto *child = new QLabel(QStringLiteral("Rising"));
        layout->addWidget(above);
        layout->addWidget(child);
        host.resize(300, 200);
        host.show();
        QVERIFY(QTest::qWaitForWindowExposed(&host));
        const QPoint home = child->pos();
        moves::rise(child, 0, 14, 200);
        QVERIFY(child->graphicsEffect());
        QCOMPARE(child->pos(), home);
        QTest::qWait(40);
        QCOMPARE(child->pos(), home);
        // The layout moves it on the way.
        above->setMinimumHeight(above->height() + 30);
        QTRY_VERIFY(child->pos().y() > home.y());
        const QPoint moved = child->pos();
        QTRY_VERIFY(!child->graphicsEffect());
        QCOMPARE(child->pos(), moved);
    }

    // Colour around a picture changes how its colours look: however strong its own,
    // what is around it stays grey.
    void theRoomTakesNoColourFromThePicture() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1400, 900);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        stage::Panes open = window.m_shell->panes();
        open.picture = true;
        open.timeline = true;
        window.m_shell->setPanes(open);
        QImage red(1920, 1080, QImage::Format_RGBA8888);
        red.fill(QColor(255, 0, 0));
        window.m_viewer->setFrame(red, 0.0);
        QCoreApplication::processEvents();
        const QRect picture = window.m_shell->rects().picture;
        const QPoint shell = window.m_shell->mapTo(&window, QPoint());
        const QImage shown = window.grab().toImage();
        if (const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR"); !shots.isEmpty()) shown.save(shots + QStringLiteral("/room-neutral.png"));
        const qreal ratio = shown.devicePixelRatio();
        // In the slot to each side of the picture, and in the conversation beside it.
        for (const int x : {picture.left() - 8, picture.left() - 40, picture.right() + 8, picture.right() + 40}) {
            const QPoint at = (QPoint(x, picture.center().y()) + shell) * ratio;
            if (!shown.rect().contains(at)) continue;
            const QColor colour = shown.pixelColor(at);
            QVERIFY2(colour.red() == colour.green() && colour.green() == colour.blue(), qPrintable(colour.name()));
        }
        // And the picture is the picture.
        QCOMPARE(shown.pixelColor((picture.center() + shell) * ratio).rgb(), QColor(255, 0, 0).rgb());
        window.close();
    }

    // Again before the last has landed, it starts over, with one rise under way.
    void focusedClipsRiseOnce() {
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        TimelineWidget timeline;
        timeline.resize(800, 300);
        timeline.show();
        timeline.setFocusClips({QStringLiteral("a")});
        QTRY_VERIFY_WITH_TIMEOUT(timeline.m_focusLift > 0.0, 2000);
        timeline.setFocusClips({QStringLiteral("b")});
        QCOMPARE(timeline.m_focusLift, 0.0);
        QTRY_COMPARE_WITH_TIMEOUT(timeline.m_focusLift, tokens::motion::clipLift, 3000);
        timeline.setFocusClips({});
        QCOMPARE(timeline.m_focusLift, 0.0);
        QTest::qWait(60);
        QCOMPARE(timeline.m_focusLift, 0.0);
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::MotionTest)

#include "motion_test.moc"
