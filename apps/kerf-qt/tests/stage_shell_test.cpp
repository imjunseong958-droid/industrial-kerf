// Tests for the window's content: the geometry of the three regions (a pure
// function of the window's size and the picture's aspect), the sheets over the
// conversation, the trays over the bottom, and Esc.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "design_tokens.h"
#include "inspector_panel.h"
#include "main_window.h"
#include "project_panel.h"
#include "timeline_widget.h"
#include "ui/browser_panel.h"
#include "ui/components.h"
#include "ui/deliver_tray.h"
#include "ui/editor_bus.h"
#include "ui/finish_tray.h"
#include "ui/inspector_column.h"
#include "ui/stage_shell.h"
#include "ui/wiring/timeline_wiring.h"
#include "viewer_widget.h"

#include <QAbstractButton>
#include <QAction>
#include <QApplication>
#include <QJsonArray>
#include <QJsonObject>
#include <QKeySequence>
#include <QLineEdit>
#include <QSet>
#include <QSettings>
#include <QTest>

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
    for (const QRect &region : {r.slot, r.transport, r.conversation, r.timeline})
        QVERIFY2(region.isEmpty() || content.contains(region), what);
    QVERIFY2(!r.slot.intersects(r.conversation), what);
    QVERIFY2(!r.slot.intersects(r.timeline), what);
    QVERIFY2(!r.slot.intersects(r.transport), what);
    QVERIFY2(!r.conversation.intersects(r.timeline), what);
    QVERIFY2(!r.transport.intersects(r.timeline), what);
    const int area = r.slot.width() * r.slot.height() + r.transport.width() * r.transport.height() +
                     r.conversation.width() * r.conversation.height() + r.timeline.width() * r.timeline.height();
    QCOMPARE(area, content.width() * content.height());
    QVERIFY2(r.slot.isEmpty() || r.slot.contains(r.picture), what);
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
        QCOMPARE(r.slot, QRect(0, 40, 1240, 420));
        QCOMPARE(r.picture, QRect(246, 40, 747, 420));
        QVERIFY(r.transport.isEmpty());
        QCOMPARE(r.conversation, QRect(1240, 40, 360, 960));
        QCOMPARE(r.timeline, QRect(0, 460, 1240, 540));
        expectFilled(r, QSize(1600, 1000), "16:9");
    }

    // Three columns: the picture over the transport, the timeline, the conversation.
    void verticalAtTheDesignSize() {
        for (const auto &[aspect, width] : {std::pair{kPortrait, 450}, std::pair{kFeed, 640}, std::pair{1.0, 640}}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect);
            QVERIFY(!r.wide);
            QCOMPARE(r.slot, QRect(0, 40, width, 800));
            QCOMPARE(r.picture, stage::fitted(r.slot, aspect));
            QCOMPARE(r.transport, QRect(0, 840, width, 160));
            QCOMPARE(r.timeline, QRect(width, 40, 1240 - width, 960));
            QCOMPARE(r.conversation, QRect(1240, 40, 360, 960));
            expectFilled(r, QSize(1600, 1000), "vertical");
        }
    }

    void theFormatsPartAtOnePointTwo() {
        QVERIFY(stage::isWide(1.2));
        QVERIFY(!stage::isWide(1.19));
        QVERIFY(stage::rects(QSize(1600, 1000), 1.2).wide);
        QVERIFY(!stage::rects(QSize(1600, 1000), 1.19).wide);
    }

    // The conversation stays 360, then gives way down to 300; the timeline has
    // its share of the height, at least 260, and what the picture does not use.
    void wideAtOtherWindowSizes() {
        // The least window.
        {
            const stage::Rects r = stage::rects(QSize(1080, 700), kWide);
            QCOMPARE(r.conversation, QRect(720, 40, 360, 660));
            QCOMPARE(r.slot, QRect(0, 40, 720, 289));
            QCOMPARE(r.timeline, QRect(0, 329, 720, 371));
            expectFilled(r, QSize(1080, 700), "least");
        }
        // Narrow: the conversation gives way, to 300 and no further.
        {
            const stage::Rects r = stage::rects(QSize(950, 900), kWide);
            QCOMPARE(r.conversation.width(), 350);
            const stage::Rects least = stage::rects(QSize(800, 900), kWide);
            QCOMPARE(least.conversation.width(), 300);
            QCOMPARE(least.slot.width(), 500);
            expectFilled(least, QSize(800, 900), "narrow");
        }
        // Large: the picture pillarboxes in black in its slot.
        {
            const stage::Rects r = stage::rects(QSize(2560, 1440), kWide);
            QCOMPARE(r.conversation.width(), 360);
            QCOMPARE(r.slot, QRect(0, 40, 2200, 612));
            QCOMPARE(r.timeline, QRect(0, 652, 2200, 788));
            QCOMPARE(r.picture.height(), 612);
            QCOMPARE(r.picture.width(), 1088);
            QCOMPARE(r.picture.left(), (2200 - 1088) / 2);
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
            QCOMPARE(r.picture.width(), 1240);
            QCOMPARE(r.picture.height(), r.slot.height());
            QCOMPARE(r.timeline.height(), 1760 - r.slot.height());
            expectFilled(r, QSize(1600, 1800), "tall");
        }
    }

    // The timeline keeps 600 of width while the picture's column can give it
    // up, down to 240; the picture then letterboxes.
    void verticalAtOtherWindowSizes() {
        // A square picture in a narrow window.
        {
            const stage::Rects r = stage::rects(QSize(1280, 800), 1.0);
            QCOMPARE(r.conversation, QRect(920, 40, 360, 760));
            QCOMPARE(r.slot, QRect(0, 40, 320, 600));
            QCOMPARE(r.picture, QRect(0, 180, 320, 320));
            QCOMPARE(r.transport, QRect(0, 640, 320, 160));
            QCOMPARE(r.timeline, QRect(320, 40, 600, 760));
            expectFilled(r, QSize(1280, 800), "letterbox");
        }
        // Wide enough: flush.
        {
            const stage::Rects r = stage::rects(QSize(2560, 1440), kPortrait);
            QCOMPARE(r.slot, QRect(0, 40, 698, 1240));
            QCOMPARE(r.picture, r.slot);
            QCOMPARE(r.timeline, QRect(698, 40, 1502, 1400));
            expectFilled(r, QSize(2560, 1440), "large");
        }
        // Short: the column keeps its least, for the transport.
        {
            const stage::Rects r = stage::rects(QSize(1600, 500), kPortrait);
            QCOMPARE(r.slot, QRect(0, 40, 240, 300));
            QCOMPARE(r.picture.size(), QSize(169, 300));
            QCOMPARE(r.transport, QRect(0, 340, 240, 160));
            expectFilled(r, QSize(1600, 500), "short");
        }
        // Narrow: the column has its least, the timeline the rest.
        {
            const stage::Rects r = stage::rects(QSize(800, 900), kPortrait);
            QCOMPARE(r.slot.width(), 240);
            QCOMPARE(r.timeline, QRect(240, 40, 260, 860));
            QCOMPARE(r.conversation.width(), 300);
            expectFilled(r, QSize(800, 900), "narrow");
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

    void sheetsAndTheTrayStandOverTheRegions() {
        for (const double aspect : {kWide, kPortrait, kFeed, 1.0}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect);
            QCOMPARE(r.media, QRect(1600 - 272, 40, 272, r.conversation.height()));
            QCOMPARE(r.browser, QRect(1600 - 304, 40, 304, r.conversation.height()));
            QCOMPARE(r.inspector, QRect(1600 - 304, 40, 304, r.conversation.height()));
            QCOMPARE(r.tray, QRect(0, 656, 1600, 344));
            // A sheet leaves the timeline free: a drag can reach it.
            QVERIFY(!r.media.intersects(r.timeline));
            QVERIFY(!r.browser.intersects(r.timeline));
        }
        // The conversation at its least: a sheet keeps to it.
        const stage::Rects narrow = stage::rects(QSize(800, 900), kWide);
        QCOMPARE(narrow.inspector, narrow.conversation);
        QVERIFY(!narrow.inspector.intersects(narrow.timeline));
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
        QCOMPARE(strip->tools()->x(), 176);

        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        QCoreApplication::processEvents();
        QVERIFY(block->isAncestorOf(play));
        QVERIFY(play->isVisible());
        QCOMPARE(play->size(), QSize(24, 24));
        // Under the timecode, centred in the block.
        QCoreApplication::processEvents();
        const QRect inBlock(play->mapTo(block, QPoint()), play->size());
        QCOMPARE(inBlock.center().x(), block->width() / 2 - 1);
        // The strip keeps the tools, from its start, the zoom at its end.
        QCOMPARE(strip->tools()->x(), 12);
        QCOMPARE(strip->zoomInButton()->geometry().right() + 1, strip->width() - 12);
        window.m_bus->setPlayhead(2.0);
        QCOMPARE(block->time(), timelineTimecode(2.0, window.m_bus->fps()));

        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCoreApplication::processEvents();
        QVERIFY(strip->isAncestorOf(play));
        QVERIFY(play->isVisible());
        QVERIFY(!block->isVisible());
        QCoreApplication::processEvents();
        QCOMPARE(strip->tools()->x(), 176);
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
            QCOMPARE(block->isVisible(), !r.wide);
            if (!r.wide) QCOMPARE(inWindow(block, &window), r.transport);
        };
        expectAt(16.0 / 9.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        // At once: no animation, nothing left to process.
        expectAt(9.0 / 16.0);
        QCOMPARE(window.m_viewer->videoRect(), QRectF(0, 0, 450, 800));
        window.applySnapshot(snapshotOf(QSize(1080, 1350)));
        expectAt(4.0 / 5.0);
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        expectAt(4.0 / 5.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1080)));
        expectAt(1.0);
        // The black is the picture's: letterboxed at this size.
        QCOMPARE(window.m_viewer->videoRect(), QRectF(0, 140, 320, 320));
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
            QCOMPARE(window.m_shell->tray(), StageShell::Tray::None);
            QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
            QCOMPARE(window.m_bus->workspace(), QStringLiteral("Review"));
            QVERIFY(window.m_chat->isVisible());
            QCOMPARE(inWindow(window.m_chat, &window), QRect(1240, 40, 360, 960));
            window.close();
        }
        settings.remove(QStringLiteral("workspace"));
        settings.remove(QStringLiteral("viewer"));
        settings.remove(QStringLiteral("editor/inspectorFollowsSelection"));
    }

    void sheetsOpenOneAtATimeFromTheirCommands() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        StageShell *shell = window.m_shell;
        QCOMPARE(shell->sheet(), StageShell::Sheet::None);
        for (QWidget *closed : {static_cast<QWidget *>(window.m_projectPanel), static_cast<QWidget *>(window.m_browserPanel),
                                static_cast<QWidget *>(window.m_inspectorColumn)})
            QVERIFY(closed->isHidden());
        const struct {
            const char *command;
            StageShell::Sheet sheet;
            QWidget *widget;
            int width;
            const char *close;
        } sheets[] = {{"window.project", StageShell::Sheet::Media, window.m_projectPanel, 272, "sheetClose.media"},
                      {"window.effects", StageShell::Sheet::Browser, window.m_browserPanel, 304, "sheetClose.browser"},
                      {"window.inspector", StageShell::Sheet::Inspector, window.m_inspectorColumn, 304, "sheetClose.inspector"}};
        const QRect conversation = inWindow(window.m_chat, &window);
        for (const auto &entry : sheets) {
            window.triggerCommand(QString::fromLatin1(entry.command));
            QCOMPARE(shell->sheet(), entry.sheet);
            QVERIFY(entry.widget->isVisible());
            // Anchored to the window's right edge, at the conversation's top
            // and of its height.
            QCOMPARE(inWindow(entry.widget, &window), QRect(1600 - entry.width, conversation.top(), entry.width, conversation.height()));
            QVERIFY(window.m_commands->action(QString::fromLatin1(entry.command))->isChecked());
            // Its left hairline.
            auto *seam = shell->findChild<QWidget *>(QStringLiteral("stageSeamSheet"));
            QVERIFY(seam && seam->isVisible());
            QCOMPARE(inWindow(seam, &window), QRect(1600 - entry.width - 1, conversation.top(), 1, conversation.height()));
            // One at a time.
            int shown = 0;
            for (const auto &other : sheets)
                if (other.widget->isVisible()) ++shown;
            QCOMPARE(shown, 1);
            // The conversation stays where it is, under it.
            QCOMPARE(inWindow(window.m_chat, &window), conversation);
            // The same command closes it.
            window.triggerCommand(QString::fromLatin1(entry.command));
            QCOMPARE(shell->sheet(), StageShell::Sheet::None);
            QVERIFY(!entry.widget->isVisible());
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
        // Another sheet's command takes over.
        window.triggerCommand(QStringLiteral("window.project"));
        window.triggerCommand(QStringLiteral("window.inspector"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Inspector);
        QVERIFY(!window.m_commands->action(QStringLiteral("window.project"))->isChecked());
        // Presets is the browser's page, Source the Inspector's.
        window.triggerCommand(QStringLiteral("window.presets"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Browser);
        QCOMPARE(window.m_browserPanel->page(), BrowserPanel::Page::Presets);
        QVERIFY(window.m_commands->action(QStringLiteral("window.presets"))->isChecked());
        QVERIFY(!window.m_commands->action(QStringLiteral("window.effects"))->isChecked());
        window.triggerCommand(QStringLiteral("window.effects"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Browser);
        QCOMPARE(window.m_browserPanel->page(), BrowserPanel::Page::Effects);
        window.triggerCommand(QStringLiteral("window.source"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Inspector);
        QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Source);
        window.triggerCommand(QStringLiteral("window.source"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::None);
        // The Inspector's kinds open it.
        window.triggerCommand(QStringLiteral("inspector.audio"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Inspector);
        QCOMPARE(window.m_inspectorColumn->kind(), InspectorColumn::Kind::Audio);
        window.triggerCommand(QStringLiteral("inspector.video"));
        window.m_shell->setSheet(StageShell::Sheet::None);
        window.close();
    }

    // Selecting a clip opens nothing by itself.
    // ⌘4 over the Source page turns to the Inspector, as its menu row (not
    // checked there) says; on its own page it closes.
    // The Inspector's tips and its ⋯ menu name the registry's keys, on every
    // platform and in every keymap.
    void theInspectorsTipsNameTheRegistrysKeys() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 10}, {"timeline", QJsonObject{{"fps", 25}, {"tracks", QJsonArray{}}}}});
        const auto key = [&window](const char *id) {
            return window.m_commands->action(QString::fromLatin1(id))->shortcut().toString(QKeySequence::NativeText);
        };
        auto *pages = window.m_inspectorColumn->findChild<ui::SegmentedControl *>(QStringLiteral("inspectorPages"));
        auto *kinds = window.m_inspectorColumn->findChild<ui::SegmentedControl *>(QStringLiteral("inspectorKinds"));
        QVERIFY(pages && kinds);
        QVERIFY(!key("inspector.video").isEmpty());
        QCOMPARE(ui::tipShortcut(pages->segment(0)), key("window.inspector"));
        QCOMPARE(ui::tipShortcut(kinds->segment(0)), key("inspector.video"));
        QCOMPARE(ui::tipShortcut(kinds->segment(1)), key("inspector.audio"));
        QCOMPARE(ui::tipShortcut(kinds->segment(2)), key("inspector.info"));
        QCOMPARE(ui::tipName(kinds->segment(2)), QStringLiteral("Info"));
        QAction *video = window.m_commands->action(QStringLiteral("inspector.video"));
        video->setShortcut(Qt::Key_F9);
        emit window.m_commands->shortcutsChanged();
        QCOMPARE(ui::tipShortcut(kinds->segment(0)), QKeySequence(Qt::Key_F9).toString(QKeySequence::NativeText));
        auto *ask = window.m_inspector->findChild<QAction *>(QStringLiteral("inspectorAsk"));
        QVERIFY(ask);
        QCOMPARE(ui::menuKey(ask), key("ai.ask"));
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
        if (CommandRegistry::isMac()) QVERIFY(moved >= 3);
    }

    void theInspectorCommandTurnsFromSource() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.triggerCommand(QStringLiteral("window.source"));
        QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Source);
        QAction *inspector = window.m_commands->action(QStringLiteral("window.inspector"));
        QVERIFY(!inspector->isChecked());
        window.triggerCommand(QStringLiteral("window.inspector"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Inspector);
        QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Inspector);
        QVERIFY(inspector->isChecked());
        window.triggerCommand(QStringLiteral("window.inspector"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        QVERIFY(!inspector->isChecked());
    }

    // Ask Kerf and ⇧⌘J: what lies over the composer goes, so the text lands
    // where it can be seen. At the bottom of the full-height conversation, a tray
    // reaches it in every format.
    void askingUncoversTheComposer() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.resize(1600, 900);
        window.applySnapshot(QJsonObject{{"revision", 1}, {"duration", 30},
                                         {"timeline", QJsonObject{{"width", 1920}, {"height", 1080}, {"fps", 24}, {"tracks", QJsonArray{}}}}});
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.triggerCommand(QStringLiteral("workspace.finish"));
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Finish);
        emit window.m_bus->askPrefill(QStringLiteral("@Kerf fix it"), QString());
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::None);
        QCOMPARE(window.m_chatInput->toPlainText(), QStringLiteral("@Kerf fix it"));
        window.triggerCommand(QStringLiteral("window.inspector"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Inspector);
        window.triggerCommand(QStringLiteral("window.assistant"));
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        window.applySnapshot(QJsonObject{{"revision", 2}, {"duration", 30},
                                         {"timeline", QJsonObject{{"width", 1080}, {"height", 1920}, {"fps", 24}, {"tracks", QJsonArray{}}}}});
        window.triggerCommand(QStringLiteral("workspace.finish"));
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Finish);
        emit window.m_bus->askPrefill(QStringLiteral("@Kerf fix it"), QString());
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::None);
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
        // With the Inspector open on Source, a click leaves it there.
        window.triggerCommand(QStringLiteral("window.source"));
        emit window.m_timeline->clipSelected(clip);
        QCOMPARE(window.m_inspectorColumn->page(), InspectorColumn::Page::Source);
        window.close();
    }

    void traysLieOverTheBottomAndEscGoesBack() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.activateWindow();
        StageShell *shell = window.m_shell;
        const QRect picture = inWindow(window.m_viewer, &window);
        const QRect conversation = inWindow(window.m_chat, &window);
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.review"))->isChecked());
        window.triggerCommand(QStringLiteral("workspace.finish"));
        QCOMPARE(shell->tray(), StageShell::Tray::Finish);
        QCOMPARE(window.m_bus->workspace(), QStringLiteral("Finish"));
        QVERIFY(window.m_finishTray->isVisible());
        QCOMPARE(inWindow(window.m_finishTray, &window), QRect(0, 656, 1600, 344));
        // Picture and conversation stay where they are underneath.
        QCOMPARE(inWindow(window.m_viewer, &window), picture);
        QCOMPARE(inWindow(window.m_chat, &window), conversation);
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.finish"))->isChecked());
        QVERIFY(!window.m_commands->action(QStringLiteral("workspace.review"))->isChecked());
        // Colour wheels stay in the Inspector sheet while Finish is open.
        QVERIFY(window.m_inspector->finish());
        const QJsonArray firstClips = window.m_project.value(QStringLiteral("tracks")).toArray().at(0).toObject().value(QStringLiteral("clips")).toArray();
        window.selectClip(firstClips.at(0).toObject().value(QStringLiteral("id")).toString());
        window.triggerCommand(QStringLiteral("window.inspector"));
        QCOMPARE(shell->sheet(), StageShell::Sheet::Inspector);
        QCOMPARE(shell->tray(), StageShell::Tray::Finish);
        QVERIFY(window.m_inspector->wheels());
        // Over the tray.
        const QWidgetList order = shell->findChildren<QWidget *>(QString(), Qt::FindDirectChildrenOnly);
        QVERIFY(order.indexOf(window.m_inspectorColumn) > order.indexOf(window.m_finishTray));
        // Esc closes the sheet, then the tray.
        window.m_timeline->setFocus();
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(shell->sheet(), StageShell::Sheet::None);
        QCOMPARE(shell->tray(), StageShell::Tray::Finish);
        QTest::keyClick(window.m_timeline, Qt::Key_Escape);
        QCOMPARE(shell->tray(), StageShell::Tray::None);
        QCOMPARE(window.m_bus->workspace(), QStringLiteral("Review"));
        QVERIFY(!window.m_finishTray->isVisible());
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.review"))->isChecked());
        // Deliver, the same; choosing it again goes back.
        window.triggerCommand(QStringLiteral("workspace.deliver"));
        QCOMPARE(shell->tray(), StageShell::Tray::Deliver);
        QCOMPARE(inWindow(window.m_deliverTray, &window), QRect(0, 656, 1600, 344));
        window.triggerCommand(QStringLiteral("workspace.deliver"));
        QCOMPARE(shell->tray(), StageShell::Tray::None);
        // One tray takes over from the other.
        window.triggerCommand(QStringLiteral("workspace.finish"));
        window.triggerCommand(QStringLiteral("workspace.deliver"));
        QCOMPARE(shell->tray(), StageShell::Tray::Deliver);
        QVERIFY(!window.m_finishTray->isVisible());
        window.triggerCommand(QStringLiteral("workspace.review"));
        QCOMPARE(shell->tray(), StageShell::Tray::None);
        // The launch flag.
        window.openTray(QStringLiteral("Finish"));
        QCOMPARE(shell->tray(), StageShell::Tray::Finish);
        window.openTray(QStringLiteral("Review"));
        QCOMPARE(shell->tray(), StageShell::Tray::None);
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

    // Drag from Media and Effects to the timeline keeps working: the timeline
    // takes drops and no sheet lies over it.
    void aSheetLeavesTheTimelineOpenToDrops() {
        for (const QSize &canvas : {QSize(1920, 1080), QSize(1080, 1920)}) {
            MainWindow window{QString(), QString()};
            window.setConnection(QStringLiteral("ready"));
            window.applySnapshot(snapshotOf(canvas));
            window.resize(1600, 1000);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QVERIFY(window.m_timeline->acceptDrops());
            for (const char *command : {"window.project", "window.effects"}) {
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
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StageShellTest)

#include "stage_shell_test.moc"
