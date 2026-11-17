// Tests for the window's content: the geometry of the three regions (a pure
// function of the window's size and the picture's aspect), the sheets over the
// conversation, and Esc.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "main_window.h"
#include "media_artwork.h"
#include "proposal_card.h"
#include "project_launcher.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "ui/components.h"
#include "ui/editor_bus.h"
#include "ui/change_reel.h"
#include "ui/stage_motion.h"
#include "ui/composer.h"
#include "ui/wiring/chat_wiring.h"
#include "ui/project_sidebar.h"
#include "ui/stage_shell.h"
#include "ui/wiring/timeline_wiring.h"
#include "ui/wiring/toolbar_wiring.h"
#include "viewer_widget.h"

#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeySequence>
#include <QLineEdit>
#include <QSet>
#include <QSettings>
#include <QScopeGuard>
#include <QTest>

#include <cmath>

namespace kerf {

using namespace test;

namespace {

constexpr double kWide = 16.0 / 9.0;
constexpr double kPortrait = 9.0 / 16.0;
constexpr double kFeed = 4.0 / 5.0;

QRect inWindow(QWidget *widget, QWidget *window) { return QRect(widget->mapTo(window, QPoint()), widget->size()); }

// Nothing overlaps, nothing is left over: the regions fill the content.
void expectFilled(const stage::Rects &r, const QSize &window, const char *what) {
    const QRect content(0, 40, window.width(), window.height() - 40);
    for (const QRect &region : {r.sidebar, r.slot, r.transport, r.conversation, r.timeline, r.media})
        QVERIFY2(region.isEmpty() || content.contains(region), what);
    for (const QRect &region : {r.sidebar, r.slot, r.transport, r.conversation, r.timeline})
        QVERIFY2(!r.media.intersects(region), what);
    QVERIFY2(!r.slot.intersects(r.conversation), what);
    QVERIFY2(!r.slot.intersects(r.timeline), what);
    QVERIFY2(!r.slot.intersects(r.transport), what);
    QVERIFY2(!r.conversation.intersects(r.timeline), what);
    QVERIFY2(!r.transport.intersects(r.timeline), what);
    QVERIFY2(!r.sidebar.intersects(r.conversation), what);
    const int area = r.sidebar.width() * r.sidebar.height() + r.slot.width() * r.slot.height() + r.transport.width() * r.transport.height() +
                     r.conversation.width() * r.conversation.height() + r.timeline.width() * r.timeline.height() +
                     r.media.width() * r.media.height();
    QCOMPARE(area, content.width() * content.height());
    QVERIFY2(r.slot.isEmpty() || r.slot.contains(r.picture), what);
}

// An empty program, and Kerf proposing a first cut of it.
QJsonObject proposingAFirstCut() {
    QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value(QStringLiteral("timeline")).toObject();
    timeline.insert(QStringLiteral("tracks"), QJsonArray{});
    snapshot.insert(QStringLiteral("timeline"), timeline);
    snapshot.insert(QStringLiteral("duration"), 0.0);
    return snapshot;
}

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

} // namespace

class StageShellTest final : public QObject {
    Q_OBJECT

private slots:
    void wideAtTheDesignSize() {
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide);
        QVERIFY(r.wide);
        QCOMPARE(r.sidebar, QRect(0, 40, 280, 960));
        QCOMPARE(r.conversation, QRect(280, 40, 448, 960));
        QCOMPARE(r.panel, QRect(728, 40, 872, 960));
        QCOMPARE(r.slot, QRect(728, 40, 872, 420));
        QCOMPARE(r.picture, QRect(812, 52, 704, 396));
        QVERIFY(r.transport.isEmpty());
        QVERIFY(r.media.isEmpty());
        QCOMPARE(r.timeline, QRect(728, 460, 872, 540));
        expectFilled(r, QSize(1600, 1000), "16:9");
    }

    // The picture over the timeline, in whatever shape.
    void verticalAtTheDesignSize() {
        for (const double aspect : {kPortrait, kFeed, 1.0}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect);
            QVERIFY(!r.wide);
            QCOMPARE(r.conversation, QRect(280, 40, 448, 960));
            QCOMPARE(r.slot, QRect(728, 40, 872, 614));
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), aspect));
            QVERIFY(r.transport.isEmpty());
            QCOMPARE(r.timeline, QRect(728, 654, 872, 346));
            expectFilled(r, QSize(1600, 1000), "vertical");
        }
    }

    void wideAtOtherWindowSizes() {
        // The least window.
        {
            const stage::Rects r = stage::rects(QSize(1080, 700), kWide);
            QVERIFY(r.sidebar.isEmpty());
            QCOMPARE(r.conversation, QRect(0, 40, 448, 660));
            QCOMPARE(r.slot, QRect(448, 40, 632, 289));
            QCOMPARE(r.timeline, QRect(448, 329, 632, 371));
            expectFilled(r, QSize(1080, 700), "least");
        }
        // Narrow: the conversation gives way, to 320 and no further.
        {
            const stage::Rects r = stage::rects(QSize(950, 900), kWide);
            QCOMPARE(r.conversation.width(), 350);
            const stage::Rects least = stage::rects(QSize(800, 900), kWide);
            QCOMPARE(least.conversation.width(), 320);
            QCOMPARE(least.slot.width(), 480);
            expectFilled(least, QSize(800, 900), "narrow");
        }
        // Large: the picture pillarboxes in black in its slot.
        {
            const stage::Rects r = stage::rects(QSize(2560, 1440), kWide);
            QCOMPARE(r.conversation.width(), 448);
            QCOMPARE(r.slot, QRect(728, 40, 1832, 612));
            QCOMPARE(r.timeline, QRect(728, 652, 1832, 788));
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), kWide));
            QCOMPARE(r.picture.height(), 588);
            expectFilled(r, QSize(2560, 1440), "large");
        }
        // Short: the timeline keeps its least.
        {
            const stage::Rects r = stage::rects(QSize(1600, 500), kWide);
            QCOMPARE(r.timeline.height(), 260);
            QCOMPARE(r.slot.height(), 200);
            expectFilled(r, QSize(1600, 500), "short");
        }
        // Tall: the picture is flush, and the timeline has the rest.
        {
            const stage::Rects r = stage::rects(QSize(1600, 1800), kWide);
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), kWide));
            QCOMPARE(r.picture.height(), r.slot.height() - 24);
            QCOMPARE(r.timeline.height(), 1760 - r.slot.height());
            expectFilled(r, QSize(1600, 1800), "tall");
        }
    }

    void verticalAtOtherWindowSizes() {
        {
            const stage::Rects r = stage::rects(QSize(1280, 800), 1.0);
            QCOMPARE(r.conversation, QRect(280, 40, 400, 760));
            QCOMPARE(r.slot, QRect(680, 40, 600, 486));
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), 1.0));
            QCOMPARE(r.timeline, QRect(680, 526, 600, 274));
            expectFilled(r, QSize(1280, 800), "square");
        }
        // Short: the timeline keeps its least.
        {
            const stage::Rects r = stage::rects(QSize(1600, 500), kPortrait);
            QCOMPARE(r.timeline.height(), 220);
            QCOMPARE(r.slot.height(), 240);
            expectFilled(r, QSize(1600, 500), "short");
        }
    }

    // A window too small for every least still places everything, whole.
    void aWindowSmallerThanTheLeastsStillAddsUp() {
        for (const QSize &size : {QSize(400, 300), QSize(200, 100), QSize(1, 41)})
            for (const double aspect : {kWide, kPortrait, 2.39}) {
                const stage::Rects r = stage::rects(size, aspect);
                expectFilled(r, size, "tiny");
                QVERIFY(r.timeline.height() >= 0 && r.conversation.width() >= 0);
            }
    }

    // The Media band lies across the panel, under the picture and over the
    // timeline a drag from it goes to: it covers nothing, and takes nothing
    // from what is said.
    void theMediaBandLiesBetweenThePictureAndTheTimeline() {
        stage::Panes open;
        open.media = true;
        QVERIFY(stage::rects(QSize(1600, 1000), kWide).media.isEmpty());
        {
            // The timeline gives it its top; the picture keeps its place.
            const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, open);
            const stage::Rects closed = stage::rects(QSize(1600, 1000), kWide);
            QCOMPARE(r.conversation, closed.conversation);
            QCOMPARE(r.panel, closed.panel);
            QCOMPARE(r.slot, closed.slot);
            QCOMPARE(r.media, QRect(728, 460, 872, tokens::layout::mediaBand));
            QCOMPARE(r.timeline, QRect(728, 612, 872, 388));
            expectFilled(r, QSize(1600, 1000), "band");
        }
        {
            // Under an upright picture the timeline would be left under its
            // least: the picture gives way the rest.
            const stage::Rects r = stage::rects(QSize(1600, 1000), kPortrait, 40, open);
            QCOMPARE(r.slot, QRect(728, 40, 872, 608));
            QCOMPARE(r.media, QRect(728, 648, 872, 152));
            QCOMPARE(r.timeline, QRect(728, 800, 872, tokens::layout::mediaBandTimelineLeast));
            expectFilled(r, QSize(1600, 1000), "band upright");
        }
        for (const double aspect : {kWide, kPortrait, kFeed, 1.0}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect, 40, open);
            QCOMPARE(r.media.top(), r.slot.bottom() + 1);
            QCOMPARE(r.timeline.top(), r.media.bottom() + 1);
            QCOMPARE(r.media.left(), r.timeline.left());
            QCOMPARE(r.media.width(), r.timeline.width());
        }
        // Under a picture alone, it is over the transport's block.
        open.timeline = false;
        {
            const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, open);
            QCOMPARE(r.media, QRect(728, 760, 872, 152));
            QCOMPARE(r.transport, QRect(728, 912, 872, tokens::layout::pictureTransportLow));
            expectFilled(r, QSize(1600, 1000), "band over the transport");
        }
        // Over a timeline alone, at the panel's top.
        open.timeline = true;
        open.picture = false;
        {
            const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, open);
            QCOMPARE(r.media, QRect(728, 40, 872, 152));
            QCOMPARE(r.timeline, QRect(728, 192, 872, 808));
            expectFilled(r, QSize(1600, 1000), "band over the timeline");
        }
        // With the panel closed there is no band: it belongs to the panel.
        open.timeline = false;
        QVERIFY(stage::rects(QSize(1600, 1000), kWide, 40, open).media.isEmpty());
        // A window too small for it all still places each, whole.
        open.picture = true;
        open.timeline = true;
        for (const QSize &size : {QSize(800, 900), QSize(1600, 400), QSize(400, 300), QSize(200, 100), QSize(1, 41)})
            for (const double aspect : {kWide, kPortrait})
                expectFilled(stage::rects(size, aspect, 40, open), size, "band, small");
    }

    // The sidebar is as wide as its edge was dragged, within its least and most.
    void theSidebarKeepsTheWidthItWasDraggedTo() {
        stage::Panes panes;
        panes.sidebarWidth = 360;
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, panes);
        QCOMPARE(r.sidebar, QRect(0, 40, 360, 960));
        QCOMPARE(r.conversation.left(), 360);
        expectFilled(r, QSize(1600, 1000), "dragged");
        panes.sidebarWidth = 10;
        QCOMPARE(stage::rects(QSize(1600, 1000), kWide, 40, panes).sidebar.width(), tokens::layout::sidebarLeast);
        // As wide as it likes, but for what it leaves the rest of the window.
        panes.sidebarWidth = 4000;
        QCOMPARE(stage::rects(QSize(1600, 1000), kWide, 40, panes).sidebar.width(), 1600 - tokens::layout::sidebarLeaves);
        panes.sidebarWidth = 600;
        QCOMPARE(stage::rects(QSize(2560, 1440), kWide, 40, panes).sidebar.width(), 600);

        QSettings().remove(QStringLiteral("layout"));
        {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(snapshotOf(QSize(1920, 1080)));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            StageShell *shell = window.m_shell;
            // Its edge takes the drag: a grip over it, from just inside to just outside.
            QWidget *grip = shell->sidebarGrip();
            QVERIFY(grip->isVisible());
            QCOMPARE(grip->width(), tokens::layout::sidebarGrip);
            QCOMPARE(grip->geometry().left(), tokens::layout::sidebar - tokens::layout::sidebarGrip / 2);
            QCOMPARE(grip->cursor().shape(), Qt::SplitHCursor);
            QSignalSpy dragged(shell, &StageShell::sidebarWidthChanged);
            // From the middle of the grip, which is the sidebar's edge.
            const int middle = tokens::layout::sidebarGrip / 2;
            QTest::mousePress(grip, Qt::LeftButton, Qt::NoModifier, QPoint(middle, 100));
            QTest::mouseMove(grip, QPoint(middle + 52, 100));
            QTest::mouseRelease(grip, Qt::LeftButton, Qt::NoModifier, QPoint(middle + 52, 100));
            QCOMPARE(dragged.size(), 1);
            // A drag is not a press: the sidebar stays.
            QTest::qWait(QApplication::doubleClickInterval() + 50);
            QVERIFY(shell->panes().sidebar);
            const int width = shell->rects().sidebar.width();
            QCOMPARE(width, tokens::layout::sidebar + 52);
            QCOMPARE(window.m_sidebar->width(), width);
            // The conversation starts where it ends, and the toolbar's stretch follows it.
            QCOMPARE(window.m_chat->geometry().left(), width);
            QCOMPARE(window.m_toolbarWiring->sidebarEdge(), width);
            // And it is kept.
            QCOMPARE(QSettings().value(QStringLiteral("layout/sidebarWidth")).toInt(), width);
            // A press alone on its edge puts it away, as the toolbar's button
            // does, once no second press has followed it.
            QTest::mouseClick(grip, Qt::LeftButton, Qt::NoModifier, QPoint(middle, 400));
            QVERIFY(shell->panes().sidebar);
            QTRY_VERIFY(!shell->panes().sidebar);
            QCOMPARE(QSettings().value(QStringLiteral("layout/sidebar")).toBool(), false);
            window.setPaneOpen(QStringLiteral("view.sidebar"), true);
            window.close();
        }
        {
            MainWindow next{QString(), QString()};
            QCOMPARE(next.m_shell->panes().sidebarWidth, tokens::layout::sidebar + 52);
            // A double-click on its edge puts it back.
            next.resize(1600, 1000);
            next.show();
            QVERIFY(QTest::qWaitForWindowExposed(&next));
            QTest::mouseDClick(next.m_shell->sidebarGrip(), Qt::LeftButton, Qt::NoModifier, QPoint(4, 100));
            QCOMPARE(next.m_shell->rects().sidebar.width(), tokens::layout::sidebar);
            // And it stays: the press of a double-click does not put it away.
            QTest::qWait(QApplication::doubleClickInterval() + 50);
            QVERIFY(next.m_shell->panes().sidebar);
            next.close();
        }
        QSettings().remove(QStringLiteral("layout"));
    }

    // The transport is in the strip over a wide sequence and under a vertical
    // picture, with the playhead's timecode, and it works from either.
    void theTransportFollowsTheFormat() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        TimelineToolStrip *strip = window.m_timelineWiring->tools();
        PictureTransport *block = window.m_timelineWiring->pictureTransport();
        ui::IconButton *play = strip->transportButton(TimelineToolStrip::Transport::Play);
        QVERIFY(strip->isAncestorOf(play));
        QCoreApplication::processEvents();
        QCOMPARE(strip->tools()->x(), 168 + strip->timecode()->width() + 28);

        // A vertical sequence keeps the strip too.
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        QCoreApplication::processEvents();
        QVERIFY(strip->isAncestorOf(play));
        QVERIFY(!block->isVisible());
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));

        // With the timeline closed the block under the picture has it.
        window.m_commands->action(QStringLiteral("view.timeline"))->trigger();
        QCoreApplication::processEvents();
        QVERIFY(block->isAncestorOf(play));
        QVERIFY(play->isVisible());
        QCOMPARE(play->size(), QSize(24, 24));
        // Under the timecode, centred in the block.
        QCoreApplication::processEvents();
        const QRect inBlock(play->mapTo(block, QPoint()), play->size());
        QCOMPARE(inBlock.center().x(), block->width() / 2 - 1);
        window.m_bus->setPlayhead(2.0);
        QCOMPARE(block->time(), timelineTimecode(2.0, window.m_bus->fps()));

        window.m_commands->action(QStringLiteral("view.timeline"))->trigger();
        QCoreApplication::processEvents();
        QVERIFY(strip->isAncestorOf(play));
        QVERIFY(play->isVisible());
        QVERIFY(!block->isVisible());
        QCoreApplication::processEvents();
        QCOMPARE(strip->tools()->x(), 168 + strip->timecode()->width() + 28);
        window.close();
    }

    void fittedKeepsTheShape() {
        QCOMPARE(stage::fitted(QRect(0, 0, 1200, 675), kWide), QRect(0, 0, 1200, 675));
        QCOMPARE(stage::fitted(QRect(10, 20, 100, 100), 1.0), QRect(10, 20, 100, 100));
        QCOMPARE(stage::fitted(QRect(0, 0, 200, 100), 1.0), QRect(50, 0, 100, 100));
        QCOMPARE(stage::fitted(QRect(0, 0, 100, 200), 1.0), QRect(0, 50, 100, 100));
        QVERIFY(stage::fitted(QRect(), 1.0).isEmpty());
    }

    // The window places its regions by the function, and a format change lays
    // out again at once.
    void theWindowFollowsTheFormat() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        const auto expectAt = [&window](double aspect) {
            const stage::Rects r = stage::rects(window.size(), aspect);
            QCOMPARE(inWindow(window.m_viewer, &window), r.slot);
            QCOMPARE(inWindow(window.m_chat, &window), r.conversation);
            QCOMPARE(inWindow(window.m_timelinePanel, &window), r.timeline);
            PictureTransport *block = window.m_timelineWiring->pictureTransport();
            QVERIFY(!block->isVisible());
        };
        expectAt(16.0 / 9.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        // At once: no animation, nothing left to process.
        expectAt(9.0 / 16.0);
        QCOMPARE(window.m_viewer->videoRect(), QRectF(stage::fitted(stage::paddedSlot(QRect(0, 0, 872, 614)), 9.0 / 16.0)));
        window.applySnapshot(snapshotOf(QSize(1080, 1350)));
        expectAt(4.0 / 5.0);
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        expectAt(4.0 / 5.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1080)));
        expectAt(1.0);
        // The black is the picture's: letterboxed at this size.
        QCOMPARE(window.m_viewer->videoRect(), QRectF(stage::fitted(stage::paddedSlot(QRect(0, 0, 600, 486)), 1.0)));
        const QImage shot = window.m_viewer->grab().toImage();
        QCOMPARE(shot.pixelColor(QPoint(10, 4) * shot.devicePixelRatio()).rgb(), qRgb(0, 0, 0));
        window.close();
    }

    // Old settings load without error and are ignored.
    void oldSettingsAreIgnored() {
        QSettings settings;
        settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Edit"));
        settings.setValue(QStringLiteral("workspace/names"), QStringList{QStringLiteral("My own")});
        settings.setValue(QStringLiteral("workspace/changes/Review"), QByteArray("not a layout"));
        settings.setValue(QStringLiteral("workspace/stage/Review"), QVariantMap{{QStringLiteral("panel"), false}, {QStringLiteral("dim"), true}});
        settings.setValue(QStringLiteral("viewer/chatOpen"), false);
        settings.setValue(QStringLiteral("viewer/chatWidth"), 480);
        settings.setValue(QStringLiteral("viewer/devicePreview"), QStringLiteral("off"));
        settings.setValue(QStringLiteral("viewer/laptopPreview"), QStringLiteral("off"));
        settings.setValue(QStringLiteral("editor/inspectorFollowsSelection"), false);
        {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(snapshotOf(QSize(1920, 1080)));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
            QVERIFY(window.m_chat->isVisible());
            QCOMPARE(inWindow(window.m_chat, &window), QRect(280, 40, 448, 960));
            window.close();
        }
        settings.remove(QStringLiteral("workspace"));
        settings.remove(QStringLiteral("viewer"));
        settings.remove(QStringLiteral("editor/inspectorFollowsSelection"));
    }

    void theMediaSheetOpensFromItsCommand() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        StageShell *shell = window.m_shell;
        QCOMPARE(shell->sheet(), StageShell::Sheet::None);
        QVERIFY(window.m_projectPanel->isHidden());
        const struct {
            const char *command;
            StageShell::Sheet sheet;
            QWidget *widget;
            const char *close;
        } sheets[] = {{"window.project", StageShell::Sheet::Media, window.m_projectPanel, "sheetClose.media"}};
        const QRect conversation = inWindow(window.m_chat, &window);
        const QRect timeline = inWindow(window.m_timelinePanel, &window);
        for (const auto &entry : sheets) {
            window.triggerCommand(QString::fromLatin1(entry.command));
            QCOMPARE(shell->sheet(), entry.sheet);
            QVERIFY(entry.widget->isVisible());
            // A band across the panel, under the picture and over the
            // timeline, which gives it its top; the conversation keeps its width.
            QCOMPARE(inWindow(window.m_chat, &window), conversation);
            const QRect band = inWindow(entry.widget, &window);
            QCOMPARE(band.height(), tokens::layout::mediaBand);
            QCOMPARE(band.left(), timeline.left());
            QCOMPARE(band.width(), timeline.width());
            QCOMPARE(band.top(), inWindow(window.m_viewer, &window).bottom() + 1);
            const QRect under = inWindow(window.m_timelinePanel, &window);
            QCOMPARE(under.top(), band.bottom() + 1);
            QCOMPARE(under.bottom(), timeline.bottom());
            // Nothing that is said is under it.
            QVERIFY(!band.intersects(conversation));
            QVERIFY(window.m_commands->action(QString::fromLatin1(entry.command))->isChecked());
            // One at a time.
            int shown = 0;
            for (const auto &other : sheets)
                if (other.widget->isVisible()) ++shown;
            QCOMPARE(shown, 1);
            // The same command closes it, and the timeline has its height back.
            window.triggerCommand(QString::fromLatin1(entry.command));
            QCOMPARE(shell->sheet(), StageShell::Sheet::None);
            QVERIFY(!entry.widget->isVisible());
            QCOMPARE(inWindow(window.m_timelinePanel, &window), timeline);
            QVERIFY(!window.m_commands->action(QString::fromLatin1(entry.command))->isChecked());
            // So does the close button at the end of its strip.
            window.triggerCommand(QString::fromLatin1(entry.command));
            auto *close = entry.widget->findChild<QAbstractButton *>(QString::fromLatin1(entry.close));
            QVERIFY2(close, entry.close);
            QVERIFY(close->isVisible());
            QVERIFY(close->geometry().right() > close->parentWidget()->width() - 16);
            close->click();
            QCOMPARE(shell->sheet(), StageShell::Sheet::None);
            QVERIFY(!window.m_commands->action(QString::fromLatin1(entry.command))->isChecked());
        }
        window.m_shell->setSheet(StageShell::Sheet::None);
        window.close();
    }

    // A bar that shows does not lie over the Media band: its tabs and close
    // stay where they can be reached.
    void aBannerLeavesTheMediaSheetsCloseInReach() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        StageShell *shell = window.m_shell;
        window.m_errorBanner->hide();
        window.triggerCommand(QStringLiteral("window.project"));
        QCoreApplication::processEvents();
        const QRect sheet = inWindow(window.m_projectPanel, &window);
        const QRect chat = inWindow(window.m_chat, &window);
        window.showError(QStringLiteral("Add an API key"));
        QCoreApplication::processEvents();
        const QRect banner = inWindow(window.m_errorBanner, &window);
        QVERIFY(banner.height() > 0);
        // The band is under the picture, clear of the bars at the top: it stays.
        QVERIFY(banner.bottom() < sheet.top());
        QCOMPARE(inWindow(window.m_projectPanel, &window), sheet);
        auto *close = window.m_projectPanel->findChild<QAbstractButton *>(QStringLiteral("sheetClose.media"));
        QVERIFY(close && close->isVisible());
        QVERIFY(!banner.intersects(inWindow(close, &window)));
        // The rest of the arrangement is as it was.
        QCOMPARE(inWindow(window.m_chat, &window), chat);
        QCOMPARE(shell->rects().sidebar.top(), 0);
        window.m_errorBanner->hide();
        QCoreApplication::processEvents();
        QCOMPARE(inWindow(window.m_projectPanel, &window), sheet);
        QCOMPARE(inWindow(window.m_chat, &window), chat);
        window.m_shell->setSheet(StageShell::Sheet::None);
        window.close();
    }

    // A Mac's Control key is Meta in the keymaps. Elsewhere those keys go on
    // Ctrl, away from the Windows and Super keys, and take no other command's.
    void metaKeysMoveToCtrlOffTheMac() {
        QCOMPARE(CommandRegistry::keymapKey(QStringLiteral("Meta+Alt+4"), true), QKeySequence(Qt::META | Qt::ALT | Qt::Key_4));
        QCOMPARE(CommandRegistry::keymapKey(QStringLiteral("Meta+Alt+4"), false), QKeySequence(Qt::CTRL | Qt::ALT | Qt::Key_4));
        QCOMPARE(CommandRegistry::keymapKey(QStringLiteral("Meta+Shift+2"), false), QKeySequence(Qt::CTRL | Qt::SHIFT | Qt::Key_2));
        QCOMPARE(CommandRegistry::keymapKey(QStringLiteral("Ctrl+Return"), false), QKeySequence(Qt::CTRL | Qt::Key_Return));
        QCOMPARE(CommandRegistry::keymapKey(QStringLiteral("Shift+F"), false), QKeySequence(Qt::SHIFT | Qt::Key_F));

        MainWindow window{QString(), QString()};
        CommandRegistry *commands = window.m_commands;
        const QList<QAction *> actions = commands->findChildren<QAction *>();
        QVERIFY(actions.size() > 50);
        const QString original = commands->keymap();
        int moved = 0;
        for (const QJsonValue &keymap : commands->keymaps()) {
            commands->setKeymap(keymap.toObject().value(QStringLiteral("id")).toString());
            QSet<QString> others;
            QStringList meta;
            for (const QAction *action : actions)
                for (const QKeySequence &shortcut : action->shortcuts()) {
                    const QString text = shortcut.toString(QKeySequence::PortableText);
                    if (text.contains(QStringLiteral("Meta+"))) meta << text;
                    else others << text;
                }
            for (const QString &text : std::as_const(meta)) {
                ++moved;
                const QString there = CommandRegistry::keymapKey(text, false).toString(QKeySequence::PortableText);
                QVERIFY2(!others.contains(there), qPrintable(commands->keymap() + QLatin1Char(' ') + there));
            }
        }
        commands->setKeymap(original);
        // One is left to move since the Inspector's and the trays' went: Final
        // Cut Pro's Render Preview.
        if (CommandRegistry::isMac()) QVERIFY(moved >= 1);
    }

    // Ask Kerf and ⇧⌘J: what lies over the composer goes, so the text lands
    // where it can be seen.
    void askingFindsTheComposerInSight() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1600, 900);
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30},
                                         {"timeline", QJsonObject{{"width", 1920}, {"height", 1080}, {"fps", 24}, {"tracks", QJsonArray{}}}}});
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        emit window.m_bus->askPrefill(QStringLiteral("Fix it"));
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("Fix it"));
        // The Media column lies beside the conversation, not over it: asking
        // finds the composer in sight, and leaves the column open.
        window.triggerCommand(QStringLiteral("window.project"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Media);
        window.triggerCommand(QStringLiteral("window.assistant"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Media);
        QVERIFY(window.m_chatInput->hasFocus());
        QVERIFY(!inWindow(window.m_projectPanel, &window).intersects(inWindow(window.m_chat->composerWidget(), &window)));
        window.m_shell->setSheet(StageShell::Sheet::None);
        window.close();
    }

    void selectingAClipOpensNothing() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const QString clip = window.m_timeline->m_project.value(QStringLiteral("tracks")).toArray().at(0).toObject()
                                 .value(QStringLiteral("clips")).toArray().at(0).toObject().value(QStringLiteral("id")).toString();
        QVERIFY(!clip.isEmpty());
        emit window.m_timeline->clipSelected(clip);
        QCOMPARE(window.m_selectedClip, clip);
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        emit window.m_timeline->clipSelectionExtended(clip, false);
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        emit window.m_timeline->deselectRequested();
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        window.close();
    }

    // Esc over a proposal closes the sheet first; it does not reject it.
    void escClosesTheSheetAheadOfTheProposal() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.activateWindow();
        window.m_timeline->setFocus();
        QTRY_VERIFY(window.m_timeline->hasFocus());
        window.triggerCommand(QStringLiteral("window.project"));
        window.m_lastCommand = {};
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        QVERIFY(window.m_lastCommand.value(QStringLiteral("type")).toString() != QStringLiteral("reject"));
        // With nothing open, Esc is the timeline's again.
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("type")).toString(), QStringLiteral("reject"));
        // A text field keeps its own Esc; the next one, unanswered, reaches the window.
        window.triggerCommand(QStringLiteral("window.project"));
        auto *search = window.m_projectPanel->findChild<QLineEdit *>();
        QVERIFY(search);
        window.m_chatInput->setFocus();
        QTRY_VERIFY(window.m_chatInput->hasFocus());
        QTest::keyClick(window.m_chatInput, Qt::Key_Escape);
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        window.close();
    }

    // The Media sheet never lies over the timeline.
    void theSheetLeavesTheTimelineInSight() {
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(snapshotOf(canvas));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            for (const char *command : {"window.project"}) {
                window.triggerCommand(QString::fromLatin1(command));
                QWidget *sheet = window.m_shell->widgetOf(window.m_shell->sheet());
                QVERIFY(sheet && sheet->isVisible());
                QVERIFY(!inWindow(sheet, &window).intersects(inWindow(window.m_timelinePanel, &window)));
                QVERIFY(window.m_timeline->isVisible() && window.m_timeline->isEnabled());
                window.triggerCommand(QString::fromLatin1(command));
            }
            window.close();
        }
    }

    void aConversationAloneHasTheRoom() {
        stage::Panes panes;
        panes.picture = false;
        panes.timeline = false;
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, panes);
        QCOMPARE(r.sidebar, QRect(0, 40, 280, 960));
        QCOMPARE(r.conversation, QRect(280, 40, 1320, 960));
        QVERIFY(r.panel.isEmpty());
        QVERIFY(r.slot.isEmpty());
        QVERIFY(r.timeline.isEmpty());
        QVERIFY(r.transport.isEmpty());
        expectFilled(r, QSize(1600, 1000), "conversation");
        panes.sidebar = false;
        QCOMPARE(stage::rects(QSize(1600, 1000), kWide, 40, panes).conversation, QRect(0, 40, 1600, 960));
    }

    void onePaneAloneHasThePanel() {
        stage::Panes picture;
        picture.timeline = false;
        const stage::Rects wide = stage::rects(QSize(1600, 1000), kWide, 40, picture);
        QCOMPARE(wide.slot, QRect(728, 40, 872, 872));
        QCOMPARE(wide.transport, QRect(728, 912, 872, 88));
        expectFilled(wide, QSize(1600, 1000), "picture");
        const stage::Rects upright = stage::rects(QSize(1600, 1000), kPortrait, 40, picture);
        QCOMPARE(upright.slot, QRect(728, 40, 872, 872));
        QCOMPARE(upright.transport, QRect(728, 912, 872, 88));
        expectFilled(upright, QSize(1600, 1000), "upright picture");

        stage::Panes timeline;
        timeline.picture = false;
        for (const double aspect : {kWide, kPortrait}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect, 40, timeline);
            QCOMPARE(r.timeline, r.panel);
            QVERIFY(r.slot.isEmpty());
            QVERIFY(r.transport.isEmpty());
            expectFilled(r, QSize(1600, 1000), "timeline");
        }
    }

    void theSidebarGivesWayInANarrowWindow() {
        QCOMPARE(stage::rects(QSize(1100, 800), kWide).sidebar.width(), 280);
        QVERIFY(stage::rects(QSize(1099, 800), kWide).sidebar.isEmpty());
    }

    // A project starts as a conversation; the panel opens with the first thing
    // to show, then stays as it is left.
    void thePanelOpensWithTheFirstThingToShow() {
        QJsonObject empty = snapshotOf(QSize(1920, 1080));
        QJsonObject timeline = empty.value(QStringLiteral("timeline")).toObject();
        timeline.insert(QStringLiteral("tracks"), QJsonArray{});
        empty.insert(QStringLiteral("timeline"), timeline);

        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(empty);
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        QVERIFY(!window.m_shell->panes().panelOpen());
        QVERIFY(!window.m_viewer->isVisible());
        QVERIFY(!window.m_timelinePanel->isVisible());
        QCOMPARE(inWindow(window.m_chat, &window), QRect(280, 40, 1320, 960));
        // No picture to stamp.
        QVERIFY(!window.m_chat->composerWidget()->stamp()->isVisible());
        QVERIFY(!window.m_chat->composerWidget()->stamped());

        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCoreApplication::processEvents();
        QVERIFY(window.m_viewer->isVisible());
        QVERIFY(window.m_timelinePanel->isVisible());
        QCOMPARE(inWindow(window.m_chat, &window), QRect(280, 40, 448, 960));
        QVERIFY(window.m_chat->composerWidget()->stamped());
        QVERIFY(window.m_commands->action(QStringLiteral("view.picture"))->isChecked());

        // Closed by hand, it stays closed through the next snapshot.
        window.m_commands->action(QStringLiteral("view.picture"))->trigger();
        window.m_commands->action(QStringLiteral("view.timeline"))->trigger();
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QVERIFY(!window.m_shell->panes().panelOpen());
        // Asking to look at a time shows the picture.
        emit window.m_chat->seekRequested(1.0);
        QVERIFY(window.m_shell->panes().picture);
        QVERIFY(!window.m_shell->panes().timeline);
        window.close();
    }

    // While Kerf makes a first cut, its card is in Kerf's answer, at work,
    // with nothing to do yet; done, the proposal's card shows that it is done.
    void aCutAtWorkIsItsCardInTheTurn() {
        QSettings().remove(QStringLiteral("layout"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const QJsonObject proposing = proposingAFirstCut();
        window.applySnapshot(proposing);
        window.m_chatWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", proposing}});
        fakeArtwork(window.m_artwork);
        emit window.m_artwork->changed();
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
        QVERIFY(!window.m_shell->panes().panelOpen());
        ProposalCard *building = window.m_chat->buildingCard();
        QVERIFY(building);
        QCOMPARE(window.m_chat->buildingProposal(), QStringLiteral("p7"));
        QVERIFY(window.m_chat->liveMessage()->isAncestorOf(building));
        QVERIFY(building->isVisible() && building->picture()->isVisible());
        QVERIFY(building->picture()->isWorking());
        QVERIFY(building->lineTitle()->isVisible());
        QVERIFY(!building->acceptButton()->isVisible() && !building->rejectButton()->isVisible());
        // Only the one at work: the proposal waits for it.
        QCOMPARE(window.m_chat->cards().size(), 1);
        // At work the picture is the newest clip placed.
        QCOMPARE(building->picture()->poster().cacheKey(), building->picture()->picture().frames.constLast().poster.cacheKey());
        // Another look at the same cut keeps the same card.
        window.applySnapshot(proposing);
        QCOMPARE(window.m_chat->buildingCard(), building);

        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
        QVERIFY(window.m_chat->buildingProposal().isEmpty());
        QVERIFY(!window.m_chat->buildingCard());
        ProposalCard *card = window.m_chat->card(QStringLiteral("p7"));
        QVERIFY(card);
        QVERIFY(!card->picture()->isWorking());
        QVERIFY(card->picture()->isFinishing());
        QVERIFY(card->acceptButton()->isVisible() && card->rejectButton()->isVisible());
        window.close();
    }

    // A cut Kerf makes stays in the conversation as its card's picture; pressing
    // it opens the stage out of it, and closing the stage puts it back.
    void aCutKerfMadeOpensFromItsCard() {
        QSettings().remove(QStringLiteral("layout"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        const QJsonObject proposing = proposingAFirstCut();
        window.applySnapshot(proposing);
        window.m_chatWiring->handleEvent(QStringLiteral("state"), QJsonObject{{"type", "state"}, {"snapshot", proposing}});
        for (int i = 0; i < 4; ++i) QCoreApplication::processEvents();
        QVERIFY(!window.m_shell->panes().panelOpen());
        ProposalCard *card = window.m_chat->card(QStringLiteral("p7"));
        QVERIFY(card);
        CardPicture *picture = card->picture();
        // No poster yet, no picture; it comes with them.
        QVERIFY(!picture->isVisible());
        fakeArtwork(window.m_artwork);
        emit window.m_artwork->changed();
        QCoreApplication::processEvents();
        QVERIFY(picture->isVisible());
        QCOMPARE(picture->picture().frames.size(), 2);
        QCOMPARE(picture->picture().duration, 18.0);
        QCOMPARE(picture->pictureRect().size(), QSize(440, 248));
        QCOMPARE(picture->pictureRect().topLeft(), QPoint(tokens::layout::cardRing, tokens::layout::cardRing));
        QVERIFY(!picture->poster().isNull());
        // The picture says what the cut is: the card keeps to its title, Reject and
        // Accept, on one line under it.
        QVERIFY(!card->titleLabel()->isVisible());
        QVERIFY(card->lineTitle()->isVisible());
        QCOMPARE(card->lineTitle()->text(), card->titleLabel()->text());
        QCOMPARE(card->lineTitle()->y(), card->acceptButton()->y());
        QVERIFY(!card->changesLink()->isVisible());
        QVERIFY(!card->watchButton()->isVisible());
        QVERIFY(!card->previewBox()->isVisible());
        QVERIFY(!card->scriptButton()->isVisible());
        QVERIFY(card->acceptButton()->isVisible() && card->rejectButton()->isVisible());

        QTest::mouseClick(picture, Qt::LeftButton, {}, picture->pictureRect().center());
        QVERIFY(window.m_shell->panes().picture && window.m_shell->panes().timeline);
        QCOMPARE(window.m_chat->stageProposal(), QStringLiteral("p7"));
        card = window.m_chat->card(QStringLiteral("p7"));
        QVERIFY(card->picture()->onStage());
        // Pressed again while the stage has it, it asks for nothing.
        QSignalSpy asked(window.m_chat, &ChatPanel::pictureRequested);
        QTest::mouseClick(card->picture(), Qt::LeftButton, {}, card->picture()->pictureRect().center());
        QCOMPARE(asked.count(), 0);

        // The stage closes back into the card.
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QVERIFY(!window.cardPictureHome().isEmpty());
        window.m_panelButton->click();
        QVERIFY(window.m_shell->isMoving());
        QVERIFY(window.m_chat->stageProposal().isEmpty());
        QWidget *stagePicture = window.m_shell->m_picture;
        QRect last;
        while (window.m_shell->isMoving()) {
            if (stagePicture->isVisible()) last = stagePicture->geometry();
            QTest::qWait(10);
        }
        QVERIFY(!window.m_shell->panes().panelOpen());
        card = window.m_chat->card(QStringLiteral("p7"));
        QVERIFY(!card->picture()->onStage());
        const QRect home = window.toShell(card->picture(), card->picture()->pictureRect());
        QVERIFY2((last.center() - home.center()).manhattanLength() < 60, "the picture goes back into its card");
        QVERIFY(last.width() < home.width() * 2);

        // Accepted, the program has the cut and its card no picture.
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QVERIFY(window.cutPicture(QStringLiteral("p7")).isEmpty());
        // Nor does the program open the stage now that it is left closed.
        QVERIFY(!window.m_shell->panes().panelOpen());
        QSettings().remove(QStringLiteral("layout"));
        window.close();
    }

    void thePaneButtonsAndTheTransportFollowTheCommands() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        // One button stands for the panel: on while either pane shows.
        QVERIFY(window.m_shell->panes().panelOpen());
        QAction *timelinePane = window.m_commands->action(QStringLiteral("view.timeline"));
        QVERIFY(timelinePane && timelinePane->isChecked());
        ui::IconButton *play = window.m_timelineWiring->tools()->transportButton(TimelineToolStrip::Transport::Play);
        PictureTransport *block = window.m_timelineWiring->pictureTransport();
        QVERIFY(window.m_timelineWiring->tools()->isAncestorOf(play));
        // With the timeline closed the block under the picture has the transport.
        timelinePane->trigger();
        QVERIFY(!timelinePane->isChecked());
        QVERIFY(!window.m_shell->panes().timeline);
        QVERIFY(window.m_shell->panes().panelOpen());
        QVERIFY(block->isVisible());
        QVERIFY(block->isAncestorOf(play));
        timelinePane->trigger();
        QVERIFY(window.m_timelineWiring->tools()->isAncestorOf(play));
        // The button closes both panes, and opens both again.
        window.m_panelButton->click();
        QVERIFY(!window.m_shell->panes().panelOpen());
        QVERIFY(!window.m_shell->panes().panelOpen());
        window.m_panelButton->click();
        QVERIFY(window.m_shell->panes().picture && window.m_shell->panes().timeline);
        // The sidebar's choice is remembered.
        QSettings settings;
        window.m_sidebarButton->click();
        QVERIFY(!window.m_sidebar->isVisible());
        QCOMPARE(settings.value(QStringLiteral("layout/sidebar")).toBool(), false);
        QCOMPARE(inWindow(window.m_chat, &window).left(), 0);
        window.m_sidebarButton->click();
        QVERIFY(window.m_sidebar->isVisible());
        settings.remove(QStringLiteral("layout"));
        window.close();
    }

    void theSidebarListsFoldersAndConversations() {
        QTemporaryDir first;
        QTemporaryDir second;
        QVERIFY(first.isValid() && second.isValid());
        ProjectSidebar sidebar;
        sidebar.setProjects({first.path(), second.path(), first.path() + QStringLiteral("/gone")}, second.path());
        QCOMPARE(sidebar.projects().size(), 2);
        QVERIFY(sidebar.projectRow(first.path()));
        QVERIFY(sidebar.newConversationRow());
        sidebar.setConversations({{QStringLiteral("t1"), QStringLiteral("Make a cut"), ProjectSidebar::State::Review},
                                  {QStringLiteral("t2"), QStringLiteral("An old note"), ProjectSidebar::State::Settled}},
                                 QStringLiteral("t1"));
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t1"))->badge(), tokens::color(tokens::activity::review));
        QVERIFY(sidebar.conversationRow(QStringLiteral("t1"))->isCurrent());
        // Settled says so with its dot at the end too.
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t2"))->badge(), tokens::color(tokens::activity::settled));
        QSignalSpy chosen(&sidebar, &ProjectSidebar::conversationChosen);
        sidebar.conversationRow(QStringLiteral("t2"))->click();
        QCOMPARE(chosen.value(0).value(0).toString(), QStringLiteral("t2"));
        // Another folder opens from its menu; its name only unfolds it.
        QSignalSpy project(&sidebar, &ProjectSidebar::projectChosen);
        sidebar.projectRow(first.path())->click();
        QVERIFY(project.isEmpty());
        QVERIFY(!sidebar.isFolded(first.path()));
        sidebar.setFolded(first.path(), true);
        sidebar.projectMenu(first.path())->findChild<QAction *>(QStringLiteral("sidebarProjectOpen"))->trigger();
        QCOMPARE(project.value(0).value(0).toString(), QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath()));
        // Unfolded, what Kerf works on shows on the conversation; folded, on the folder.
        sidebar.setWorking(true);
        QVERIFY(!sidebar.projectRow(second.path())->isShimmering());
        sidebar.setFolded(second.path(), true);
        QVERIFY(sidebar.projectRow(second.path())->isShimmering());
        sidebar.setFolded(second.path(), false);
        // The start window lists folders alone, and a press chooses one.
        sidebar.setConversationsShown(false);
        QVERIFY(!sidebar.newConversationRow());
        QVERIFY(!sidebar.conversationRow(QStringLiteral("t1")));
        project.clear();
        sidebar.projectRow(first.path())->click();
        QCOMPARE(project.value(0).value(0).toString(), QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath()));
        QSettings().remove(QStringLiteral("sidebar/folded"));
    }

    // What the sidebar asks of the window for a folder that is not the open
    // one: its conversation opens that project on it, and a folder taken off
    // the list is forgotten, with nothing on disk touched.
    void theSidebarsFoldersReachTheWindow() {
        QTemporaryDir first;
        QTemporaryDir second;
        QVERIFY(first.isValid() && second.isValid());
        const QStringList before = ProjectLauncher::recentProjects();
        const auto restore = qScopeGuard([before] {
            ProjectLauncher::clearRecentProjects();
            for (auto it = before.crbegin(); it != before.crend(); ++it) ProjectLauncher::rememberProject(*it);
        });
        ProjectLauncher::clearRecentProjects();
        ProjectLauncher::rememberProject(first.path());
        ProjectLauncher::rememberProject(second.path());
        const QString one = QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath());
        const QString two = QDir::cleanPath(QFileInfo(second.path()).absoluteFilePath());
        QCOMPARE(ProjectLauncher::recentProjects(), (QStringList{two, one}));

        MainWindow window{QString(), QString()};
        QSignalSpy opened(&window, &MainWindow::conversationOpenRequested);
        emit window.m_sidebar->conversationChosenIn(first.path(), QStringLiteral("c-7"));
        QCOMPARE(opened.size(), 1);
        QCOMPARE(opened.first().at(0).toString(), one);
        QCOMPARE(opened.first().at(1).toString(), QStringLiteral("c-7"));
        // The window it opens is on that conversation as soon as it is connected.
        window.openConversation(QStringLiteral("c-7"));
        QCOMPARE(window.m_chatWiring->conversations()->currentId(), QStringLiteral("c-7"));

        emit window.m_sidebar->projectRemoveRequested(first.path());
        QCOMPARE(ProjectLauncher::recentProjects(), QStringList{two});
        QVERIFY(QFileInfo(first.path()).isDir());
        QVERIFY(!window.m_sidebar->projects().contains(one));
        // One that is not on the list changes nothing.
        ProjectLauncher::forgetProject(QStringLiteral("/not/a/project"));
        QCOMPARE(ProjectLauncher::recentProjects(), QStringList{two});
    }

    void aStartingRequestWaitsForTheProject() {
        MainWindow window{QString(), QString()};
        window.startWith(QStringLiteral("Make a 60-second cut"), false);
        QVERIFY(window.m_lastCommand.value(QStringLiteral("type")).toString() != QStringLiteral("agent"));
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("type")).toString(), QStringLiteral("agent"));
        QCOMPARE(window.m_lastCommand.value(QStringLiteral("text")).toString(), QStringLiteral("Make a 60-second cut"));
        // Once.
        window.m_lastCommand = {};
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QVERIFY(window.m_lastCommand.isEmpty());
    }

    // The first snapshot can come before the connection says it is ready.
    void aStartingRequestGoesOnceConnectedWhicheverComesFirst() {
        MainWindow window{QString(), QString()};
        int asked = 0;
        window.m_bus->setSender([&asked](const QJsonObject &command) {
            if (command.value(QStringLiteral("type")).toString() == QStringLiteral("agent")) ++asked;
            return true;
        });
        window.setConnection(QStringLiteral("connecting"));
        window.startWith(QStringLiteral("Make a cut"), false);
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCOMPARE(asked, 0);
        window.setConnection(QStringLiteral("ready"));
        QCOMPARE(asked, 1);
    }

    void aFolderThatStartsAProjectBringsItsMediaIn() {
        QTemporaryDir folder;
        QVERIFY(folder.isValid());
        for (const char *name : {"take.MOV", "note.txt"}) {
            QFile file(QDir(folder.path()).filePath(QString::fromLatin1(name)));
            QVERIFY(file.open(QIODevice::WriteOnly));
        }
        MainWindow window{folder.path(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.m_lastCommand = {};
        window.startWith(QString(), true);
        const QString sent = QString::fromUtf8(QJsonDocument(window.m_lastCommand).toJson());
        QVERIFY2(sent.contains(QStringLiteral("take.MOV")), qPrintable(sent));
        QVERIFY(!sent.contains(QStringLiteral("note.txt")));
    }

    void theReelPlansWhatChangedWithHandles() {
        ProposalModel::ClipChange trim;
        trim.clip = QStringLiteral("c1");
        trim.name = QStringLiteral("sea");
        trim.trimmed = true;
        trim.oldStart = 10;
        trim.oldEnd = 14;
        trim.newStart = 10;
        trim.newEnd = 12;
        ProposalModel::ClipChange near = trim;
        near.name = QStringLiteral("road");
        near.oldStart = 12;
        near.oldEnd = 16;
        near.newStart = 12;
        near.newEnd = 16;
        ProposalModel::ClipChange added;
        added.clip = QStringLiteral("c9");
        added.name = QStringLiteral("cta");
        added.added = true;
        added.newStart = 40;
        added.newEnd = 45;
        const QList<reel::Segment> segments = reel::plan({added, trim, near}, 50, 45);
        QCOMPARE(segments.size(), 2);
        QCOMPARE(segments.at(0).after, 8.0);
        QCOMPARE(segments.at(0).afterEnd, 18.0);
        QCOMPARE(segments.at(0).title, QStringLiteral("Trim sea and 1 more"));
        QCOMPARE(segments.at(1).after, 38.0);
        // The end keeps to what there is.
        QCOMPARE(segments.at(1).afterEnd, 45.0);
        QCOMPARE(segments.at(1).before, 38.0);
        QCOMPARE(reel::total(segments), 17.0);
    }

    void aToolsPositionsAreRead() {
        QCOMPARE(reel::parseTime(QStringLiteral("4.5s"), 24, 0), 4.5);
        QCOMPARE(reel::parseTime(QStringLiteral("96f"), 24, 0), 4.0);
        QCOMPARE(reel::parseTime(QStringLiteral("01:00:04:12"), 24, 3600), 4.5);
        QVERIFY(reel::parseTime(QStringLiteral("soon"), 24, 0) < 0);
    }

    void theReelPlaysEachSideInTurn() {
        QStringList calls;
        ChangeReel::Host host;
        host.preview = [&calls](const QString &id, double start, double duration) {
            calls << QStringLiteral("preview %1 %2 %3").arg(id).arg(start).arg(duration);
            return true;
        };
        host.exitPreview = [&calls] { calls << QStringLiteral("exit"); };
        host.seek = [&calls](double seconds, double) { calls << QStringLiteral("seek %1").arg(seconds); };
        host.play = [&calls](bool on) { calls << (on ? QStringLiteral("play") : QStringLiteral("pause")); };
        ChangeReel watching(host);
        reel::Segment one;
        one.after = 2;
        one.afterEnd = 6;
        one.before = 3;
        one.beforeEnd = 7;
        reel::Segment two;
        two.after = 20;
        two.afterEnd = 24;
        two.before = 21;
        two.beforeEnd = 25;
        QVERIFY(watching.start(QStringLiteral("p1"), {one, two}));
        QVERIFY(watching.isWaiting());
        QVERIFY(calls.contains(QStringLiteral("preview p1 2 22")));
        calls.clear();
        watching.followPreview(QJsonObject{{"status", "running"}});
        QVERIFY(watching.isWaiting());
        watching.followPreview(QJsonObject{{"status", "ready"}, {"proposalId", "p1"}});
        QVERIFY(!watching.isWaiting());
        QCOMPARE(calls, QStringList({QStringLiteral("seek 2"), QStringLiteral("play")}));
        calls.clear();
        watching.followPlayhead(3.0);
        QVERIFY(calls.isEmpty());
        // The end of a segment moves on, with no new render: it is in this one.
        watching.followPlayhead(6.0);
        QCOMPARE(watching.index(), 1);
        QVERIFY(calls.contains(QStringLiteral("seek 20")));
        calls.clear();
        // Before plays the program.
        watching.setSide(ChangeReel::Side::Before);
        QVERIFY(calls.contains(QStringLiteral("exit")));
        QVERIFY(watching.isWaiting());
        calls.clear();
        watching.followPreview(QJsonObject{{"status", "exited"}});
        // Not until playback has reopened the program.
        QVERIFY(watching.isWaiting());
        watching.followPreview(QJsonObject{{"status", "ready"}});
        QVERIFY(!watching.isWaiting());
        QVERIFY(calls.contains(QStringLiteral("seek 21")));
        // The last one ends it, still open.
        watching.followPlayhead(25.0);
        QVERIFY(watching.isFinished());
        QVERIFY(watching.isActive());
        watching.stop();
        QVERIFY(!watching.isActive());
    }

    void flightsWaitForMotionAndEndOnTheirOwn() {
        StageMotion layer;
        layer.resize(400, 300);
        StageMotion::Flight flight;
        flight.from = QRectF(0, 0, 10, 10);
        flight.to = QRectF(100, 100, 20, 20);
        flight.duration = 40;
        // Tests run with motion off: nothing flies.
        QVERIFY(!layer.fly(flight));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QVERIFY(layer.fly(flight));
        QVERIFY(layer.isFlying());
        QTRY_VERIFY(!layer.isFlying());
        flight.stays = true;
        QVERIFY(layer.fly(flight));
        QTest::qWait(120);
        QVERIFY(layer.isFlying());
        layer.releaseStaying();
        QTRY_VERIFY(!layer.isFlying());
    }

    void footageTilesKeepTheirPlacesAsTheyGoOn() {
        FootageSheet sheet;
        sheet.resize(400, 200);
        QImage poster(16, 9, QImage::Format_RGB32);
        poster.fill(Qt::blue);
        QList<FootageTile> tiles;
        for (const char *id : {"a", "b", "c", "d", "e"}) tiles << FootageTile{QString::fromLatin1(id), QString::fromLatin1(id), poster};
        sheet.setTiles(tiles);
        // Four to a row, the fifth under the first, all inside the sheet.
        const QRect first = sheet.tileRect(QStringLiteral("a"));
        QCOMPARE(sheet.tileRect(QStringLiteral("e")).left(), first.left());
        QVERIFY(sheet.tileRect(QStringLiteral("e")).top() > first.bottom());
        QVERIFY(sheet.tileRect(QStringLiteral("d")).right() < sheet.width());
        QCOMPARE(sheet.height(), sheet.tileRect(QStringLiteral("e")).bottom() + 1);
        QVERIFY(sheet.tileRect(QStringLiteral("nothing")).isEmpty());
        sheet.take(QStringLiteral("a"));
        QVERIFY(sheet.isTaken(QStringLiteral("a")));
        QCOMPARE(sheet.tileRect(QStringLiteral("a")), first);
    }

    void aFirstCutShowsOnTheTimelineUntilItIsTheProgram() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QJsonObject empty = snapshotOf(QSize(1920, 1080));
        QJsonObject timeline = empty.value(QStringLiteral("timeline")).toObject();
        const QJsonArray sources = timeline.value(QStringLiteral("sources")).toArray();
        const QJsonObject withClips = timeline;
        timeline.insert(QStringLiteral("tracks"), QJsonArray{});
        empty.insert(QStringLiteral("timeline"), timeline);
        empty.insert(QStringLiteral("duration"), 0.0);
        window.applySnapshot(empty);
        QVERIFY(!window.m_timeline->showsProposedCut());
        // Kerf proposes a cut: it is what the timeline has to show.
        QJsonObject proposing = empty;
        proposing.insert(QStringLiteral("pendingProposal"), QJsonObject{{QStringLiteral("proposalId"), QStringLiteral("p1")}});
        proposing.insert(QStringLiteral("proposalTimeline"), withClips);
        proposing.insert(QStringLiteral("proposalTimelines"), QJsonObject{{QStringLiteral("p1"), withClips}});
        window.m_proposalId = QStringLiteral("p1");
        window.m_snapshot = proposing;
        window.showProposedCut();
        QVERIFY(window.m_timeline->showsProposedCut());
        QVERIFY(window.m_proposedDuration > 0.0);
        // Accepted, the program has it and the timeline is the program's again.
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QVERIFY(!window.m_timeline->showsProposedCut());
        QCOMPARE(window.m_proposedDuration, 0.0);
    }

    void theBeforeStillComesAndGoes() {
        ViewerWidget viewer;
        viewer.resize(640, 360);
        viewer.setProjectInfo(1920, 1080);
        QVERIFY(!viewer.isComparing());
        QImage before(16, 9, QImage::Format_RGB32);
        before.fill(Qt::blue);
        viewer.setCompareFrame(before);
        QVERIFY(viewer.isComparing());
        viewer.setCompareFrame(QImage());
        QVERIFY(!viewer.isComparing());
    }

    void motionUiTokensHoldTogether() {
        namespace motion = tokens::motion;
        // A timeline starts in before the stage lands, and a flight's stand-in
        // appears before it lands.
        QVERIFY(motion::timelineLead < motion::stage);
        QVERIFY(motion::appear < motion::flight);
        QVERIFY(motion::flightLand < motion::flight);
        QVERIFY(motion::chatBehind > 0.0 && motion::chatBehind < 1.0);
        QVERIFY(motion::timelineBehind > 0.0 && motion::timelineBehind < 1.0);
    }

    void theStripSitsOnAnUprightPicture() {
        stage::Panes panes;
        panes.reel = true;
        const stage::Rects r = stage::rects(QSize(1600, 1000), 9.0 / 16.0, 40, panes);
        // Narrower than the strip, the picture has it centred across it.
        QVERIFY(r.picture.width() < r.reel.width() + 24);
        QVERIFY(qAbs(r.reel.center().x() - r.picture.center().x()) <= 1);
        QCOMPARE(r.reel.top(), r.picture.top() + 12);
        QVERIFY(r.slot.contains(r.reel));
    }

    void aProposedCutBringsItsOwnFormat() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QJsonObject empty = snapshotOf(QSize(1920, 1080));
        QJsonObject timeline = empty.value(QStringLiteral("timeline")).toObject();
        QJsonObject upright = timeline;
        upright.insert(QStringLiteral("width"), 1080);
        upright.insert(QStringLiteral("height"), 1920);
        timeline.insert(QStringLiteral("tracks"), QJsonArray{});
        empty.insert(QStringLiteral("timeline"), timeline);
        empty.insert(QStringLiteral("duration"), 0.0);
        window.applySnapshot(empty);
        QVERIFY(stage::isWide(window.m_viewer->pictureAspect()));
        empty.insert(QStringLiteral("proposalTimelines"), QJsonObject{{QStringLiteral("p1"), upright}});
        window.m_proposalId = QStringLiteral("p1");
        window.m_snapshot = empty;
        window.showProposedCut();
        QVERIFY(!stage::isWide(window.m_viewer->pictureAspect()));
        // Another look at the program does not take it back while the cut is shown.
        window.m_snapshot = empty;
        window.showProposedCut();
        QVERIFY(!stage::isWide(window.m_viewer->pictureAspect()));
    }

    void theRoomHasNoLineAndThePictureFloats() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCoreApplication::processEvents();
        QVERIFY(window.m_shell->rects().panel.isValid());
        QVERIFY(!window.findChild<QWidget *>(QStringLiteral("stageSeamUpright"))->isVisible());
        QVERIFY(window.m_viewer->m_floating);
        // The timeline can fall back under the room.
        window.m_shell->veilTimeline(0.6, 0, 0);
        QCOMPARE(window.m_shell->timelineVeil(), 0.6);
        window.m_shell->veilTimeline(0.0, 0, 0);
        QCOMPARE(window.m_shell->timelineVeil(), 0.0);
    }

    void regionsMoveToANewArrangementWhenMotionIsOn() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        window.m_commands->action(QStringLiteral("view.timeline"))->trigger();
        // On its way, then there.
        QVERIFY(window.m_shell->isMoving());
        QVERIFY(window.m_timelinePanel->isVisible());
        QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        QVERIFY(!window.m_timelinePanel->isVisible());
        QCOMPARE(inWindow(window.m_viewer, &window), window.m_shell->rects().slot.translated(window.m_shell->pos()));
        window.close();
    }

    // While the panel closes and opens, its regions keep their edge against the
    // conversation's, which follows its own width: nothing lies over anything on
    // the way. The timeline, which only travels, is shown from a picture of it;
    // once there, it is itself again.
    void aPanelMovesWithTheConversationsEdge() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QWidget *content = window.m_chat->listContent();
        const QImage timeline = window.m_timelinePanel->grab().toImage().convertToFormat(QImage::Format_RGB32);
        const QRect open = window.m_chat->geometry();
        for (const bool opening : {false, true}) {
            window.m_panelButton->click();
            QVERIFY(window.m_shell->isMoving());
            // The timeline only travels: a picture of it is shown in its place, and it
            // is not drawn meanwhile.
            QWidget *still = window.m_shell->m_still;
            QVERIFY(still && still->isVisible());
            QVERIFY(!window.m_timelinePanel->updatesEnabled());
            // It is the timeline as it shows, and covers all it is over.
            QVERIFY(still->testAttribute(Qt::WA_OpaquePaintEvent));
            if (!opening) QCOMPARE(still->grab().toImage().convertToFormat(QImage::Format_RGB32), timeline);
            int seen = 0;
            int widths = 0;
            int last = -1;
            while (window.m_shell->isMoving()) {
                const int edge = window.m_chat->geometry().right() + 1;
                // The panel's regions start where the conversation ends, in every frame.
                QCOMPARE(window.m_viewer->geometry().left(), edge);
                QCOMPARE(window.m_timelinePanel->geometry().left(), edge);
                if (window.m_shell->m_still) QCOMPARE(window.m_shell->m_still->geometry(), window.m_timelinePanel->geometry());
                // They only travel: the picture is not scaled on the way.
                QCOMPARE(window.m_viewer->size(), window.m_shell->rects().slot.isEmpty() ? window.m_viewer->size() : window.m_shell->rects().slot.size());
                // What is said follows the width its panel has, in the same frame.
                QCOMPARE(content->width(), window.m_chat->list()->viewport()->width());
                if (edge != last) ++widths;
                last = edge;
                ++seen;
                QTest::qWait(16);
            }
            QVERIFY(seen > 2);
            QVERIFY(widths > 2);
            QVERIFY(!window.m_shell->m_still);
            QVERIFY(window.m_timelinePanel->updatesEnabled());
            QCOMPARE(window.m_viewer->isVisible(), opening);
            QCOMPARE(window.m_timelinePanel->isVisible(), opening);
            QCOMPARE(content->width(), window.m_chat->list()->viewport()->width());
            QVERIFY(window.m_chat->composerWidget()->isVisible());
        }
        // Back where it was.
        QCOMPARE(window.m_chat->geometry(), open);
        window.close();
    }

    // A pane opened by hand is there sooner than the stage takes to grow.
    void aPaneOpenedByHandIsQuick() {
        QVERIFY(tokens::motion::panel < tokens::motion::stage);
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        for (const char *command : {"view.sidebar", "view.timeline", "view.picture", "window.project"}) {
            window.triggerCommand(QString::fromLatin1(command));
            QVERIFY2(window.m_shell->isMoving(), command);
            QCOMPARE(window.m_shell->m_moves->duration(), tokens::motion::panel);
            QTRY_VERIFY_WITH_TIMEOUT(!window.m_shell->isMoving(), 3000);
        }
        window.close();
    }

    // The Media band comes down out from under the picture as the timeline
    // moves down for it, and goes back up under it: it never lies over the
    // picture, and the conversation stays as it is.
    void theMediaBandComesOutFromUnderThePicture() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ui::setReducedMotionOverride(0);
        const auto restore = qScopeGuard([] { ui::setReducedMotionOverride(-1); });
        QWidget *sheet = window.m_projectPanel;
        const QRect chat = window.m_chat->geometry();
        for (const bool opening : {true, false}) {
            window.triggerCommand(QStringLiteral("window.project"));
            QVERIFY(window.m_shell->isMoving());
            int seen = 0;
            while (window.m_shell->isMoving()) {
                QVERIFY(sheet->isVisible());
                const QRect picture = window.m_viewer->geometry();
                // Across the panel, as wide as the picture's region.
                QCOMPARE(sheet->x(), picture.left());
                QCOMPARE(sheet->width(), picture.width());
                // All of it that shows is under the picture, and the timeline
                // starts where it ends.
                const QRect shown = sheet->mask().boundingRect().translated(sheet->pos());
                QVERIFY(shown.top() >= picture.bottom() + 1 || shown.height() <= 1);
                QCOMPARE(window.m_timelinePanel->geometry().top(), sheet->geometry().bottom() + 1);
                QCOMPARE(window.m_chat->geometry(), chat);
                ++seen;
                QTest::qWait(16);
            }
            QVERIFY(seen > 2);
            QCOMPARE(sheet->isVisible(), opening);
            QVERIFY(sheet->mask().isEmpty());
            if (opening) {
                const stage::Rects r = window.m_shell->rects();
                QCOMPARE(sheet->geometry(), r.media);
                QCOMPARE(r.media.top(), r.slot.bottom() + 1);
                QCOMPARE(r.timeline.top(), r.media.bottom() + 1);
            }
        }
        window.close();
    }

    void watchingLeavesARoomOverThePicture() {
        stage::Panes panes;
        panes.reel = true;
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, panes);
        // The stage reaches over the conversation, with the strip over the picture.
        QCOMPARE(r.conversation, QRect(280, 40, 448, 960));
        QCOMPARE(r.slot, QRect(608, 40, 992, 558));
        QCOMPARE(r.reel, QRect(r.picture.left() + tokens::layout::picturePad, 64, 460, 40));
        QCOMPARE(r.timeline.left(), 608);
    }

    void thePictureFollowsKerf() {
        QSettings().remove(QStringLiteral("layout"));
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        window.handleEvent(QJsonObject{{"type", "agent_focus"}, {"tool", "look"}, {"times", QJsonArray{"2s"}}});
        QCOMPARE(window.m_timeline->agentCursor(), 2.0);
        QCOMPARE(window.m_playhead, 2.0);
        // Not following: the cursor moves, the picture stays.
        window.m_commands->action(QStringLiteral("view.followKerf"))->trigger();
        QVERIFY(!window.m_followKerf);
        QVERIFY(!window.m_followButton->isOn());
        window.handleEvent(QJsonObject{{"type", "agent_focus"}, {"tool", "look"}, {"times", QJsonArray{"3s"}}});
        QCOMPARE(window.m_timeline->agentCursor(), 3.0);
        QCOMPARE(window.m_playhead, 2.0);
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});
        QVERIFY(std::isnan(window.m_timeline->agentCursor()));
        QSettings().remove(QStringLiteral("layout"));
        window.close();
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StageShellTest)

#include "stage_shell_test.moc"
