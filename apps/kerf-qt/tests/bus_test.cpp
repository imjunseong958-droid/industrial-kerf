// Tests for the editor bus: the window's bridge events fan out to it, its
// requests go out as the window's own do, and the state it mirrors (the
// snapshot, the playhead, the selection, the workspace, the program frame)
// follows the window. Also the region shells that hear the engine through it.

#include "fixtures.h"
#include "test_registry.h"

#include "agent_hud.h"
#include "bridge.h"
#include "browser_panel.h"
#include "chat_panel.h"
#include "command_registry.h"
#include "editor_bus.h"
#include "inspector_column.h"
#include "main_window.h"
#include "program_strip.h"
#include "scrub_strip.h"
#include "timeline_widget.h"
#include "viewer_widget.h"
#include "workspace.h"
#include "wiring/review_wiring.h"

#include <QAction>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>
#include <QToolButton>

namespace kerf {

using namespace test;

class BusTest final : public QObject {
    Q_OBJECT

private slots:
    // Each test starts with the conversation's preferences as a new user has them.
    void init() { QSettings().remove(QStringLiteral("viewer")); }

    void eventsFanOutAfterTheWindowHandlesThem() {
        MainWindow window{QString(), QString()};
        EditorBus *bus = window.bus();
        QVERIFY(bus);
        QCOMPARE(bus, window.m_bus);
        QSignalSpy events(bus, &EditorBus::event);
        const QJsonObject check{{"type", "check"}, {"status", "running"}};
        window.handleEvent(check);
        QCOMPARE(events.size(), 1);
        QCOMPARE(events.first().at(0).toString(), QStringLiteral("check"));
        QCOMPARE(events.first().at(1).toJsonObject(), check);
        QCOMPARE(bus->lastEvent(QStringLiteral("check")), check);
        QVERIFY(bus->lastEvent(QStringLiteral("loudness")).isEmpty());
        // Events the window acts on still reach the bus, after the window:
        // an agent turn, a refused preset export (which returns early).
        window.handleEvent(QJsonObject{{"type", "agent"}, {"kind", "done"}, {"text", "Trimmed the intro."}});
        QCOMPARE(events.size(), 2);
        QVERIFY(window.m_chat->transcript().contains(QStringLiteral("Trimmed the intro.")));
        window.handleEvent(QJsonObject{{"type", "preset_export"}, {"path", QStringLiteral("/nonexistent/dir/a.kerfpreset")}, {"document", "x"}});
        QCOMPARE(events.size(), 3);
        QCOMPARE(events.last().at(0).toString(), QStringLiteral("preset_export"));
        // An event without a type goes nowhere.
        window.handleEvent(QJsonObject{{"status", "ready"}});
        QCOMPARE(events.size(), 3);
        // collab.status names the person at this Mac.
        QSignalSpy identity(bus, &EditorBus::identityChanged);
        QJsonObject status = sharedStatus();
        status.insert(QStringLiteral("user"), QJsonObject{{"name", "Jun"}, {"email", "jun@example.com"}});
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QCOMPARE(identity.size(), 1);
        QCOMPARE(bus->identity().value("email").toString(), QStringLiteral("jun@example.com"));
        window.handleEvent(QJsonObject{{"type", "collab"}, {"status", status}});
        QCOMPARE(identity.size(), 1);
    }

    void sendAndRunGoOutAsTheWindowsOwnRequests() {
        MainWindow window{QString(), QString()};
        EditorBus *bus = window.bus();
        // Offline, nothing is sent.
        QVERIFY(!bus->send(QJsonObject{{"type", "check"}}));
        QVERIFY(window.m_lastCommand.isEmpty());
        window.setConnection(QStringLiteral("ready"));
        window.applySnapshot(QJsonObject{{"revision", 5}, {"duration", 4}, {"timeline", QJsonObject{{"fps", 24}, {"tracks", QJsonArray{}}}}});
        window.m_lastCommand = {};
        // send(): through sendSimple, so m_lastCommand has it (the tests'
        // bridge is absent, so the send itself reports failure).
        bus->send(QJsonObject{{"type", "comment_list"}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "comment_list"}}));
        QVERIFY(!bus->send(QJsonObject{}));
        // run(): an engine command resolves in the window's context.
        bus->run(QStringLiteral("qc.run"));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "check"}}));
        bus->run(QStringLiteral("export.master"));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "export"}, {"quality", "master"}}));
        bus->run(QStringLiteral("edit.undo"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("export"));
        bus->run(QStringLiteral("proposal.accept"), QJsonObject{{"proposal", "p9"}});
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "accept"}, {"proposalId", "p9"}, {"revision", 5}}));
        // A frontend command runs as its menu item does: a switch toggles.
        QVERIFY(window.chatOpen());
        QVERIFY(bus->run(QStringLiteral("view.comments")));
        QVERIFY(!window.chatOpen());
        QVERIFY(bus->run(QStringLiteral("window.assistant")));
        QVERIFY(window.chatOpen());
        // The viewer's switches run now; one still to come does not.
        QAction *dim = window.m_commands->action(QStringLiteral("view.dim"));
        const bool dimmed = dim->isChecked();
        QVERIFY(bus->run(QStringLiteral("view.dim")));
        QCOMPARE(dim->isChecked(), !dimmed);
        QVERIFY(bus->run(QStringLiteral("view.dim")));
        QCOMPARE(dim->isChecked(), dimmed);
        QVERIFY(!bus->run(QStringLiteral("sequence.settings")));
        QVERIFY(!bus->run(QString()));
    }

    void stateFollowsTheWindow() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        EditorBus *bus = window.bus();
        window.setConnection(QStringLiteral("ready"));
        QSignalSpy snapshots(bus, &EditorBus::snapshotChanged);
        QSignalSpy playheads(bus, &EditorBus::playheadChanged);
        QSignalSpy playing(bus, &EditorBus::playingChanged);
        QSignalSpy selections(bus, &EditorBus::selectionChanged);
        QSignalSpy workspaces(bus, &EditorBus::workspaceChanged);
        QSignalSpy frames(bus, &EditorBus::programFrameChanged);
        const QJsonObject snapshot = showcaseSnapshot(7, QStringLiteral("clean"));
        window.applySnapshot(snapshot);
        QCOMPARE(snapshots.size(), 1);
        QCOMPARE(bus->revision(), quint64(7));
        QCOMPARE(bus->snapshot(), snapshot);
        QCOMPARE(bus->timeline(), window.m_project);
        QCOMPARE(bus->fps(), window.m_fps);
        QVERIFY(bus->frameSize().isValid());
        QCOMPARE(bus->duration(), window.m_duration);
        // The clock and the seek move the playhead.
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 2.5}, {"duration", window.m_duration}});
        QCOMPARE(bus->playhead(), 2.5);
        QCOMPARE(playheads.size(), 1);
        window.queueSeek(1.0);
        QCOMPARE(bus->playhead(), 1.0);
        window.handleEvent(QJsonObject{{"type", "playback"}, {"playing", true}});
        QVERIFY(bus->playing());
        QCOMPARE(playing.size(), 1);
        window.setConnection(QStringLiteral("disconnected"));
        QVERIFY(!bus->playing());
        window.setConnection(QStringLiteral("ready"));
        // Selections, by click, by edit and by ⌘-click.
        window.clickSelectClip(QStringLiteral("v1b"));
        QCOMPARE(bus->selection(), QStringList{QStringLiteral("v1b")});
        window.extendClipSelection(QStringLiteral("v1a"), true);
        QCOMPARE(bus->selection(), (QStringList{QStringLiteral("v1b"), QStringLiteral("v1a")}));
        window.selectClip(QString());
        QVERIFY(bus->selection().isEmpty());
        QCOMPARE(selections.size(), 3);
        // The workspace.
        QCOMPARE(bus->workspace(), QStringLiteral("Review"));
        window.m_commands->action(QStringLiteral("workspace.finish"))->trigger();
        QCOMPARE(bus->workspace(), QStringLiteral("Finish"));
        QCOMPARE(workspaces.size(), 1);
        // The program frame is the one the viewer gets, through the path
        // pollBridge takes (offline, none is shown).
        QImage frame(16, 9, QImage::Format_RGBA8888);
        frame.fill(Qt::red);
        const KerfFrame bridged{frame.constBits(), size_t(frame.sizeInBytes()), 16, 9, 0.5};
        window.setConnection(QStringLiteral("offline"));
        QVERIFY(!window.showProgramFrame(bridged));
        QCOMPARE(frames.size(), 0);
        window.setConnection(QStringLiteral("ready"));
        QVERIFY(window.showProgramFrame(bridged));
        QCOMPARE(frames.size(), 1);
        QCOMPARE(window.m_viewer->frame(), frame);
        QCOMPARE(bus->programFrame(), window.m_viewer->frame());
        QCOMPARE(frames.first().first().value<QImage>(), frame);
        // View-as and the review model are the bus's to hold.
        QSignalSpy viewAs(bus, &EditorBus::viewAsChanged);
        bus->setViewAs(QStringLiteral("mina@example.com"));
        bus->setViewAs(QStringLiteral("mina@example.com"));
        QCOMPARE(viewAs.size(), 1);
        QCOMPARE(bus->viewAs(), QStringLiteral("mina@example.com"));
        // The comments region builds the review model and hands it over.
        QVERIFY(bus->review());
        QCOMPARE(bus->review(), window.m_reviewWiring->model());
    }

    // The shells hear the engine through the bus: the Program strip says
    // when a proposal is previewed and exits it; the HUD follows the
    // agent; the scrub strip shows the playhead.
    void shellsHearTheEngineThroughTheBus() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QJsonObject snapshot = showcaseSnapshot(3, QStringLiteral("clean"));
        snapshot.insert(QStringLiteral("proposals"), QJsonArray{QJsonObject{
            {"proposalId", "p2"}, {"status", "pending"}, {"version", 2},
            {"versions", QJsonArray{QJsonObject{{"version", 1}, {"proposalId", "p1"}}, QJsonObject{{"version", 2}, {"proposalId", "p2"}}}}}});
        window.applySnapshot(snapshot);
        ProgramStrip *strip = window.m_programStrip;
        QVERIFY(!strip->previewShown());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "p2"}});
        QVERIFY(strip->previewShown());
        QCOMPARE(strip->previewText(), QStringLiteral("Previewing proposal v2"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "live"}, {"proposalId", "p1"}});
        QCOMPARE(strip->previewText(), QStringLiteral("Previewing proposal v1"));
        window.m_lastCommand = {};
        strip->exitPreviewButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "preview_exit"}}));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "exited"}});
        QVERIFY(!strip->previewShown());
        // The program's own preview is not a proposal's.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}});
        QVERIFY(!strip->previewShown());

        // The HUD follows a proposal's preview render (mockup-spec 3.3),
        // not the agent's turn.
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}});
        AgentHud hud(nullptr, window.bus());
        QVERIFY(!hud.active() && hud.isHidden());
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", true}});
        QVERIFY(!hud.active());
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "running"}, {"proposalId", "p2"}, {"progress", 0.62}});
        QVERIFY(hud.active());
        QCOMPARE(hud.text(), QStringLiteral("Rendering preview · v2"));
        QCOMPARE(hud.percentText(), QStringLiteral("62%"));
        window.handleEvent(QJsonObject{{"type", "preview"}, {"status", "ready"}, {"proposalId", "p2"}});
        QVERIFY(!hud.active() && hud.text().isEmpty());
        window.handleEvent(QJsonObject{{"type", "agent_busy"}, {"busy", false}});

        ScrubStrip scrub(nullptr, window.bus());
        QCOMPARE(scrub.height(), 56);
        window.handleEvent(QJsonObject{{"type", "clock"}, {"position", 1.0}, {"duration", window.m_duration}});
        QVERIFY(scrub.timeText().startsWith(QStringLiteral("00:00:01:00 / ")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::BusTest)

#include "bus_test.moc"
