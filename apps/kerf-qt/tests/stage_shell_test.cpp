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
#include "ui/change_reel.h"
#include "ui/compare_view.h"
#include "ui/stage_motion.h"
#include "ui/composer.h"
#include "ui/wiring/review_wiring.h"
#include "ui/inspector_column.h"
#include "ui/project_sidebar.h"
#include "ui/stage_shell.h"
#include "ui/wiring/timeline_wiring.h"
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
    for (const QRect &region : {r.sidebar, r.slot, r.transport, r.conversation, r.timeline})
        QVERIFY2(region.isEmpty() || content.contains(region), what);
    QVERIFY2(!r.slot.intersects(r.conversation), what);
    QVERIFY2(!r.slot.intersects(r.timeline), what);
    QVERIFY2(!r.slot.intersects(r.transport), what);
    QVERIFY2(!r.conversation.intersects(r.timeline), what);
    QVERIFY2(!r.transport.intersects(r.timeline), what);
    QVERIFY2(!r.sidebar.intersects(r.conversation), what);
    const int area = r.sidebar.width() * r.sidebar.height() + r.slot.width() * r.slot.height() + r.transport.width() * r.transport.height() +
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
        QCOMPARE(r.sidebar, QRect(0, 40, 240, 960));
        QCOMPARE(r.conversation, QRect(240, 40, 400, 960));
        QCOMPARE(r.panel, QRect(640, 40, 960, 960));
        QCOMPARE(r.slot, QRect(640, 40, 960, 420));
        QCOMPARE(r.picture, QRect(768, 52, 704, 396));
        QVERIFY(r.transport.isEmpty());
        QCOMPARE(r.timeline, QRect(640, 460, 960, 540));
        expectFilled(r, QSize(1600, 1000), "16:9");
    }

    // The picture over the timeline, in whatever shape.
    void verticalAtTheDesignSize() {
        for (const double aspect : {kPortrait, kFeed, 1.0}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect);
            QVERIFY(!r.wide);
            QCOMPARE(r.conversation, QRect(240, 40, 400, 960));
            QCOMPARE(r.slot, QRect(640, 40, 960, 614));
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), aspect));
            QVERIFY(r.transport.isEmpty());
            QCOMPARE(r.timeline, QRect(640, 654, 960, 346));
            expectFilled(r, QSize(1600, 1000), "vertical");
        }
    }

    void wideAtOtherWindowSizes() {
        // The least window.
        {
            const stage::Rects r = stage::rects(QSize(1080, 700), kWide);
            QVERIFY(r.sidebar.isEmpty());
            QCOMPARE(r.conversation, QRect(0, 40, 400, 660));
            QCOMPARE(r.slot, QRect(400, 40, 680, 289));
            QCOMPARE(r.timeline, QRect(400, 329, 680, 371));
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
            QCOMPARE(r.conversation.width(), 400);
            QCOMPARE(r.slot, QRect(640, 40, 1920, 612));
            QCOMPARE(r.timeline, QRect(640, 652, 1920, 788));
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
            QCOMPARE(r.conversation, QRect(240, 40, 400, 760));
            QCOMPARE(r.slot, QRect(640, 40, 640, 486));
            QCOMPARE(r.picture, stage::fitted(stage::paddedSlot(r.slot), 1.0));
            QCOMPARE(r.timeline, QRect(640, 526, 640, 274));
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

    void sheetsAndTheTrayStandOverTheRegions() {
        for (const double aspect : {kWide, kPortrait, kFeed, 1.0}) {
            const stage::Rects r = stage::rects(QSize(1600, 1000), aspect);
            const int side = r.conversation.right() + 1;
            QCOMPARE(r.media, QRect(side - 272, 40, 272, r.conversation.height()));
            QCOMPARE(r.browser, QRect(side - 304, 40, 304, r.conversation.height()));
            QCOMPARE(r.inspector, QRect(side - 304, 40, 304, r.conversation.height()));
            QCOMPARE(r.tray, QRect(240, 656, 1360, 344));
            // A sheet leaves the timeline free: a drag can reach it.
            QVERIFY(!r.media.intersects(r.timeline));
            QVERIFY(!r.browser.intersects(r.timeline));
        }
        // The conversation at its least: a sheet keeps to it.
        const stage::Rects narrow = stage::rects(QSize(800, 900), kWide);
        QVERIFY(narrow.conversation.contains(narrow.inspector));
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
            QVERIFY(!block->isVisible());
        };
        expectAt(16.0 / 9.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        // At once: no animation, nothing left to process.
        expectAt(9.0 / 16.0);
        QCOMPARE(window.m_viewer->videoRect(), QRectF(stage::fitted(stage::paddedSlot(QRect(0, 0, 960, 614)), 9.0 / 16.0)));
        window.applySnapshot(snapshotOf(QSize(1080, 1350)));
        expectAt(4.0 / 5.0);
        window.resize(1280, 800);
        QCoreApplication::processEvents();
        expectAt(4.0 / 5.0);
        window.applySnapshot(snapshotOf(QSize(1080, 1080)));
        expectAt(1.0);
        // The black is the picture's: letterboxed at this size.
        QCOMPARE(window.m_viewer->videoRect(), QRectF(stage::fitted(stage::paddedSlot(QRect(0, 0, 640, 486)), 1.0)));
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
            QCOMPARE(inWindow(window.m_chat, &window), QRect(240, 40, 400, 960));
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
            // At the conversation's panel side, at its top and of its height.
            QCOMPARE(inWindow(entry.widget, &window),
                     QRect(conversation.right() + 1 - entry.width, conversation.top(), entry.width, conversation.height()));
            QVERIFY(window.m_commands->action(QString::fromLatin1(entry.command))->isChecked());
            // Its left hairline.
            auto *seam = shell->findChild<QWidget *>(QStringLiteral("stageSeamSheet"));
            QVERIFY(seam && seam->isVisible());
            QCOMPARE(inWindow(seam, &window), QRect(conversation.right() - entry.width, conversation.top(), 1, conversation.height()));
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
        QCOMPARE(inWindow(window.m_finishTray, &window), QRect(240, 656, 1360, 344));
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
        QCOMPARE(inWindow(window.m_deliverTray, &window), QRect(240, 656, 1360, 344));
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

    void aConversationAloneHasTheRoom() {
        stage::Panes panes;
        panes.picture = false;
        panes.timeline = false;
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, panes);
        QCOMPARE(r.sidebar, QRect(0, 40, 240, 960));
        QCOMPARE(r.conversation, QRect(240, 40, 1360, 960));
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
        QCOMPARE(wide.slot, QRect(640, 40, 960, 872));
        QCOMPARE(wide.transport, QRect(640, 912, 960, 88));
        expectFilled(wide, QSize(1600, 1000), "picture");
        const stage::Rects upright = stage::rects(QSize(1600, 1000), kPortrait, 40, picture);
        QCOMPARE(upright.slot, QRect(640, 40, 960, 872));
        QCOMPARE(upright.transport, QRect(640, 912, 960, 88));
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
        QCOMPARE(stage::rects(QSize(1100, 800), kWide).sidebar.width(), 240);
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
        QCOMPARE(inWindow(window.m_chat, &window), QRect(240, 40, 1360, 960));
        // No picture to stamp or draw on.
        QVERIFY(!window.m_chat->composerWidget()->stamp()->isVisible());
        QVERIFY(!window.m_chat->composerWidget()->drawButton()->isVisible());
        QVERIFY(!window.m_chat->composerWidget()->stamped());

        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        QCoreApplication::processEvents();
        QVERIFY(window.m_viewer->isVisible());
        QVERIFY(window.m_timelinePanel->isVisible());
        QCOMPARE(inWindow(window.m_chat, &window), QRect(240, 40, 400, 960));
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

    void thePaneButtonsAndTheTransportFollowTheCommands() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QVERIFY(window.m_pictureButton->isOn());
        QVERIFY(window.m_timelineButton->isOn());
        ui::IconButton *play = window.m_timelineWiring->tools()->transportButton(TimelineToolStrip::Transport::Play);
        PictureTransport *block = window.m_timelineWiring->pictureTransport();
        QVERIFY(window.m_timelineWiring->tools()->isAncestorOf(play));
        // With the timeline closed the block under the picture has the transport.
        window.m_timelineButton->click();
        QVERIFY(!window.m_timelineButton->isOn());
        QVERIFY(!window.m_shell->panes().timeline);
        QVERIFY(block->isVisible());
        QVERIFY(block->isAncestorOf(play));
        window.m_timelineButton->click();
        QVERIFY(window.m_timelineWiring->tools()->isAncestorOf(play));
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
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t1"))->trailing(), QStringLiteral("Review"));
        QVERIFY(sidebar.conversationRow(QStringLiteral("t1"))->isCurrent());
        QCOMPARE(sidebar.conversationRow(QStringLiteral("t2"))->trailing(), QStringLiteral("Done"));
        QSignalSpy chosen(&sidebar, &ProjectSidebar::conversationChosen);
        sidebar.conversationRow(QStringLiteral("t2"))->click();
        QCOMPARE(chosen.value(0).value(0).toString(), QStringLiteral("t2"));
        QSignalSpy project(&sidebar, &ProjectSidebar::projectChosen);
        sidebar.projectRow(first.path())->click();
        QCOMPARE(project.value(0).value(0).toString(), QDir::cleanPath(QFileInfo(first.path()).absoluteFilePath()));
        sidebar.setWorking(true);
        QCOMPARE(sidebar.projectRow(second.path())->trailing(), QStringLiteral("Working"));
        sidebar.setConversationsShown(false);
        QVERIFY(!sidebar.newConversationRow());
        QVERIFY(!sidebar.conversationRow(QStringLiteral("t1")));
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
        ReviewModel::ClipChange trim;
        trim.clip = QStringLiteral("c1");
        trim.name = QStringLiteral("sea");
        trim.trimmed = true;
        trim.oldStart = 10;
        trim.oldEnd = 14;
        trim.newStart = 10;
        trim.newEnd = 12;
        ReviewModel::ClipChange near = trim;
        near.name = QStringLiteral("road");
        near.oldStart = 12;
        near.oldEnd = 16;
        near.newStart = 12;
        near.newEnd = 16;
        ReviewModel::ClipChange added;
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

    void versionCardsTakeThePicturesShape() {
        CompareView view;
        view.resize(820, 560);
        QVERIFY(view.cardRect(0, 3).height() < view.cardRect(0, 3).width() * 1.5);
        view.setUpright(true);
        const QRect upright = view.cardRect(0, 3);
        QVERIFY(upright.height() > upright.width() * 1.7);
        QVERIFY(view.rect().contains(view.cardRect(2, 3)));
    }

    void anUnusedCardKeepsItsPosterWhenTheStateChanges() {
        CompareView view;
        view.resize(820, 560);
        QImage poster(16, 9, QImage::Format_RGB32);
        poster.fill(Qt::red);
        CompareView::Version version;
        version.key = QStringLiteral("A");
        version.poster = poster;
        view.setVersions({version});
        version.poster = QImage();
        view.setVersions({version});
        QVERIFY(view.findChild<ComparePoster *>()->hasImage());
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

    void watchingLeavesARoomOverThePicture() {
        stage::Panes panes;
        panes.reel = true;
        const stage::Rects r = stage::rects(QSize(1600, 1000), kWide, 40, panes);
        // The stage reaches over the conversation, with the strip over the picture.
        QCOMPARE(r.conversation, QRect(240, 40, 400, 960));
        QCOMPARE(r.slot, QRect(520, 40, 1080, 608));
        QCOMPARE(r.reel, QRect(553, 64, 460, 40));
        QCOMPARE(r.timeline.left(), 520);
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

    void versionsRunSideBySideAndOneIsUsed() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1920, 1080)));
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QList<QJsonObject> sent;
        window.m_bus->setSender([&sent](const QJsonObject &command) {
            sent << command;
            return true;
        });
        Composer *composer = window.m_chat->composerWidget();
        composer->versionsButton()->click();
        QVERIFY(composer->versionsOn());
        composer->fill(QStringLiteral("Open stronger"));
        QVERIFY(window.m_reviewWiring->post());
        QCOMPARE(sent.last().value("type").toString(), QStringLiteral("agent_versions"));
        QCOMPARE(sent.last().value("text").toString(), QStringLiteral("Open stronger"));
        QVERIFY(!composer->versionsOn());
        QVERIFY(window.m_shell->compareShown());
        QVERIFY(window.m_compare->isVisible());
        QCOMPARE(inWindow(window.m_compare, &window), window.m_shell->rects().slot.translated(window.m_shell->pos()));
        for (const char *key : {"A", "B", "C"})
            window.handleEvent(QJsonObject{{"type", "agent_version"}, {"key", key}, {"state", "running"}});
        QCOMPARE(window.m_compare->keys(), QStringList({"A", "B", "C"}));
        QCOMPARE(window.m_compare->statusText("B"), QStringLiteral("Working"));
        QVERIFY(!window.m_compare->useButton("A")->isEnabled());
        window.handleEvent(QJsonObject{{"type", "agent_version"}, {"key", "A"}, {"state", "done"}, {"proposalId", "pa"}});
        window.handleEvent(QJsonObject{{"type", "agent_version"}, {"key", "B"}, {"state", "done"}, {"proposalId", "pb"}});
        window.handleEvent(QJsonObject{{"type", "agent_version"}, {"key", "C"}, {"state", "failed"}, {"message", "No key"}});
        QVERIFY(window.m_compare->useButton("A")->isEnabled());
        QVERIFY(!window.m_compare->useButton("C")->isEnabled());
        QCOMPARE(window.m_compare->statusText("C"), QStringLiteral("No key"));
        // Used: it is accepted, and the versions are done with.
        window.m_compare->useButton("A")->click();
        QVERIFY(!window.m_shell->compareShown());
        QVERIFY(std::any_of(sent.cbegin(), sent.cend(), [](const QJsonObject &command) {
            return command.value("proposalId").toString() == QStringLiteral("pb") && command.value("type").toString().contains(QStringLiteral("reject"));
        }));
        QCOMPARE(sent.last().value("proposalId").toString(), QStringLiteral("pa"));
        window.close();
    }

    // At a laptop's size a tray came up over the lower part of an upright picture, and a sheet over
    // the scopes it is read beside. The picture now moves into what the tray leaves, and a sheet
    // ends where the tray begins, over it as ever where they would meet. At the design's size the
    // upright picture was clear of the tray already, and stays where it is.
    void aTrayLeavesThePictureAndASheetLeavesTheTray() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(snapshotOf(QSize(1080, 1920)));
        window.resize(1389, 868);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.activateWindow();
        const QRect whole = window.m_shell->rects().picture;
        for (const char *command : {"workspace.finish", "workspace.deliver"}) {
            window.triggerCommand(QString::fromLatin1(command));
            const QRect tray = inWindow(window.m_shell->tray() == StageShell::Tray::Finish ? static_cast<QWidget *>(window.m_finishTray) : static_cast<QWidget *>(window.m_deliverTray), &window);
            QCOMPARE(tray.height(), 344);
            const QRect picture = window.m_shell->rects().picture;
            QVERIFY2(picture.bottom() + 40 < tray.top(), qPrintable(QStringLiteral("picture to %1, tray from %2").arg(picture.bottom() + 40).arg(tray.top())));
            QVERIFY(picture.height() < whole.height());
            QCOMPARE(picture.center().x(), whole.center().x());
        }
        const QJsonArray firstClips = window.m_project.value(QStringLiteral("tracks")).toArray().at(0).toObject().value(QStringLiteral("clips")).toArray();
        window.selectClip(firstClips.at(0).toObject().value(QStringLiteral("id")).toString());
        window.triggerCommand(QStringLiteral("workspace.finish"));
        window.triggerCommand(QStringLiteral("window.inspector"));
        const QRect tray = inWindow(window.m_finishTray, &window);
        const QRect sheet = inWindow(window.m_inspectorColumn, &window);
        QVERIFY2(sheet.bottom() < tray.top(), qPrintable(QStringLiteral("sheet to %1, tray from %2").arg(sheet.bottom()).arg(tray.top())));
        QVERIFY(sheet.height() > 300);
        // Closed, the picture has its room again.
        window.triggerCommand(QStringLiteral("workspace.review"));
        QCOMPARE(window.m_shell->rects().picture, whole);
        QVERIFY(inWindow(window.m_inspectorColumn, &window).height() > sheet.height());
        window.close();

        // The design's size: nothing moves.
        MainWindow large{QString(), QString()};
        large.setConnection(QStringLiteral("ready"));
        large.applySnapshot(snapshotOf(QSize(1080, 1920)));
        large.resize(1600, 1000);
        large.show();
        QVERIFY(QTest::qWaitForWindowExposed(&large));
        const QRect there = large.m_shell->rects().picture;
        large.triggerCommand(QStringLiteral("workspace.finish"));
        QCOMPARE(large.m_shell->rects().picture, there);
        large.close();
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::StageShellTest)

#include "stage_shell_test.moc"
