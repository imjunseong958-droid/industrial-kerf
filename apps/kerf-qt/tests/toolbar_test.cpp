// Tests for the toolbar (mockup-spec 1, 11, 14): the workspaces, the
// centred title, the format segments, the sequence button, the Export split
// button and its preflight.

#include "fixtures.h"
#include "test_registry.h"

#include "command_registry.h"
#include "components.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "main_window.h"
#include "render_queue_view.h"
#include "review_model.h"
#include "toolbar_widgets.h"
#include "wiring/toolbar_wiring.h"
#include "workspace.h"

#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QCoreApplication>
#include <QDir>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QMenu>
#include <QSettings>
#include <QStatusBar>
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

    // The workspace segments are the four built-ins; a click chooses one,
    // and a workspace chosen elsewhere moves the fill. One's own workspace
    // turns none on. The chevron opens the Workspaces menu.
    void toolbarWorkspaceSegmentsSwitchTheWorkspace() {
        const WorkspaceSettingsGuard guard;
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        ToolbarSegments *segments = wiring->m_workspaces;
        QVERIFY(segments);
        QCOMPARE(segments->count(), 4);
        QCOMPARE(segments->segmentText(0), QStringLiteral("Edit"));
        QCOMPARE(segments->segmentText(3), QStringLiteral("Deliver"));
        QCOMPARE(segments->height(), 28);
        QCOMPARE(segments->segment(0)->height(), 24);
        QCOMPARE(segments->menuTail()->width(), ToolbarSegments::kTailWidth);
        QCOMPARE(segments->currentIndex(), Workspace::builtIns().indexOf(window.m_workspace->current()));
        // The tips name each and give the keymap's key.
        QCOMPARE(ui::tipName(segments->segment(2)), QStringLiteral("Finish workspace · scopes, loudness, QC"));
        QCOMPARE(ui::tipShortcut(segments->segment(0)),
                 window.m_commands->action(QStringLiteral("workspace.edit"))->shortcut().toString(QKeySequence::NativeText));

        QTest::mouseClick(segments->segment(0), Qt::LeftButton);
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Edit"));
        QCOMPARE(segments->currentIndex(), 0);
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.edit"))->isChecked());
        window.m_workspace->activate(QStringLiteral("Finish"));
        QCOMPARE(segments->currentIndex(), 2);
        // The fill sits under the on segment once it has slid there.
        QTRY_COMPARE(segments->fillRect(), segments->segmentRect(2));

        // One's own workspace: no built-in segment is on.
        QVERIFY(window.m_workspace->saveAs(QStringLiteral("Toolbar own")));
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Toolbar own"));
        QCOMPARE(segments->currentIndex(), -1);
        QVERIFY(segments->fillRect().isEmpty());

        // The chevron: the Workspaces menu, as Window ▸ Workspaces lists it.
        QTest::mouseClick(segments->menuTail(), Qt::LeftButton);
        QMenu *menu = wiring->m_workspaceMenu;
        QTRY_VERIFY(menu->isVisible());
        QCOMPARE(menuTexts(menu), (QStringList{"Workspaces", "Edit", "Review", "Finish", "Deliver", "Toolbar own", "-",
                                               "Save Changes to This Workspace", "Save as New Workspace…", "Reset to Saved Layout"}));
        QCOMPARE(ui::menuRole(menu->actions().first()), ui::MenuRole::Header);
        QAction *own = menu->actions().at(5);
        QVERIFY(own->isChecked());
        QCOMPARE(own->property("kerfMenuKey").toString(), QStringLiteral("saved"));
        menu->hide();
        menu->actions().at(2)->trigger();
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Review"));
        QCOMPARE(segments->currentIndex(), 1);
        // ←→ walk the segments and choose.
        segments->setFocus(Qt::TabFocusReason);
        QTest::keyClick(segments, Qt::Key_Right);
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Finish"));
        window.m_workspace->activate(QStringLiteral("Review"));
        window.close();
    }

    // The format segments: one per preset, 28 wide icons; a click sends the
    // set-sequence edit through sequence.setFormat; ⌥1-4 do the same; a
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
        QCOMPARE(ui::tipName(bank->segment(3)), QStringLiteral("Portrait 4:5 · 1080×1350"));
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

        // ⌥3 is Square.
        QAction *square = nullptr;
        for (QAction *action : window.actions())
            if (action->objectName() == QStringLiteral("format.1:1")) square = action;
        QVERIFY(square);
        QCOMPARE(square->shortcut(), QKeySequence(Qt::ALT | Qt::Key_3));
        square->trigger();
        const QJsonObject squareOp = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(QSize(squareOp.value("width").toInt(), squareOp.value("height").toInt()), QSize(1080, 1080));

        // A size no preset has: none on, the tooltip names it.
        window.syncCanvasPreset(QSize(1280, 720));
        QCOMPARE(bank->currentIndex(), -1);
        QVERIFY(bank->fillRect().isEmpty());
        QCOMPARE(bank->toolTip(), QStringLiteral("Custom · 1280 × 720"));
        window.syncCanvasPreset(QSize(1080, 1350));
        QCOMPARE(bank->currentIndex(), 3);

        // Disconnected, the segments are disabled.
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(!bank->isEnabled());
    }

    // The sequence button names the frame size and rate and opens the
    // Sequence menu; a colour space shows only when the sequence has one.
    void toolbarSequenceButtonOpensTheSequenceMenu() {
        MainWindow window{QString(), QString()};
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        // Before the project answers there is nothing to name.
        QCOMPARE(wiring->m_sequence->text(), QStringLiteral("Sequence"));
        QVERIFY(!wiring->m_sequence->isEnabled());
        window.applySnapshot(titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z")));
        QCOMPARE(wiring->m_sequence->text(), QStringLiteral("1920×1080 · 25p"));
        QVERIFY(wiring->m_sequence->isMono());
        QCOMPARE(wiring->m_sequence->height(), 28);
        QCOMPARE(ui::tipName(wiring->m_sequence), QStringLiteral("Sequence settings"));
        window.syncCanvasPreset(QSize(1080, 1920));
        QCOMPARE(wiring->m_sequence->text(), QStringLiteral("1080×1920 · 25p"));

        QMenu *menu = wiring->m_sequenceMenu;
        wiring->buildSequenceMenu(menu);
        QStringList items = menuTexts(menu);
        QCOMPARE(items.first(), QStringLiteral("Sequence · 1080×1920 · 25p"));
        QVERIFY(!items.contains(QStringLiteral("Colour space")));
        QVERIFY(items.contains(QStringLiteral("Frame rate")));
        QVERIFY(!findAction(menu, QStringLiteral("sequenceFrameRate"))->isEnabled());
        QAction *frameSize = findAction(menu, QStringLiteral("sequenceFrameSize"));
        QVERIFY(frameSize && frameSize->menu());
        QCOMPARE(frameSize->menu()->actions().size(), 4);
        QVERIFY(frameSize->menu()->actions().at(1)->isChecked());
        QCOMPARE(menu->actions().last(), window.m_commands->action(QStringLiteral("sequence.settings")));
        // The presets go the format's way.
        window.setConnection(QStringLiteral("ready"));
        wiring->buildSequenceMenu(menu);
        findAction(findAction(menu, QStringLiteral("sequenceFrameSize"))->menu(), QStringLiteral("frameSize.4:5"))->trigger();
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().first().toObject();
        QCOMPARE(QSize(op.value("width").toInt(), op.value("height").toInt()), QSize(1080, 1350));

        // A sequence that carries a colour space names it.
        QJsonObject snapshot = titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z"));
        QJsonObject timeline = snapshot.value("timeline").toObject();
        QJsonArray sequences = timeline.value("sequences").toArray();
        QJsonObject sequence = sequences.first().toObject();
        sequence.insert("colorSpace", "Rec.709");
        sequences.replace(0, sequence);
        timeline.insert("sequences", sequences);
        snapshot.insert("timeline", timeline);
        window.applySnapshot(snapshot);
        wiring->buildSequenceMenu(menu);
        items = menuTexts(menu);
        QCOMPARE(items.first(), QStringLiteral("Sequence · 1920×1080 · 25p · Rec.709"));
        QVERIFY(items.contains(QStringLiteral("Colour space")));
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
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "master"}, {"codec", "prores_hq"}}));

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
        QCOMPARE(items.last()->text(), QStringLiteral("Open Deliver workspace"));
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

        // Open Deliver workspace.
        button->showMenu();
        findAction(menu, QStringLiteral("exportOpenDeliver"))->trigger();
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Deliver"));
        menu->hide();
        window.m_workspace->activate(QStringLiteral("Review"));

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
        QSignalSpy panel(window.m_bus, &EditorBus::commentsPanelRequested);
        QSignalSpy thread(window.m_bus, &EditorBus::openThread);
        QSignalSpy prefill(window.m_bus, &EditorBus::askPrefill);
        peak->trigger();
        QCOMPARE(panel.size(), 1);
        QCOMPARE(thread.size(), 1);
        QCOMPARE(thread.at(0).at(0).toString(), QStringLiteral("c-2718"));
        QCOMPARE(prefill.size(), 1);
        QCOMPARE(prefill.at(0).at(0).toString(), QStringLiteral("@Kerf bring true peak to ≤ −1 dBTP (now −0.4 dBTP)"));
        QCOMPARE(prefill.at(0).at(1).toString(), QStringLiteral("c-2718"));
        // A passed row shows the QC pane in Finish.
        QSignalSpy highlight(window.m_bus, &EditorBus::highlightPane);
        wiring->buildExportMenu();
        findAction(menu, QStringLiteral("preflight.loudness.integrated"))->trigger();
        QCOMPARE(window.m_workspace->current(), QStringLiteral("Finish"));
        QCOMPARE(highlight.size(), 1);
        QCOMPARE(highlight.at(0).at(0).toString(), QStringLiteral("qc"));
        window.m_workspace->activate(QStringLiteral("Review"));

        // A failed check says so and runs again; its rows stay.
        window.handleEvent(QJsonObject{{"type", "check"}, {"status", "failed"}, {"message", "The check was cancelled"}});
        wiring->buildExportMenu();
        QCOMPARE(findAction(menu, QStringLiteral("preflightFailed"))->text(), QStringLiteral("The last check failed: The check was cancelled"));
        QVERIFY(findAction(menu, QStringLiteral("preflight.loudness.true_peak")));
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
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "master"}, {"codec", "prores_hq"}}));
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
        // kept under deliver/<project folder>/edlLocators, and a change tells
        // Deliver so its paperwork rebuilds.
        window.m_projectDirectory = QStringLiteral("/tmp/kerf-toolbar-test/atlas");
        const QString key = wiring->edlLocatorsKey();
        QCOMPARE(key, deliver::settingsKey(window.m_projectDirectory, QStringLiteral("edlLocators")));
        QCOMPARE(key, QStringLiteral("deliver/atlas/edlLocators"));
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
        window.applySnapshot(titledSnapshot(QStringLiteral("2026-11-04T10:10:00Z")));
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
            const int leftEnd = wiring->m_workspaces->mapTo(window.m_toolbar, QPoint(wiring->m_workspaces->width(), 0)).x();
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
        timeline.insert("sequences", QJsonArray{QJsonObject{{"id", "seq-1"}, {"name", "Client review cut v12 with burnt-in captions and temp music"},
                                                            {"width", 1920}, {"height", 1080}, {"rate", "25"}, {"fps", 25}}});
        snapshot.insert("timeline", timeline);
        window.applySnapshot(snapshot);
        const QString fullTitle = QStringLiteral("Harbor Teaser, the long cut for review · Client review cut v12 with burnt-in captions and temp music");
        QCOMPARE(window.m_toolbarWiring->titleText(), fullTitle);
        window.resize(1366, 768);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        QCoreApplication::processEvents();
        QToolBar *toolbar = window.m_toolbar;
        ToolbarWiring *wiring = window.m_toolbarWiring.get();
        QCOMPARE(toolbar->height(), 40);
        QCOMPARE(wiring->m_content->geometry(), toolbar->rect());
        QList<QWidget *> controls;
        for (const auto &[button, id] : std::as_const(wiring->m_commandButtons)) controls << button;
        controls << wiring->m_workspaces << window.m_formatBank << wiring->m_sequence << window.m_exportButton << wiring->m_settings;
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
            // Settings ends 12 in from the window's right.
            QCOMPARE(frame(wiring->m_settings).right() + 1, toolbar->width() - 12);
            // The title keeps between the groups, shortened when it must be.
            QLabel *title = window.m_projectTitle;
            QVERIFY(!title->text().isEmpty());
            QVERIFY2(title->text() != fullTitle, qPrintable(title->text()));
            QVERIFY(title->text().endsWith(QChar(0x2026)));
            QCOMPARE(title->toolTip(), fullTitle);
            QVERIFY(QFontMetrics(title->font()).horizontalAdvance(title->text()) <= title->width());
            QVERIFY(title->geometry().left() > frame(wiring->m_workspaces).right());
            QVERIFY(title->geometry().right() < frame(window.m_formatBank).left());
        }
        // The spacings of the right group (mockup-spec 1): 8, 12, 8.
        window.resize(1600, 1000);
        QCoreApplication::processEvents();
        QCOMPARE(frame(wiring->m_sequence).left() - frame(window.m_formatBank).right() - 1, 8);
        QCOMPARE(frame(window.m_exportButton).left() - frame(wiring->m_sequence).right() - 1, 12);
        QCOMPARE(frame(wiring->m_settings).left() - frame(window.m_exportButton).right() - 1, 8);
        // The command buttons are 4 apart and 28 square.
        QCOMPARE(frame(controls.at(1)).left() - frame(controls.at(0)).right() - 1, 4);
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
            window.statusBar()->grab().save(QDir(folder).filePath(QStringLiteral("status.png")));
        }
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::ToolbarTest)

#include "toolbar_test.moc"
