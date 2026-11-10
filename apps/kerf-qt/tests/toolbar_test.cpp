// Tests for the toolbar (mockup-spec 1, 11, 14): the workspaces, the
// centred title, the format segments, the sequence button, the Export split
// button and its preflight.

#include "fixtures.h"
#include "test_registry.h"

#include "chat_panel.h"
#include "command_registry.h"
#include "composer.h"
#include "components.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "project_panel.h"
#include "main_window.h"
#include "render_queue_view.h"
#include "review_model.h"
#include "sequence_settings.h"
#include "timeline_widget.h"
#include "toolbar_widgets.h"
#include "wiring/toolbar_wiring.h"
#include "ui/stage_shell.h"

#include <QAbstractButton>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QComboBox>
#include <QLabel>
#include <QLineEdit>
#include <QScopeGuard>
#include <QCoreApplication>
#include <QDir>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QMenu>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <QToolBar>
#include <QToolButton>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace test;

namespace {

QAction *findAction(QMenu *menu, const QString &name) {
    for (QAction *action : menu->actions())
        if (action->objectName() == name) return action;
    return nullptr;
}

QStringList menuTexts(QMenu *menu) {
    QStringList items;
    for (const QAction *action : menu->actions()) items << (action->isSeparator() ? QStringLiteral("-") : action->text());
    return items;
}

// A finished check, as the bridge sends it: the first rows of FULL_ROWS.
QJsonObject checkFixture() {
    const auto row = [](const char *code, const char *name, bool passed, const char *value, const char *limit = nullptr,
                        const char *comment = nullptr) {
        QJsonObject out{{"code", code}, {"name", name}, {"passed", passed}, {"value", value}, {"count", passed ? 0 : 1},
                        {"blocking", passed ? 0 : 1}};
        if (limit) out.insert("limit", limit);
        if (comment) out.insert("commentId", comment);
        return out;
    };
    const QJsonArray rows{
        row("loudness.integrated", "Integrated loudness", true, "−23.1 LUFS", "R128"),
        row("loudness.true_peak", "True peak", false, "−0.4 dBTP", "≤ −1 dBTP", "c-2718"),
        row("loudness.range", "Loudness range", true, "6.2 LU"),
        row("picture.gamut", "Gamut Rec.709", true, "legal"),
        row("picture.levels", "Luma levels", true, "0 outside"),
        row("text.safe_area", "Title safe 90%", true, "0 outside"),
        row("text.overlap", "Caption overlap", false, "1 at 00:00:17:08", nullptr, "c-1708"),
        row("picture.flash_black", "Flash and black", true, "0 found")};
    return QJsonObject{{"type", "check"}, {"status", "done"}, {"valid", false}, {"findings", QJsonArray{}},
                       {"summary", QJsonObject{{"passed", 6}, {"total", 8}, {"blocking", 2}, {"rows", rows}}}};
}

QJsonObject comment(const char *id, const char *parent, bool blocking, bool resolved) {
    return QJsonObject{{"id", id}, {"parent", parent ? QJsonValue(QString::fromLatin1(parent)) : QJsonValue()},
                       {"author", QJsonObject{{"kind", "person"}, {"name", "Mina Park"}, {"id", "mina@example.com"}}},
                       {"at", 17.32}, {"duration", QJsonValue()}, {"frame", 433}, {"body", "Caption sits on the lower third"},
                       {"tags", QJsonArray{}}, {"blocking", blocking}, {"drawing", QJsonArray{}}, {"proposal", QJsonValue()},
                       {"steps", QJsonArray{}}, {"error", false}, {"resolved", resolved}, {"resolvedBy", QJsonValue()},
                       {"resolvedRevision", QJsonValue()}, {"qcKey", QJsonValue()}, {"created", "2026-11-04T10:12:00Z"},
                       {"revision", 22}, {"edited", QJsonValue()}};
}

// The showcase snapshot named as the mockup's project, with its sequence and
// the time of its revision.
QJsonObject titledSnapshot(const QString &revisionTime) {
    QJsonObject snapshot = showcaseSnapshot(22, QStringLiteral("clean"));
    QJsonObject timeline = snapshot.value("timeline").toObject();
    timeline.insert("name", "Atlas");
    timeline.insert("sequence", "seq-1");
    timeline.insert("sequences", QJsonArray{QJsonObject{{"id", "seq-1"}, {"name", "30 s spot v7"}, {"width", 1920}, {"height", 1080},
                                                        {"rate", "25"}, {"fps", 25}}});
    timeline.insert("fps", 25);
    snapshot.insert("timeline", timeline);
    snapshot.insert("revisionTime", revisionTime);
    return snapshot;
}

// Keeps the workspace settings a test changes from reaching the next test:
// it starts from the built-ins alone and puts everything back afterwards.
struct WorkspaceSettingsGuard {
    QHash<QString, QVariant> saved;
    WorkspaceSettingsGuard() {
        QSettings settings;
        for (const QString &key : settings.allKeys())
            if (key.startsWith(QStringLiteral("workspace/"))) saved.insert(key, settings.value(key));
        settings.remove(QStringLiteral("workspace/names"));
        settings.setValue(QStringLiteral("workspace/current"), QStringLiteral("Review"));
    }
    ~WorkspaceSettingsGuard() {
        QSettings settings;
        for (const QString &key : settings.allKeys())
            if (key.startsWith(QStringLiteral("workspace/"))) settings.remove(key);
        for (auto it = saved.cbegin(); it != saved.cend(); ++it) settings.setValue(it.key(), it.value());
    }
};

} // namespace

class ToolbarTest final : public QObject {
    Q_OBJECT

private slots:
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void toolbarHoldsOnlyTheMenuTheFormatsAndExport() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QToolBar *toolbar = window.m_toolbar;
        QCOMPARE(toolbar->height(), 40);
        // One 28 pt ≡ button, then the two format icons and the Export split.
        auto *menuButton = window.findChild<ui::IconButton *>(QStringLiteral("appMenu"));
        QVERIFY(menuButton);
        QCOMPARE(menuButton, wiring->appMenuButton());
        QCOMPARE(menuButton->size(), QSize(28, 28));
        QCOMPARE(window.m_formatBank->count(), 2);
        for (const char *gone : {"toolbar.file.import", "toolbar.edit.undo", "toolbar.edit.redo", "toolbar.clip.split", "toolbar.clip.delete",
                                 "workspaceSegments", "sequenceButton", "settingsButton", "applicationMenuButton", "windowViewerMenu"})
            QVERIFY2(!window.findChild<QWidget *>(QString::fromLatin1(gone)), gone);
        // Their commands, keys and native menu items stay.
        for (const char *id : {"edit.undo", "edit.redo", "clip.split", "clip.delete", "file.import", "sequence.settings", "app.preferences"}) {
            QAction *action = window.m_commands->action(QString::fromLatin1(id));
            QVERIFY2(action, id);
            QVERIFY2(!action->shortcut().isEmpty(), id);
        }
        QVERIFY(!window.m_commands->action(QStringLiteral("workspace.edit")));
        window.close();
    }

    // The ≡ menu: the mockup's rows, the registry's commands behind each.
    void theAppMenuReachesEverythingElse() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z")));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        wiring->buildAppMenu();
        QMenu *menu = wiring->appMenu();
        QStringList rows;
        for (QAction *action : menu->actions()) rows << (action->isSeparator() ? QStringLiteral("|") : action->text());
        const QStringList expected{QStringLiteral("Media"), QStringLiteral("Effects and titles"), QStringLiteral("Inspector"), QStringLiteral("|"),
                                   QStringLiteral("Sidebar"), QStringLiteral("Picture"), QStringLiteral("Timeline"), QStringLiteral("Follow Kerf"), QStringLiteral("|"),
                                   QStringLiteral("Scopes, loudness, QC"), QStringLiteral("Deliver"), QStringLiteral("|"),
                                   QStringLiteral("Import…"), QStringLiteral("Sequence settings…"), QStringLiteral("Settings…"), QStringLiteral("View")};
        QCOMPARE(rows.mid(0, expected.size()), expected);
        // Keys as the registry has them, never written here.
        const auto keyOf = [&window](const char *id) { return window.m_commands->action(QString::fromLatin1(id))->shortcut().toString(QKeySequence::NativeText); };
        QCOMPARE(ui::menuKey(findAction(menu, QStringLiteral("appMenu.media"))), keyOf("window.project"));
        QCOMPARE(ui::menuKey(findAction(menu, QStringLiteral("appMenu.effects"))), keyOf("window.effects"));
        QCOMPARE(ui::menuKey(findAction(menu, QStringLiteral("appMenu.inspector"))), keyOf("window.inspector"));
        // Each row runs its command.
        findAction(menu, QStringLiteral("appMenu.media"))->trigger();
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::Media);
        wiring->buildAppMenu();
        QVERIFY(findAction(menu, QStringLiteral("appMenu.media"))->isChecked());
        findAction(menu, QStringLiteral("appMenu.media"))->trigger();
        QCOMPARE(window.m_shell->sheet(), StageShell::Sheet::None);
        wiring->buildAppMenu();
        findAction(menu, QStringLiteral("appMenu.finish"))->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Finish);
        QCOMPARE(window.m_bus->workspace(), QStringLiteral("Finish"));
        wiring->buildAppMenu();
        findAction(menu, QStringLiteral("appMenu.deliver"))->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Deliver);
        window.m_shell->setTray(StageShell::Tray::None);
        // View: the masks, annotations, safe zones, zoom, keyboard and sync.
        wiring->buildAppMenu();
        QMenu *view = findAction(menu, QStringLiteral("appMenu.view"))->menu();
        QVERIFY(view);
        QStringList names;
        for (QAction *action : view->actions())
            if (!action->isSeparator()) names << action->text();
        for (const QAction *tool : std::as_const(window.m_maskTools)) QVERIFY2(names.contains(tool->text()), qPrintable(tool->text()));
        QCOMPARE(window.m_maskTools.size(), 3);
        for (const char *row : {"Annotations", "Safe Zones", "Viewer zoom", "Keyboard", "Sync"})
            QVERIFY2(names.contains(QString::fromLatin1(row)), row);
        // The keyboard's choice, and the sync's rows.
        QMenu *keymaps = view->findChild<QMenu *>(QStringLiteral("keymapMenu"));
        QVERIFY(keymaps);
        QVERIFY(findAction(keymaps, QStringLiteral("keymapChoice.premiere")));
        findAction(keymaps, QStringLiteral("keymapChoice.premiere"))->trigger();
        QCOMPARE(window.m_commands->keymap(), QStringLiteral("premiere"));
        window.m_commands->setKeymap(QStringLiteral("kerf"));
        QSettings().remove(QStringLiteral("commands/keymap"));
        QMenu *sync = view->findChild<QMenu *>(QStringLiteral("syncMenu"));
        QVERIFY(sync);
        QVERIFY(sync->actions().contains(window.m_commands->action(QStringLiteral("collab.publish"))));
    }

    // The format segments: one per preset, 28 wide icons; a click sends the
    // set-sequence edit through sequence.setFormat; ⌥1-2 do the same; a
    // custom size turns none on and the tooltip names it; offline, the
    // click goes back to the project's format.
    void toolbarFormatSegmentsSendTheCanvasOp() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        FormatBank *bank = window.m_formatBank;
        const QList<CanvasPreset> presets = canvasPresets();
        QCOMPARE(bank->count(), int(presets.size()));
        QCOMPARE(bank->height(), 28);
        for (int i = 0; i < bank->count(); ++i) {
            QCOMPARE(bank->segment(i)->width(), 28);
            QCOMPARE(bank->segment(i)->height(), 24);
            QCOMPARE(bank->format(i), QSize(presets.at(i).width, presets.at(i).height));
        }
        QCOMPARE(ui::tipName(bank->segment(0)), QStringLiteral("Landscape 16:9 · 1920×1080"));
        QCOMPARE(ui::tipName(bank->segment(1)), QStringLiteral("Vertical 9:16 · 1080×1920"));
        QCOMPARE(ui::tipShortcut(bank->segment(1)), QKeySequence(Qt::ALT | Qt::Key_2).toString(QKeySequence::NativeText));

        // Offline the click does not hold: the project's format stays on.
        window.applySnapshot(showcaseSnapshot(3, QStringLiteral("clean")));
        QCOMPARE(bank->currentIndex(), 0);
        QTest::mouseClick(bank->segment(1), Qt::LeftButton);
        QCOMPARE(bank->currentIndex(), 0);

        window.setConnection(QStringLiteral("ready"));
        window.m_lastCommand = {};
        QTest::mouseClick(bank->segment(1), Qt::LeftButton);
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(op.value("width").toInt(), 1080);
        QCOMPARE(op.value("height").toInt(), 1920);
        // No bridge took the edit here, so the project's format comes back on;
        // the snapshot that follows an edit turns its own on.
        QCOMPARE(bank->currentIndex(), 0);
        // The current one asks nothing.
        window.m_lastCommand = {};
        QTest::mouseClick(bank->segment(0), Qt::LeftButton);
        QVERIFY(window.m_lastCommand.isEmpty());

        // ⌥2 is Vertical: the registry's sequence.setFormat.916, which the
        // keymap owns (no window action of the toolbar's own is left).
        QAction *vertical = window.m_commands->action(QStringLiteral("sequence.setFormat.916"));
        QVERIFY(vertical);
        QCOMPARE(vertical->shortcut(), QKeySequence(Qt::ALT | Qt::Key_2));
        for (QAction *action : window.actions()) QVERIFY(!action->objectName().startsWith(QStringLiteral("format.")));
        QCOMPARE(window.m_toolbarWiring->m_formatActions.size(), 2);
        vertical->trigger();
        const QJsonObject verticalOp = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(QSize(verticalOp.value("width").toInt(), verticalOp.value("height").toInt()), QSize(1080, 1920));

        // A size no preset has: none on, the tooltip names it.
        window.syncCanvasPreset(QSize(1280, 720));
        QCOMPARE(bank->currentIndex(), -1);
        QVERIFY(bank->fillRect().isEmpty());
        QCOMPARE(bank->toolTip(), QStringLiteral("Custom · 1280 × 720"));
        window.syncCanvasPreset(QSize(1080, 1920));
        QCOMPARE(bank->currentIndex(), 1);

        // Disconnected, the segments are disabled.
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(!bank->isEnabled());
    }

    // ⌘0 opens the dialog from anywhere, and ≡ ▸ Sequence settings… does.
    void sequenceSettingsOpensFromTheMenu() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z")));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        wiring->buildAppMenu();
        findAction(wiring->appMenu(), QStringLiteral("appMenu.sequenceSettings"))->trigger();
        QVERIFY(wiring->sequenceSettings());
        QVERIFY(wiring->sequenceSettings()->isVisible());
        wiring->sequenceSettings()->reject();
    }

    // The program of a 25 fps sequence "main", empty or with a clip.
    static QJsonObject sequenceSnapshot(int revision, bool withClip, const QString &name = QStringLiteral("Spot"),
                                        const QString &rate = QStringLiteral("25"), double fps = 25, double start = 0) {
        QJsonArray clips;
        if (withClip)
            clips.append(QJsonObject{{"id", "c1"}, {"kind", "media"}, {"sourceId", "m1"}, {"timelineStart", 0.0}, {"duration", 2.0}});
        return QJsonObject{{"revision", revision}, {"duration", 4}, {"timeline", QJsonObject{
            {"fps", fps}, {"rate", rate}, {"sequence", "main"}, {"name", "Atlas"}, {"width", 1920}, {"height", 1080}, {"start", start},
            {"sequences", QJsonArray{QJsonObject{{"id", "main"}, {"name", name}, {"width", 1920}, {"height", 1080}, {"rate", rate}, {"fps", fps}}}},
            {"markers", QJsonArray{}},
            {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", clips}}}}}}};
    }

    // Sequence Settings… (⌘0) is the registry's sequence.settings: a dialog
    // whose Apply sends set-sequence with what changed; kerfd's refusal
    // shows in it and the next revision closes it.
    void sequenceSettingsSendsSetSequence() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(sequenceSnapshot(3, false));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QAction *settings = window.m_commands->action(QStringLiteral("sequence.settings"));
        QVERIFY(settings && settings->isEnabled());
        QCOMPARE(settings->shortcut(), QKeySequence(Qt::CTRL | Qt::Key_0));
        QVERIFY(window.m_commands->reason(QStringLiteral("sequence.settings")).isEmpty());
        QVERIFY(window.bus()->run(QStringLiteral("sequence.settings")));
        SequenceSettingsDialog *dialog = wiring->sequenceSettings();
        QVERIFY(dialog);
        QVERIFY(dialog->isVisible());
        QCOMPARE(dialog->nameField()->text(), QStringLiteral("Spot"));
        QCOMPARE(dialog->formatControl()->currentIndex(), 0);
        QCOMPARE(dialog->startField()->text(), QStringLiteral("00:00:00:00"));
        // An empty sequence can take a new rate.
        QVERIFY(dialog->rateEditable());
        QVERIFY(dialog->rateField()->isEnabled());
        QVERIFY(!dialog->rateNoteLabel()->isVisibleTo(dialog));
        dialog->nameField()->setText(QStringLiteral("Spot v2"));
        dialog->rateField()->setCurrentIndex(dialog->rateField()->findData(QStringLiteral("30")));
        dialog->startField()->setText(QStringLiteral("01:00:00:00"));
        window.m_lastCommand = {};
        dialog->applyButton()->click();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        QCOMPARE(window.m_lastCommand.value("ops").toArray(),
                 (QJsonArray{QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"name", "Spot v2"},
                                         {"rate", QJsonObject{{"num", 30}, {"den", 1}}}, {"start", "01:00:00:00"}}}));
        // No bridge took it here, and the dialog says so.
        QVERIFY(dialog->error().contains(QStringLiteral("not connected")));
        // kerfd's refusal, in its words, as it comes for a sent edit.
        wiring->m_sequencePending = true;
        wiring->m_sequenceRevision = 3;
        window.handleEvent(QJsonObject{{"type", "error"}, {"message", SequenceSettingsDialog::rateNote()}});
        QCOMPARE(dialog->error(), SequenceSettingsDialog::rateNote());
        QVERIFY(dialog->isVisible());
        // A newer revision that does not hold the change (someone else's
        // edit) leaves the request waiting and the dialog open, typed
        // fields as they were.
        wiring->m_sequencePending = true;
        window.handleEvent(QJsonObject{{"type", "state"}, {"snapshot", sequenceSnapshot(4, false)}});
        QVERIFY(wiring->m_sequencePending);
        QVERIFY(dialog->isVisible());
        QCOMPARE(dialog->nameField()->text(), QStringLiteral("Spot v2"));
        // The revision whose sequence says what was asked is kerfd taking
        // it: the dialog closes. kerfd writes the rate back as "30".
        window.handleEvent(QJsonObject{{"type", "state"}, {"snapshot", sequenceSnapshot(5, false, QStringLiteral("Spot v2"), QStringLiteral("30"), 30, 3600.0)}});
        QTRY_VERIFY(!wiring->sequenceSettings() || !wiring->sequenceSettings()->isVisible());
        QVERIFY(!wiring->m_sequencePending);
        // holds() on its own: an NTSC rate as kerfd writes it is the
        // fraction sent.
        QVERIFY(SequenceSettingsDialog::holds(sequenceSnapshot(6, false, QStringLiteral("Spot"), QStringLiteral("29.97"), 29.97).value("timeline").toObject(),
                                              QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"rate", QJsonObject{{"num", 30000}, {"den", 1001}}}}));
        QVERIFY(!SequenceSettingsDialog::holds(sequenceSnapshot(6, false).value("timeline").toObject(),
                                               QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"name", "Other"}}));

        // With a clip in it, the rate is read-only and says why.
        window.applySnapshot(sequenceSnapshot(6, true));
        SequenceSettingsDialog *again = wiring->openSequenceSettings();
        QVERIFY(!again->rateEditable());
        QVERIFY(!again->rateField()->isEnabled());
        QVERIFY(again->rateNoteLabel()->isVisibleTo(again));
        QCOMPARE(again->rateNoteLabel()->text(), QStringLiteral("Frame rate can change only while the sequence is empty"));
        // A format from the segments sends the size only.
        again->formatControl()->setCurrentIndex(1, false);
        emit again->formatControl()->activated(1);
        window.m_lastCommand = {};
        again->apply();
        QCOMPARE(window.m_lastCommand.value("ops").toArray(),
                 (QJsonArray{QJsonObject{{"op", "set-sequence"}, {"sequence", "main"}, {"width", 1080}, {"height", 1920}}}));
        again->reject();
    }

    // composer.draw (D) is the composer's Draw, while the composer has the
    // focus; elsewhere it does nothing.
    void composerDrawIsTheRegistrysCommand() {
        QSettings().remove(QStringLiteral("workspace"));
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        window.activateWindow();
        QVERIFY(QTest::qWaitForWindowActive(&window));
        Composer *composer = window.m_chat->composerWidget();
        QAction *draw = window.m_commands->action(QStringLiteral("composer.draw"));
        QVERIFY(draw);
        QCOMPARE(draw->shortcut(), QKeySequence(Qt::Key_D));
        QCOMPARE(composer->drawAction(), draw);
        QVERIFY(composer->actions().contains(draw));
        QCOMPARE(draw->shortcutContext(), Qt::WidgetWithChildrenShortcut);
        QVERIFY(window.m_commandHandlers.contains(QStringLiteral("composer.draw")));
        QCOMPARE(ui::tipShortcut(composer->drawButton()), QStringLiteral("D"));
        QSignalSpy clicked(composer, &Composer::drawClicked);
        window.m_timeline->setFocus();
        QVERIFY(!window.m_toolbarWiring->drawInComposer());
        QCOMPARE(clicked.size(), 0);
        composer->drawButton()->setFocus(Qt::TabFocusReason);
        QTRY_VERIFY(composer->hasFocusWithin());
        draw->trigger();
        QCOMPARE(clicked.size(), 1);
    }

    // Export is a split button: "Export  Master" exports the chosen quality
    // (export {master, prores_hq} or {review, h264}); the chevron opens the
    // menu: Master and Draft, Render preview, Preflight, Render master, the
    // EDL locators choice and Open Deliver workspace.
    void toolbarExportSplitButton() {
        MainWindow window{QString(), QString()};
        window.resize(1366, 768);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ExportButton *button = window.m_exportButton;
        QCOMPARE(button->height(), ExportButton::kHeight);
        QCOMPARE(button->menuPart()->width(), ExportButton::kArrowWidth);
        QCOMPARE(button->menuPart()->geometry().right(), button->width() - 1);
        QCOMPARE(button->mainPart()->accessibleName(), QStringLiteral("Export"));
        QCOMPARE(button->menuPart()->accessibleName(), QStringLiteral("Export options"));
        QCOMPARE(button->qualityName(), QStringLiteral("Master"));
        QCOMPARE(button->exportQuality(), QStringLiteral("master"));
        QCOMPARE(button->codec(), QStringLiteral("prores_hq"));
        QCOMPARE(ui::tipName(button->menuPart()), QStringLiteral("Export options"));
        // Offline it says why it cannot export.
        QVERIFY(!button->mainPart()->isEnabled());
        QVERIFY(ui::tipName(button->mainPart()).startsWith(QStringLiteral("Export master · ")));

        window.setConnection(QStringLiteral("ready"));
        QVERIFY(button->mainPart()->isEnabled());
        QCOMPARE(ui::tipName(button->mainPart()), QStringLiteral("Export master"));
        QCOMPARE(ui::tipShortcut(button->mainPart()),
                 window.m_commands->action(QStringLiteral("export.master"))->shortcut().toString(QKeySequence::NativeText));
        QTest::mouseClick(button->mainPart(), Qt::LeftButton);
        // The Deliver tray's Master command; without a project folder it names no folder or file.
        QCOMPARE(window.m_lastCommand, deliver::exportCommand(deliver::output(window.m_projectDirectory, QStringLiteral("master"))));
        QCOMPARE(window.m_lastCommand.value("quality").toString(), QStringLiteral("master"));
        QVERIFY(!window.m_lastCommand.contains("codec"));

        // The menu.
        button->showMenu();
        QMenu *menu = button->menu();
        QTRY_VERIFY(menu->isVisible());
        const QList<QAction *> items = menu->actions();
        QCOMPARE(items.at(0), button->masterAction());
        QCOMPARE(items.at(1), button->draftAction());
        QCOMPARE(items.at(2), window.m_commands->action(QStringLiteral("sequence.renderPreview")));
        QVERIFY(items.at(3)->isSeparator());
        QCOMPARE(items.at(4)->text(), QStringLiteral("Preflight"));
        QCOMPARE(ui::menuRole(items.at(4)), ui::MenuRole::Header);
        QVERIFY(button->masterAction()->isChecked() && button->masterAction()->actionGroup()->isExclusive());
        QVERIFY(findAction(menu, QStringLiteral("exportRenderMaster")));
        QVERIFY(findAction(menu, QStringLiteral("exportEdlLocators"))->isCheckable());
        QCOMPARE(items.last()->text(), QStringLiteral("Open Deliver"));
        menu->hide();

        // Draft: the label follows, and the main part exports a review copy.
        QSignalSpy changed(button, &ExportButton::qualityChanged);
        button->draftAction()->trigger();
        QCOMPARE(changed.size(), 1);
        QCOMPARE(button->qualityName(), QStringLiteral("Draft"));
        QCOMPARE(ui::tipName(button->mainPart()), QStringLiteral("Export draft"));
        QTest::mouseClick(button->mainPart(), Qt::LeftButton);
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "review"}, {"codec", "h264"}}));
        // File ▸ Export… still reads the quality in its own words.
        QCOMPARE(button->quality(), QStringLiteral("preview"));
        button->masterAction()->trigger();
        QCOMPARE(button->quality(), QStringLiteral("full"));

        // Open Deliver.
        button->showMenu();
        findAction(menu, QStringLiteral("exportOpenDeliver"))->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Deliver);
        menu->hide();
        window.m_shell->setTray(StageShell::Tray::None);

        // Its outline is line2 on the chrome; the label is ink over ink3.
        const QImage image = button->grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        const QColor edge = image.pixelColor(QPoint(button->width() / 2, 0) * ratio);
        QVERIFY2(edge.lightness() > tokens::color(tokens::chrome).lightness(), qPrintable(edge.name()));
    }

    // The preflight rows come from the last check: failed rows always, and
    // passed rows in the check's order until there are four. A failed row
    // asks Kerf (the comments panel at its thread, the composer filled);
    // Run QC when nothing was checked; the running check's progress.
    void toolbarPreflightRowsComeFromTheCheck() {
        // The selection, without a window.
        const QJsonObject summary = checkFixture().value("summary").toObject();
        const QList<ToolbarWiring::PreflightRow> rows = ToolbarWiring::preflightRows(summary);
        QCOMPARE(rows.size(), 4);
        QCOMPARE(rows.at(0).label, QStringLiteral("Integrated loudness · −23.1 LUFS"));
        QCOMPARE(rows.at(0).key, QStringLiteral("R128"));
        QVERIFY(rows.at(0).passed);
        QCOMPARE(rows.at(1).label, QStringLiteral("True peak · −0.4 dBTP"));
        QVERIFY(!rows.at(1).passed);
        QCOMPARE(rows.at(1).ask, QStringLiteral("@Kerf bring true peak to ≤ −1 dBTP (now −0.4 dBTP)"));
        QCOMPARE(rows.at(1).commentId, QStringLiteral("c-2718"));
        QCOMPARE(rows.at(2).code, QStringLiteral("loudness.range"));
        QCOMPARE(rows.at(3).code, QStringLiteral("text.overlap"));
        QCOMPARE(rows.at(3).ask, QStringLiteral("@Kerf fix the caption overlap (1 at 00:00:17:08)"));
        QCOMPARE(ToolbarWiring::preflightHidden(summary), 0);
        // Many failures: the first six, and how many more.
        QJsonArray failing;
        for (int i = 0; i < 9; ++i)
            failing.append(QJsonObject{{"code", QStringLiteral("r%1").arg(i)}, {"name", QStringLiteral("Row %1").arg(i)}, {"passed", false},
                                       {"value", "1"}});
        const QJsonObject many{{"rows", failing}};
        QCOMPARE(ToolbarWiring::preflightRows(many).size(), 6);
        QCOMPARE(ToolbarWiring::preflightHidden(many), 3);

        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        ExportButton *button = window.m_exportButton;
        QMenu *menu = button->menu();
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        // Never checked: Run QC.
        wiring->buildExportMenu();
        QAction *run = findAction(menu, QStringLiteral("preflightRun"));
        QVERIFY(run);
        QCOMPARE(run->text(), QStringLiteral("Not checked yet"));
        window.m_lastCommand = {};
        run->trigger();
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("check"));

        // Running.
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "running"}, {"progress", 0.42}});
        wiring->buildExportMenu();
        QCOMPARE(findAction(menu, QStringLiteral("preflightRunning"))->text(), QStringLiteral("Checking… 42%"));
        QVERIFY(!findAction(menu, QStringLiteral("preflightRunning"))->isEnabled());

        // Done: the rows, dotted green and red.
        window.handleEvent(checkFixture());
        wiring->buildExportMenu();
        QVERIFY(!findAction(menu, QStringLiteral("preflightRun")));
        QAction *integrated = findAction(menu, QStringLiteral("preflight.loudness.integrated"));
        QAction *peak = findAction(menu, QStringLiteral("preflight.loudness.true_peak"));
        QVERIFY(integrated && peak);
        QCOMPARE(integrated->text(), QStringLiteral("Integrated loudness · −23.1 LUFS"));
        QCOMPARE(integrated->property("kerfMenuDot").value<QColor>(), tokens::color(tokens::pos));
        QCOMPARE(peak->property("kerfMenuDot").value<QColor>(), tokens::color(tokens::neg));
        QVERIFY(!findAction(menu, QStringLiteral("preflight.picture.levels")));
        QVERIFY(findAction(menu, QStringLiteral("preflight.text.overlap")));

        // Ask Kerf: the panel at the row's thread, the composer filled.
        QSignalSpy thread(window.m_bus, &EditorBus::openThread);
        QSignalSpy prefill(window.m_bus, &EditorBus::askPrefill);
        peak->trigger();
        QCOMPARE(thread.size(), 1);
        QCOMPARE(thread.at(0).at(0).toString(), QStringLiteral("c-2718"));
        QCOMPARE(prefill.size(), 1);
        QCOMPARE(prefill.at(0).at(0).toString(), QStringLiteral("@Kerf bring true peak to ≤ −1 dBTP (now −0.4 dBTP)"));
        QCOMPARE(prefill.at(0).at(1).toString(), QStringLiteral("c-2718"));
        // A passed row shows the QC pane in Finish.
        QSignalSpy highlight(window.m_bus, &EditorBus::highlightPane);
        wiring->buildExportMenu();
        findAction(menu, QStringLiteral("preflight.loudness.integrated"))->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Finish);
        QCOMPARE(highlight.size(), 1);
        QCOMPARE(highlight.at(0).at(0).toString(), QStringLiteral("qc"));
        window.m_shell->setTray(StageShell::Tray::None);

        // A failed check says so and runs again; its rows stay.
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "failed"}, {"message", "The check was cancelled"}});
        wiring->buildExportMenu();
        QCOMPARE(findAction(menu, QStringLiteral("preflightFailed"))->text(), QStringLiteral("The last check failed: The check was cancelled"));
        QVERIFY(findAction(menu, QStringLiteral("preflight.loudness.true_peak")));
    }

    // Export follows Deliver: its master waits while a blocking note is
    // open, says which and where, and brings up the blocking notes to
    // resolve; a draft still goes. kerfd's own refusal is shown the same way.
    void exportWaitsForBlockingNotesAsDeliverDoes() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        ExportButton *button = window.m_exportButton;
        QAbstractButton *main = nullptr;
        for (QAbstractButton *child : button->findChildren<QAbstractButton *>())
            if (child->accessibleName() == QStringLiteral("Export")) main = child;
        QVERIFY(main);
        QCOMPARE(button->exportQuality(), QStringLiteral("master"));
        window.handleEvent(QJsonObject{{"type", "comments"}, {"reviewRevision", 3},
                                       {"items", QJsonArray{comment("c-1708", nullptr, true, false), comment("c-0903", nullptr, false, false)}}});
        ReviewModel review;
        review.setComments(QJsonArray{comment("c-1708", nullptr, true, false), comment("c-0903", nullptr, false, false)});
        window.m_bus->setReview(&review);

        window.m_lastCommand = {};
        window.m_errorBanner->hide();
        main->click();
        QVERIFY(window.m_lastCommand.isEmpty());
        QVERIFY(!window.m_errorBanner->isHidden());
        QCOMPARE(window.m_errorMessage->text(),
                 QStringLiteral("A blocking note holds the master back: \u201cCaption sits on the lower third\u201d by Mina Park at "
                                "00:00:17:08. Resolve it in Review, or export a Draft meanwhile."));
        QCOMPARE(review.tag(), QString::fromLatin1(ReviewModel::kBlockingTag));

        // A draft is a review copy, not a delivery.
        button->setQuality(QStringLiteral("review"));
        main->click();
        QCOMPARE(window.m_lastCommand.value("quality").toString(), QStringLiteral("review"));

        // kerfd refuses a master too (an agent's, or one sent before the notes
        // came); the editor says so in its words and shows the notes.
        review.setTag(QString());
        window.m_errorBanner->hide();
        const QString refusal = QStringLiteral("a blocking note holds delivery back: \"Mix is hot\" by Mina (comment c-1)");
        window.handleEvent(QJsonObject{{"type", "error"}, {"code", "BLOCKING_NOTES"}, {"command", "export"}, {"message", refusal}});
        QCOMPARE(window.m_errorMessage->text(), refusal);
        QCOMPARE(review.tag(), QString::fromLatin1(ReviewModel::kBlockingTag));

        // Resolved, the master goes.
        review.setComments(QJsonArray{comment("c-1708", nullptr, true, true)});
        button->setQuality(QStringLiteral("master"));
        window.m_lastCommand = {};
        main->click();
        QCOMPARE(window.m_lastCommand.value("quality").toString(), QStringLiteral("master"));
        window.m_bus->setReview(nullptr);
    }

    // Render master waits while blocking notes are open; the warn row counts
    // them (the review model's count when there is one). The EDL locators
    // choice is kept per project.
    void toolbarRenderMasterIsDisabledWhileBlocking() {
        QCOMPARE(ToolbarWiring::blockingThreads(QJsonArray{comment("c-1", nullptr, true, false), comment("c-2", nullptr, false, false),
                                                           comment("c-3", nullptr, false, false), comment("c-3r", "c-3", true, false),
                                                           comment("c-4", nullptr, true, true)}),
                 2);

        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.setConnection(QStringLiteral("ready"));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QMenu *menu = window.m_exportButton->menu();
        wiring->buildExportMenu();
        QCOMPARE(findAction(menu, QStringLiteral("preflightBlocking"))->text(), QStringLiteral("No blocking notes open"));
        QVERIFY(findAction(menu, QStringLiteral("exportRenderMaster"))->isEnabled());

        window.handleEvent(QJsonObject{{"type", "comments"}, {"reviewRevision", 3},
                                       {"items", QJsonArray{comment("c-1708", nullptr, true, false), comment("c-0903", nullptr, true, false)}}});
        wiring->buildExportMenu();
        QAction *blocking = findAction(menu, QStringLiteral("preflightBlocking"));
        QCOMPARE(blocking->text(), QStringLiteral("2 blocking notes open"));
        QCOMPARE(ui::menuRole(blocking), ui::MenuRole::Warn);
        QAction *render = findAction(menu, QStringLiteral("exportRenderMaster"));
        QVERIFY(!render->isEnabled());
        QCOMPARE(render->toolTip(), QStringLiteral("Resolve blocking notes first"));

        // With a review model on the bus, its count is the one.
        ReviewModel review;
        review.setComments(QJsonArray{comment("c-1708", nullptr, true, false), comment("c-0903", nullptr, true, true)});
        window.m_bus->setReview(&review);
        QCOMPARE(wiring->blockingCount(), 1);
        wiring->buildExportMenu();
        QCOMPARE(findAction(menu, QStringLiteral("preflightBlocking"))->text(), QStringLiteral("1 blocking note open"));
        review.setComments(QJsonArray{comment("c-1708", nullptr, true, true)});
        wiring->buildExportMenu();
        render = findAction(menu, QStringLiteral("exportRenderMaster"));
        QVERIFY(render->isEnabled());
        window.m_lastCommand = {};
        render->trigger();
        // The Deliver tray's Master command; without a project folder it names no folder or file.
        QCOMPARE(window.m_lastCommand, deliver::exportCommand(deliver::output(window.m_projectDirectory, QStringLiteral("master"))));
        QCOMPARE(window.m_lastCommand.value("quality").toString(), QStringLiteral("master"));
        QVERIFY(!window.m_lastCommand.contains("codec"));
        window.m_bus->setReview(nullptr);

        // Offline, Render master says why.
        window.setConnection(QStringLiteral("disconnected"));
        wiring->buildExportMenu();
        QVERIFY(!findAction(menu, QStringLiteral("exportRenderMaster"))->isEnabled());

        // EDL locators: without a project there is nothing to keep it for.
        QVERIFY(wiring->edlLocatorsKey().isEmpty());
        wiring->buildExportMenu();
        QVERIFY(!findAction(menu, QStringLiteral("exportEdlLocators"))->isEnabled());

        // With one it is the Deliver paperwork's own option: on by default,
        // kept under Deliver's key for the project (deliver::settingsKey,
        // named after the folder and told apart by its path), and a change
        // tells Deliver so its paperwork rebuilds.
        window.m_projectDirectory = QStringLiteral("/tmp/kerf-toolbar-test/atlas");
        const QString key = wiring->edlLocatorsKey();
        QCOMPARE(key, deliver::settingsKey(window.m_projectDirectory, QStringLiteral("edlLocators")));
        QVERIFY(key.startsWith(QStringLiteral("deliver/atlas")) && key.endsWith(QStringLiteral("/edlLocators")));
        QSettings().remove(key);
        wiring->buildExportMenu();
        QAction *locators = findAction(menu, QStringLiteral("exportEdlLocators"));
        QVERIFY(locators->isEnabled());
        QVERIFY(locators->isChecked());
        QSignalSpy changed(deliver::Settings::instance(), &deliver::Settings::changed);
        locators->trigger();
        QVERIFY(!deliver::option(window.m_projectDirectory, QStringLiteral("edlLocators")));
        QCOMPARE(changed.count(), 1);
        QCOMPARE(changed.at(0).at(0).toString(), window.m_projectDirectory);
        wiring->buildExportMenu();
        QVERIFY(!findAction(menu, QStringLiteral("exportEdlLocators"))->isChecked());
        // A change made in Deliver shows here.
        deliver::setOption(window.m_projectDirectory, QStringLiteral("edlLocators"), true);
        wiring->buildExportMenu();
        QVERIFY(findAction(menu, QStringLiteral("exportEdlLocators"))->isChecked());
        QSettings().remove(key);
        window.m_projectDirectory.clear();
    }

    // The title: "Project · Sequence" over "Edited 2 min ago · rev 22",
    // centred on the window at 1600 and 1366, clear of both groups.
    void toolbarTitleIsCentredOnTheWindow() {
        const QDateTime now = QDateTime::fromString(QStringLiteral("2026-11-04T10:12:00Z"), Qt::ISODate);
        QCOMPARE(editedAgo(now.addSecs(-20), now), QStringLiteral("just now"));
        QCOMPARE(editedAgo(now.addSecs(-120), now), QStringLiteral("2 min ago"));
        QCOMPARE(editedAgo(now.addSecs(-3 * 3600), now), QStringLiteral("3 h ago"));
        QCOMPARE(editedAgo(now.addDays(-3), now), QStringLiteral("3 days ago"));
        QCOMPARE(editedAgo(QDateTime(), now), QString());

        MainWindow window{QString(), QString()};
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        wiring->setClock([now] { return now; });
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z")));
        window.m_errorBanner->hide();
        QCOMPARE(wiring->titleText(), QStringLiteral("Atlas · 30 s spot v7"));
        QCOMPARE(wiring->subtitleText(), QStringLiteral("Edited 2 min ago · rev 22"));
        QCOMPARE(window.windowTitle(), QStringLiteral("Atlas · 30 s spot v7"));
        for (const int width : {1600, 1366}) {
            window.resize(width, width == 1600 ? 1000 : 768);
            window.show();
            QVERIFY(QTest::qWaitForWindowExposed(&window));
            QCoreApplication::processEvents();
            QLabel *title = window.m_projectTitle;
            QLabel *subtitle = window.m_projectSubtitle;
            QCOMPARE(title->text(), QStringLiteral("Atlas · 30 s spot v7"));
            QCOMPARE(subtitle->text(), QStringLiteral("Edited 2 min ago · rev 22"));
            const int centre = window.m_toolbar->width() / 2;
            QVERIFY2(std::abs(title->geometry().center().x() - centre) <= 1,
                     qPrintable(QStringLiteral("%1 at %2 vs %3").arg(width).arg(title->geometry().center().x()).arg(centre)));
            QCOMPARE(subtitle->geometry().center().x(), title->geometry().center().x());
            QCOMPARE(title->geometry().top(), 4);
            QCOMPARE(subtitle->geometry().top(), 20);
            QVERIFY(title->testAttribute(Qt::WA_TransparentForMouseEvents));
            // UI 500 13 ink over UI 11 ink3.
            QCOMPARE(title->palette().color(QPalette::WindowText), tokens::color(tokens::ink));
            QCOMPARE(subtitle->palette().color(QPalette::WindowText), tokens::color(tokens::ink3));
            const int leftEnd = wiring->m_appButton->mapTo(window.m_toolbar, QPoint(wiring->m_appButton->width(), 0)).x();
            QVERIFY(title->geometry().left() > leftEnd);
            QVERIFY(title->geometry().right() < window.m_formatBank->mapTo(window.m_toolbar, QPoint()).x());
        }
        // Without a revision time, only the revision.
        QJsonObject snapshot = titledSnapshot(QString());
        snapshot.remove("revisionTime");
        window.applySnapshot(snapshot);
        QCOMPARE(wiring->subtitleText(), QStringLiteral("rev 22"));
    }

    // At 1366 × 768 (and the window's least size) every control shows, 28
    // tall and centred in the 40 pt bar, the groups never overlap, and the
    // title gives way between them.
    void toolbarFitsASmallWindow() {
        MainWindow window{QString(), QString()};
        // A long project and sequence name, as the engine names them (the
        // title prefers timeline.name to the window's project name).
        QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
        QJsonObject timeline = snapshot.value("timeline").toObject();
        timeline.insert("name", "Harbor Teaser, the long cut for review");
        timeline.insert("sequence", "seq-1");
        timeline.insert("sequences", QJsonArray{QJsonObject{{"id", "seq-1"}, {"name", "Client review cut v12 with burnt-in captions and temp music for the second agency round, version after the Friday notes and the second colour pass with the new end card"},
                                                            {"width", 1920}, {"height", 1080}, {"rate", "25"}, {"fps", 25}}});
        snapshot.insert("timeline", timeline);
        window.applySnapshot(snapshot);
        const QString fullTitle = QStringLiteral("Harbor Teaser, the long cut for review · Client review cut v12 with burnt-in captions and temp music for the second agency round, version after the Friday notes and the second colour pass with the new end card");
        QCOMPARE(window.m_toolbarWiring->titleText(), fullTitle);
        window.resize(1366, 768);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        QToolBar *toolbar = window.m_toolbar;
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QCOMPARE(toolbar->height(), 40);
        QCOMPARE(wiring->m_content->geometry(), toolbar->rect());
        QList<QWidget *> controls{wiring->m_appButton, window.m_formatBank, window.m_exportButton};
        const auto frame = [toolbar](QWidget *w) { return QRect(w->mapTo(toolbar, QPoint()), w->size()); };
        for (int size : {1366, 1080}) {
            window.resize(size, 768);
            QCoreApplication::processEvents();
            for (qsizetype i = 0; i < controls.size(); ++i) {
                QWidget *control = controls.at(i);
                QVERIFY2(control->isVisible(), qPrintable(control->objectName()));
                QCOMPARE(control->height(), 28);
                QVERIFY2(QRect(QPoint(), toolbar->size()).contains(frame(control)), qPrintable(control->objectName()));
                QCOMPARE(frame(control).center().y(), toolbar->height() / 2 - 1);
                if (i > 0) QVERIFY2(frame(controls.at(i - 1)).right() < frame(control).left(), qPrintable(control->objectName()));
            }
            // Export ends 12 in from the window's right.
            QCOMPARE(frame(window.m_exportButton).right() + 1, toolbar->width() - 12);
            // The title keeps between the groups, shortened when it must be.
            QLabel *title = window.m_projectTitle;
            QVERIFY(!title->text().isEmpty());
            QVERIFY2(title->text() != fullTitle, qPrintable(title->text()));
            QVERIFY(title->text().endsWith(QChar(0x2026)));
            QCOMPARE(title->toolTip(), fullTitle);
            QVERIFY(QFontMetrics(title->font()).horizontalAdvance(title->text()) <= title->width());
            QVERIFY(title->geometry().left() > frame(wiring->m_appButton).right());
            QVERIFY(title->geometry().right() < frame(window.m_formatBank).left());
        }
        // The right group's spacing, and the ≡ button's size.
        window.resize(1600, 1000);
        QCoreApplication::processEvents();
        QCOMPARE(frame(window.m_exportButton).left() - frame(window.m_formatBank).right() - 1, 12);
        QCOMPARE(controls.at(0)->width(), 28);
        // No » extension: nothing is folded away.
        if (auto *more = toolbar->findChild<QToolButton *>(QStringLiteral("qt_toolbar_ext_button"))) QVERIFY(!more->isVisible());
        // The window buttons' room, past which the commands start.
        QCOMPARE(ToolbarWiring::firstControlX(0, false), 84);
        QCOMPARE(ToolbarWiring::firstControlX(70, false), 86);
        QCOMPARE(ToolbarWiring::firstControlX(70, true), 16);
        // The toolbar is chrome with its hairline at the foot (sampled above
        // the title's first line, which the long title fills here).
        const QImage image = toolbar->grab().toImage();
        const qreal ratio = image.devicePixelRatio();
        QCOMPARE(image.pixelColor(QPoint(toolbar->width() / 2 + 300, 2) * ratio).rgb(), tokens::chrome);

        // Renders for the eye: KERF_TOOLBAR_GALLERY=folder.
        if (const QString folder = qEnvironmentVariable("KERF_TOOLBAR_GALLERY"); !folder.isEmpty()) {
            window.setConnection(QStringLiteral("ready"));
            window.handleEvent(checkFixture());
            for (int size : {1600, 1366}) {
                window.resize(size, size == 1600 ? 1000 : 768);
                QCoreApplication::processEvents();
                toolbar->grab().save(QDir(folder).filePath(QStringLiteral("toolbar-%1.png").arg(size)));
            }
            window.m_exportButton->showMenu();
            QTRY_VERIFY(window.m_exportButton->menu()->isVisible());
            window.m_exportButton->menu()->grab().save(QDir(folder).filePath(QStringLiteral("export-menu.png")));
            window.m_exportButton->menu()->hide();
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ToolbarTest)

#include "toolbar_test.moc"
