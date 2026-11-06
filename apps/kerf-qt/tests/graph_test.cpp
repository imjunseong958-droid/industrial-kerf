// Tests for the graph editor: the curves it draws against the compositor's,
// what it reads of a clip, and what a drag, a double-click, Delete and the
// ruler ask for.

#include "fixtures.h"
#include "test_registry.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "graph_editor.h"
#include "graph_model.h"
#include "main_window.h"
#include "ui/stage_shell.h"

#include <QAction>
#include <QJsonArray>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QTest>

namespace kerf {

namespace {

using graph::Handles;
using graph::Key;

QJsonObject key(int frame, double value, const char *interp = "linear") {
    return {{"at", frame / 24.0}, {"frame", frame}, {"value", value}, {"interp", interp}};
}

// A clip at 2 seconds, 49 frames long: x and an effect's radius with keys, and
// a static rotation.
QJsonObject clip() {
    return {{"id", "c1"},
            {"name", "Clip"},
            {"timelineStart", 2.0},
            {"durationFrames", 49},
            {"props", QJsonObject{{"x", QJsonArray{key(0, 0.0), key(24, 0.5), key(48, 0.0)}}, {"rotation", 15.0}}},
            {"effects", QJsonArray{QJsonObject{{"fx", "blur.gaussian"}, {"params", QJsonObject{{"radius", QJsonArray{key(0, 0.0, "ease"), key(48, 40.0)}}}}}}},
            {"masks", QJsonArray{QJsonObject{{"name", "k1"}, {"params", QJsonObject{{"feather", QJsonArray{key(0, 1.0), key(12, 9.0)}}}}}}}};
}

QJsonObject timeline(const QJsonObject &first) {
    return {{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"clips", QJsonArray{first}}}}}};
}

struct Harness {
    EditorBus bus;
    GraphEditor editor{nullptr, &bus};
    QSignalSpy edits{&editor, &GraphEditor::editRequested};
    QSignalSpy seeks{&editor, &GraphEditor::seekRequested};

    Harness() {
        editor.resize(1000, 320);
        set(clip());
        bus.setSelection({QStringLiteral("c1")});
    }
    void set(const QJsonObject &first) { bus.setSnapshot(QJsonObject{{"revision", 1}}, timeline(first), 24.0, QSize(1920, 1080)); }
    // The keys the last edit gave, by the channel's name, and that it was one op.
    QJsonArray lastKeys(const QString &name) const {
        const QJsonArray ops = edits.last().at(0).toJsonArray();
        Q_ASSERT(ops.size() == 1);
        const QJsonObject op = ops.first().toObject();
        const QJsonObject values = op.contains("props") ? op.value("props").toObject() : op.value("params").toObject();
        return values.value(name).toObject().value("keys").toArray();
    }
    void drag(QPointF from, QPointF to, Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
        QTest::mousePress(&editor, Qt::LeftButton, modifiers, from.toPoint());
        QMouseEvent move(QEvent::MouseMove, to, editor.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, modifiers);
        QApplication::sendEvent(&editor, &move);
        QTest::mouseRelease(&editor, Qt::LeftButton, modifiers, to.toPoint());
    }
};

} // namespace

class GraphTest final : public QObject {
    Q_OBJECT

private slots:
    // The same table holds the compositor to these, in kerf_model's value.rs.
    void curvesAreTheCompositorsCurves() {
        const struct {
            const char *interp;
            double t;
            double wanted;
        } table[] = {
            {"linear", 0.25, 0.25},
            {"hold", 0.9, 0.0},
            {"ease-in", 0.5, 0.25},
            {"ease-out", 0.5, 0.75},
            {"ease", 0.25, 0.15625},
            {"0.25,0.1,0.25,1", 0.5, 0.802403},
            {"0.42,0,0.58,1", 0.25, 0.129162},
            {"0.5,-1,0.5,2", 0.5, 0.5},
            {"0.5,-1,0.5,2", 0.25, -0.182322},
            {"0,0,1,1", 0.3, 0.3},
        };
        for (const auto &row : table)
            QVERIFY2(std::abs(graph::curve(QString::fromLatin1(row.interp), row.t) - row.wanted) < 1e-5, row.interp);
    }

    void aNamedCurvesHandlesDrawThatCurve() {
        for (const QString &name : {QStringLiteral("linear"), QStringLiteral("ease"), QStringLiteral("ease-in"), QStringLiteral("ease-out")}) {
            const auto handles = graph::handlesOf(name);
            QVERIFY(handles);
            const QString drawn = graph::interpOf(*handles);
            for (double t = 0.05; t < 1.0; t += 0.1)
                QVERIFY2(std::abs(graph::curve(name, t) - graph::curve(drawn, t)) < 1e-3, qPrintable(name));
        }
        QVERIFY(!graph::handlesOf(QStringLiteral("hold")));
    }

    void aCurveIsWrittenAsTheDocumentAllows() {
        QCOMPARE(graph::interpOf(Handles{0.25, 0.1, 0.75, 0.9}), QStringLiteral("0.25,0.1,0.75,0.9"));
        // Across stays in the span; up has a limit; four places.
        QCOMPARE(graph::interpOf(Handles{-1.0, 9.0, 3.0, -9.0}), QStringLiteral("0,5,1,-5"));
        QCOMPARE(graph::interpOf(Handles{0.123456, 0.0, 0.5, 1.0}), QStringLiteral("0.1235,0,0.5,1"));
    }

    void valuesFollowTheirKeysAndHoldOutsideThem() {
        const QList<Key> keys{{10, 1.0, QStringLiteral("linear")}, {20, 3.0, QStringLiteral("hold")}, {30, 7.0, QStringLiteral("linear")}};
        QCOMPARE(graph::valueAt(keys, 0), 1.0);
        QCOMPARE(graph::valueAt(keys, 15), 2.0);
        QCOMPARE(graph::valueAt(keys, 25), 3.0);
        QCOMPARE(graph::valueAt(keys, 30), 7.0);
        QCOMPARE(graph::valueAt(keys, 99), 7.0);
        // What a curve overshoots to counts.
        const auto [low, high] = graph::reach({{0, 0.0, QStringLiteral("0.5,-1,0.5,2")}, {10, 1.0, QStringLiteral("linear")}});
        QVERIFY(low < -0.1 && high > 1.1);
    }

    void onlyNumbersWithKeysAreChannels() {
        QJsonObject withColor = clip();
        QJsonObject props = withColor.value("props").toObject();
        props.insert("color", QJsonArray{QJsonObject{{"frame", 0}, {"value", "#ff0000"}, {"interp", "linear"}}});
        withColor.insert("props", props);
        const auto channels = graph::channelsOf(withColor);
        QCOMPARE(channels.size(), 3);
        QCOMPARE(channels.at(0).label, QStringLiteral("x"));
        QCOMPARE(channels.at(0).keys.size(), 3);
        QCOMPARE(channels.at(1).label, QStringLiteral("blur.gaussian · radius"));
        QCOMPARE(channels.at(1).index, 0);
        QCOMPARE(channels.at(2).label, QStringLiteral("k1 · feather"));
        QCOMPARE(channels.at(2).mask, QStringLiteral("k1"));
    }

    void eachKindOfChannelHasItsOwnOp() {
        const auto channels = graph::channelsOf(clip());
        QCOMPARE(graph::editOf("c1", channels.at(0), 1.0).value("op").toString(), QStringLiteral("set"));
        const QJsonObject effect = graph::editOf("c1", channels.at(1), 1.0);
        QCOMPARE(effect.value("op").toString(), QStringLiteral("fx-set"));
        QCOMPARE(effect.value("index").toInt(), 0);
        const QJsonObject mask = graph::editOf("c1", channels.at(2), 1.0);
        QCOMPARE(mask.value("op").toString(), QStringLiteral("mask-set"));
        QCOMPARE(mask.value("name").toString(), QStringLiteral("k1"));
    }

    void aKeyStaysBetweenItsNeighboursAndOnTheClip() {
        const QList<Key> keys{{0, 0.0, {}}, {10, 1.0, {}}, {20, 2.0, {}}};
        QCOMPARE(graph::moved(keys, 1, 15, 5.0, 49).at(1).frame, 15);
        QCOMPARE(graph::moved(keys, 1, 40, 5.0, 49).at(1).frame, 19);
        QCOMPARE(graph::moved(keys, 1, -4, 5.0, 49).at(1).frame, 1);
        QCOMPARE(graph::moved(keys, 2, 90, 5.0, 49).at(2).frame, 48);
        QCOMPARE(graph::moved(keys, 0, -9, 5.0, 49).at(0).frame, 0);
        QCOMPARE(graph::moved(keys, 1, 15, 5.0, 49).at(1).value, 5.0);
    }

    void itSaysWhyThereIsNothingToDraw() {
        EditorBus bus;
        GraphEditor editor(nullptr, &bus);
        QVERIFY(editor.message().contains(QStringLiteral("Select a clip")));
        QJsonObject still = clip();
        still.remove("effects");
        still.remove("masks");
        still.insert("props", QJsonObject{{"x", 0.5}});
        bus.setSnapshot(QJsonObject{{"revision", 1}}, timeline(still), 24.0, QSize(1920, 1080));
        bus.setSelection({QStringLiteral("c1")});
        QVERIFY(editor.message().contains(QStringLiteral("Nothing on this clip moves")));
        QVERIFY(editor.channelLabels().isEmpty());
    }

    void itShowsTheSelectedClipsNumbers() {
        Harness h;
        QCOMPARE(h.editor.channelLabels(), (QStringList{"x", "blur.gaussian · radius", "k1 · feather"}));
        QCOMPARE(h.editor.selectedChannel(), 0);
        h.bus.setSelection({});
        QVERIFY(h.editor.channelLabels().isEmpty());
    }

    void draggingAKeyIsOneEditWhenItEnds() {
        Harness h;
        const QPointF from = h.editor.keyPoint(0, 1);
        const QPointF to = h.editor.plotPoint(0, 30.0, 0.25);
        QTest::mousePress(&h.editor, Qt::LeftButton, Qt::NoModifier, from.toPoint());
        QMouseEvent move(QEvent::MouseMove, to, h.editor.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(&h.editor, &move);
        QCOMPARE(h.edits.size(), 0);
        // A state arriving during a drag does not take the key from under it.
        h.set(clip());
        QCOMPARE(h.editor.selectedKey(), 1);
        QTest::mouseRelease(&h.editor, Qt::LeftButton, Qt::NoModifier, to.toPoint());
        QCOMPARE(h.edits.size(), 1);
        const QJsonArray keys = h.lastKeys("x");
        QCOMPARE(keys.size(), 3);
        QCOMPARE(keys.at(1).toObject().value("at").toInt(), 30);
        QVERIFY(std::abs(keys.at(1).toObject().value("value").toDouble() - 0.25) < 0.01);
        QCOMPARE(keys.at(0).toObject().value("value").toDouble(), 0.0);
    }

    void aKeyCannotPassItsNeighbour() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 1), h.editor.plotPoint(0, 60.0, 0.25));
        QCOMPARE(h.lastKeys("x").at(1).toObject().value("at").toInt(), 47);
    }

    void shiftMovesAKeyOneWay() {
        Harness h;
        const QPointF from = h.editor.keyPoint(0, 1);
        h.drag(from, from + QPointF(120, 9), Qt::ShiftModifier);
        const QJsonObject across = h.lastKeys("x").at(1).toObject();
        QVERIFY(across.value("at").toInt() > 24);
        QCOMPARE(across.value("value").toDouble(), 0.5);
        Harness v;
        const QPointF start = v.editor.keyPoint(0, 1);
        v.drag(start, start + QPointF(9, 60), Qt::ShiftModifier);
        const QJsonObject down = v.lastKeys("x").at(1).toObject();
        QCOMPARE(down.value("at").toInt(), 24);
        QVERIFY(down.value("value").toDouble() < 0.5);
    }

    void clickingAKeyOnAnotherCurveTakesThatCurve() {
        Harness h;
        h.drag(h.editor.keyPoint(2, 1), h.editor.keyPoint(2, 1));
        QCOMPARE(h.editor.selectedChannel(), 2);
        QCOMPARE(h.editor.selectedKey(), 1);
        QCOMPARE(h.edits.size(), 0);
    }

    void pressingAKeyWithoutMovingChangesNothing() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 0), h.editor.keyPoint(0, 0));
        QCOMPARE(h.edits.size(), 0);
    }

    void aKeyDoesNotJumpToWhereItWasTakenHold() {
        Harness h;
        // Taken a few points off its middle and carried straight across.
        const QPointF grip = h.editor.keyPoint(0, 1) + QPointF(4, 3);
        h.drag(grip, grip + QPointF(h.editor.plotPoint(0, 30.0, 0.5).x() - h.editor.keyPoint(0, 1).x(), 0));
        const QJsonObject moved = h.lastKeys("x").at(1).toObject();
        QCOMPARE(moved.value("at").toInt(), 30);
        QVERIFY(std::abs(moved.value("value").toDouble() - 0.5) < 0.01);
    }

    void aHandleShapesTheCurveAsTheDocumentWritesIt() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 0), h.editor.keyPoint(0, 0));
        const auto handles = h.editor.handlePoints();
        QVERIFY(handles);
        h.drag(handles->first, h.editor.plotPoint(0, 6.0, 0.4));
        QCOMPARE(h.edits.size(), 1);
        const QString interp = h.lastKeys("x").at(0).toObject().value("interp").toString();
        QVERIFY2(QRegularExpression(QStringLiteral("^-?[0-9.]+,-?[0-9.]+,-?[0-9.]+,-?[0-9.]+$")).match(interp).hasMatch(), qPrintable(interp));
        const auto shaped = graph::handlesOf(interp);
        QVERIFY(std::abs(shaped->x1 - 0.25) < 0.02);
        QVERIFY(std::abs(shaped->y1 - 0.8) < 0.02);
        // The other handle is where it was.
        QVERIFY(std::abs(shaped->x2 - 2.0 / 3.0) < 1e-3);
    }

    void aHoldOrTheLastKeyHasNoHandles() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 2), h.editor.keyPoint(0, 2));
        QCOMPARE(h.editor.selectedKey(), 2);
        QVERIFY(!h.editor.handlePoints());
        h.drag(h.editor.keyPoint(0, 0), h.editor.keyPoint(0, 0));
        h.editor.setInterp(QStringLiteral("hold"));
        QVERIFY(!h.editor.handlePoints());
        QCOMPARE(h.lastKeys("x").at(0).toObject().value("interp").toString(), QStringLiteral("hold"));
    }

    void doubleClickingAddsAKeyOnTheCurve() {
        Harness h;
        QTest::mouseDClick(&h.editor, Qt::LeftButton, Qt::NoModifier, h.editor.plotPoint(0, 12.0, 0.1).toPoint());
        QCOMPARE(h.edits.size(), 1);
        const QJsonArray keys = h.lastKeys("x");
        QCOMPARE(keys.size(), 4);
        QCOMPARE(keys.at(1).toObject().value("at").toInt(), 12);
        QVERIFY(std::abs(keys.at(1).toObject().value("value").toDouble() - 0.25) < 1e-9);
        QCOMPARE(h.editor.selectedKey(), 1);
    }

    void deleteTakesAKeyAndTheLastOneLeavesItsValue() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 1), h.editor.keyPoint(0, 1));
        QTest::keyClick(&h.editor, Qt::Key_Delete);
        QCOMPARE(h.lastKeys("x").size(), 2);
        QCOMPARE(h.editor.selectedKey(), -1);

        QJsonObject one = clip();
        one.insert("props", QJsonObject{{"x", QJsonArray{key(5, 0.75)}}});
        Harness s;
        s.set(one);
        s.drag(s.editor.keyPoint(0, 0), s.editor.keyPoint(0, 0));
        QTest::keyClick(&s.editor, Qt::Key_Backspace);
        const QJsonObject op = s.edits.last().at(0).toJsonArray().first().toObject();
        QCOMPARE(op.value("props").toObject().value("x").toDouble(), 0.75);
    }

    void theRulerSeeksToWholeFramesOfTheClip() {
        Harness h;
        QTest::mouseClick(&h.editor, Qt::LeftButton, Qt::NoModifier, h.editor.rulerPoint(12.0).toPoint());
        QCOMPARE(h.seeks.size(), 1);
        QVERIFY(std::abs(h.seeks.last().at(0).toDouble() - (2.0 + 12.0 / 24.0)) < 1e-9);
        QCOMPARE(h.edits.size(), 0);
    }

    void aRefusedEditIsTakenBack() {
        Harness h;
        h.drag(h.editor.keyPoint(0, 1), h.editor.plotPoint(0, 30.0, 0.25));
        QCOMPARE(h.edits.size(), 1);
        h.bus.dispatch(QJsonObject{{"type", "error"}, {"message", "no"}});
        const QPointF back = h.editor.keyPoint(0, 1);
        QVERIFY(std::abs(back.x() - h.editor.plotPoint(0, 24.0, 0.5).x()) < 0.01);
    }

    void aNewStateKeepsWhatIsSelected() {
        Harness h;
        h.drag(h.editor.keyPoint(1, 1), h.editor.keyPoint(1, 1));
        QCOMPARE(h.editor.selectedChannel(), 1);
        h.set(clip());
        QCOMPARE(h.editor.selectedChannel(), 1);
        QCOMPARE(h.editor.selectedKey(), 1);
    }

    void itComesUpAsATrayOverTheTimeline() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        QAction *action = window.m_commands->action(QStringLiteral("view.graph"));
        QVERIFY(action);
        QVERIFY(action->isCheckable());
        action->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Graph);
        QVERIFY(action->isChecked());
        QCOMPARE(window.m_shell->widgetOf(StageShell::Tray::Graph), static_cast<QWidget *>(window.m_graphEditor));
        // It is a panel over the review, not a workspace of its own.
        QCOMPARE(window.bus()->workspace(), QStringLiteral("Review"));
        QVERIFY(window.m_commands->action(QStringLiteral("workspace.review"))->isChecked());
        // Finish takes its place, and it unchecks.
        window.m_commands->action(QStringLiteral("workspace.finish"))->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Finish);
        QVERIFY(!action->isChecked());
        action->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::Graph);
        action->trigger();
        QCOMPARE(window.m_shell->tray(), StageShell::Tray::None);
    }

    void itsEditsReachTheWindow() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_lastCommand = {};
        emit window.m_graphEditor->editRequested(QJsonArray{QJsonObject{{"op", "set"}, {"clip", "c1"}, {"props", QJsonObject{{"x", 0.5}}}}});
        QVERIFY(!window.m_lastCommand.isEmpty());
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::GraphTest)

#include "graph_test.moc"
