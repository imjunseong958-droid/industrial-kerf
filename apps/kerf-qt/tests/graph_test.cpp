// Tests for the graph editor: the curves it draws against the compositor's,
// what it reads of a clip, what a drag, a box, a double-click, Delete, the wheel
// and the ruler ask for, and a motion path on the picture.

#include "fixtures.h"
#include "test_registry.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "graph_editor.h"
#include "graph_model.h"
#include "main_window.h"
#include "ui/stage_shell.h"
#include "viewer_widget.h"

#include <QAction>
#include <QJsonArray>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QSettings>
#include <QSignalSpy>
#include <QKeyEvent>
#include <QTest>
#include <QWheelEvent>

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
    void click(QPointF at, Qt::KeyboardModifiers modifiers = Qt::NoModifier) { QTest::mouseClick(&editor, Qt::LeftButton, modifiers, at.toPoint()); }
    void scroll(QPointF at, QPoint angle, Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
        QWheelEvent wheel(at, editor.mapToGlobal(at), QPoint(), angle, Qt::NoButton, modifiers, Qt::NoScrollPhase, false);
        QApplication::sendEvent(&editor, &wheel);
    }
    // Every op of the last edit, in order, by what it sets.
    QJsonArray lastOps() const { return edits.last().at(0).toJsonArray(); }
};

// A clip of `frames` frames with keys every `every` frames on x.
QJsonObject dense(int frames, int every) {
    QJsonArray keys;
    for (int frame = 0; frame < frames; frame += every) keys.append(key(frame, frame % (2 * every) == 0 ? 0.0 : 1.0));
    return {{"id", "c1"}, {"name", "Long"}, {"timelineStart", 0.0}, {"durationFrames", frames}, {"props", QJsonObject{{"x", keys}}}};
}

// A viewer showing a path through three points.
struct PathStage {
    ViewerWidget viewer;
    QSignalSpy edits{&viewer, &ViewerWidget::editRequested};

    PathStage() {
        viewer.resize(960, 600);
        viewer.setProjectInfo(1920, 1080);
        viewer.setMotionPath(QStringLiteral("c1"), QStringLiteral("0,0 0.25,-0.25 0.5,0"), QPointF(0.5, 0.5), 0.5);
    }
    void drag(QPointF from, QPointF to) {
        QTest::mousePress(&viewer, Qt::LeftButton, Qt::NoModifier, from.toPoint());
        QMouseEvent move(QEvent::MouseMove, to, viewer.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(&viewer, &move);
        QTest::mouseRelease(&viewer, Qt::LeftButton, Qt::NoModifier, to.toPoint());
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

    // The same table holds the compositor to these, in kerf_model's path.rs.
    void pathsAreTheCompositorsPaths() {
        const struct {
            const char *path;
            double aspect;
            double along;
            double x;
            double y;
            double heading;
        } table[] = {
            {"0,0 1,0", 1.0, 0.25, 0.25, 0.0, 0.0},
            {"0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0, 0.0, 0.0, 0.0, 40.186380},
            {"0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0, 0.2, 0.138852744, 0.255491011, 37.182011},
            {"0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0, 0.5, 0.377158462, 0.044861162, -46.192624},
            {"0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0, 0.8, 0.637278976, -0.006712763, 34.546793},
            {"0,0 0.2,0.3 0.5,-0.1 0.8,0.2", 16.0 / 9.0, 1.0, 0.8, 0.2, 29.384498},
            {"0,0 0.01,0 0.02,0 1,0", 1.0, 0.5, 0.499950046, 0.0, 0.0},
            {"0,0 0.5,0 0.5,0.5", 2.0, 0.5, 0.381055017, -0.061333311, 0.422966},
            {"0,0 0.5,0 0.5,0.5", 2.0, 0.9, 0.508284207, 0.348099501, 99.522964},
            {"-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2", 9.0 / 16.0, 0.1, -0.251897404, -0.034195522, -59.751285},
            {"-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2", 9.0 / 16.0, 0.37, 0.146873937, -0.219570365, 84.473801},
            {"-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2", 9.0 / 16.0, 0.66, 0.182237509, 0.237851333, 68.262319},
            {"-0.4,0.1 0.1,-0.3 0.2,0.25 0.45,0.05 0.3,-0.2", 9.0 / 16.0, 0.95, 0.354996038, -0.127356395, -115.521511},
            {"0.3,0.3 0.3,0.3", 1.0, 0.5, 0.3, 0.3, 0.0},
        };
        for (const auto &row : table) {
            const auto points = graph::parsePath(QString::fromLatin1(row.path));
            QVERIFY2(points, row.path);
            const auto at = graph::MotionPath(*points, row.aspect).at(row.along);
            const QString where = QStringLiteral("%1 at %2: %3,%4 heading %5").arg(QString::fromLatin1(row.path)).arg(row.along).arg(at.x).arg(at.y).arg(at.heading);
            QVERIFY2(std::abs(at.x - row.x) < 1e-6, qPrintable(where));
            QVERIFY2(std::abs(at.y - row.y) < 1e-6, qPrintable(where));
            QVERIFY2(std::abs(at.heading - row.heading) < 1e-3, qPrintable(where));
        }
    }

    void aPathReadsAndWritesAsTheDocumentDoes() {
        const auto points = graph::parsePath(QStringLiteral("0,0 0.5,-0.25"));
        QVERIFY(points);
        QCOMPARE(*points, (QList<QPointF>{{0.0, 0.0}, {0.5, -0.25}}));
        for (const char *refused : {"", "0,0", "0 0", "0,0 x,1", "0,0 11,0", "0,0 nan,0", "0,0 1,2,3"})
            QVERIFY2(!graph::parsePath(QString::fromLatin1(refused)), refused);
        QCOMPARE(graph::pathText({{0.123456, -0.5}, {0.0, 12.0}, {-0.00001, 1.0}}), QStringLiteral("0.1235,-0.5 0,10 0,1"));
    }

    void aPlacementFollowsTheClipsPath() {
        const QSizeF canvas(1920, 1080);
        const QJsonObject onPath{{"props", QJsonObject{{"path", "0,0 0.5,0"}, {"along", 0.5}}}};
        QVERIFY(std::abs(picturePlacement(onPath, canvas, true, canvas, 0.0).map(QPointF(0.5, 0.5)).x() - 1440.0) < 1e-6);
        // Facing the way it goes: straight down turns it a quarter clockwise.
        const QJsonObject turned{{"props", QJsonObject{{"path", "0,0 0,0.5"}, {"along", 1.0}, {"orient", true}}}};
        const QPointF right = picturePlacement(turned, canvas, true, canvas, 0.0).map(QPointF(1.0, 0.5));
        QVERIFY(std::abs(right.x() - 960.0) < 1e-3);
        QVERIFY(std::abs(right.y() - (1080.0 + 960.0)) < 1e-3);
        // What is keyed goes by its curve, as the compositor reads it.
        const QJsonObject eased{{"along", QJsonArray{key(0, 0.0, "ease-in"), key(24, 1.0)}}};
        QCOMPARE(propertyAt(eased, "along", 0.5, 0.0), 0.25);
        // The path starts from where the clip otherwise is.
        const QPointF origin = pathOrigin(QJsonObject{{"props", QJsonObject{{"x", 0.1}, {"y", -0.2}, {"anchor_x", 0.25}}}}, canvas, true, canvas, 0.0);
        QVERIFY(std::abs(origin.x() - 0.85) < 1e-9);
        QVERIFY(std::abs(origin.y() - 0.3) < 1e-9);
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

    void keysMovedTogetherKeepTheirSpacingBetweenTheirNeighbours() {
        const QList<Key> keys{{0, 0.0, {}}, {10, 1.0, {}}, {14, 2.0, {}}, {30, 3.0, {}}};
        // The two in the middle: back to just past the first, on to just before the last.
        QCOMPARE(graph::roomToMove(keys, {1, 2}, 49), (std::pair<qint64, qint64>{-9, 15}));
        // One of them is the other's neighbour no longer.
        QCOMPARE(graph::roomToMove(keys, {2}, 49), (std::pair<qint64, qint64>{-3, 15}));
        // The ends of the clip stop the first and the last.
        QCOMPARE(graph::roomToMove(keys, {0}, 49), (std::pair<qint64, qint64>{0, 9}));
        QCOMPARE(graph::roomToMove(keys, {3}, 49), (std::pair<qint64, qint64>{-15, 18}));
        QCOMPARE(graph::roomToMove(keys, {0, 1, 2, 3}, 49), (std::pair<qint64, qint64>{0, 18}));
        // A key already past the end can stay.
        QCOMPARE(graph::roomToMove({{60, 0.0, {}}}, {0}, 49), (std::pair<qint64, qint64>{-60, 0}));
        const QList<Key> moved = graph::shifted(keys, {1, 2}, 5, 0.5);
        QCOMPARE(moved.at(0).frame, 0);
        QCOMPARE(moved.at(1).frame, 15);
        QCOMPARE(moved.at(2).frame, 19);
        QCOMPARE(moved.at(3).frame, 30);
        QCOMPARE(moved.at(1).value, 1.5);
        QCOMPARE(moved.at(3).value, 3.0);
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

    // ------------------------------------------------------------ the time axis

    void itShowsTheWholeClipFirst() {
        Harness h;
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 48.0}));
    }

    void scrollingZoomsAroundThePointer() {
        Harness h;
        const QPointF at = h.editor.plotPoint(0, 24.0, 0.25);
        const auto [start, end] = h.editor.viewFrames();
        h.scroll(at, QPoint(0, 240));
        const auto [zoomedStart, zoomedEnd] = h.editor.viewFrames();
        QVERIFY(zoomedEnd - zoomedStart < end - start);
        // The frame under the pointer is where it was.
        QVERIFY(std::abs(h.editor.plotPoint(0, 24.0, 0.25).x() - at.x()) < 0.5);
        // Dense keys stand further apart for it.
        Harness dense;
        dense.set(::kerf::dense(2400, 10));
        const double before = dense.editor.keyPoint(0, 2).x() - dense.editor.keyPoint(0, 1).x();
        dense.scroll(dense.editor.keyPoint(0, 1), QPoint(0, 1200));
        QVERIFY(dense.editor.keyPoint(0, 2).x() - dense.editor.keyPoint(0, 1).x() > 5.0 * before);
        // And back out it stops at the whole clip.
        h.scroll(at, QPoint(0, -2400));
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 48.0}));
    }

    void itStopsZoomingWhenAFrameIsPlain() {
        Harness h;
        h.scroll(h.editor.plotPoint(0, 24.0, 0.25), QPoint(0, 12000));
        const auto [start, end] = h.editor.viewFrames();
        QCOMPARE(end - start, 8.0);
    }

    void scrollingSidewaysPansAndStopsAtTheClipsEnds() {
        Harness h;
        h.editor.frameSelection();
        const QPointF at = h.editor.plotPoint(0, 24.0, 0.25);
        h.scroll(at, QPoint(0, 480));
        const auto [start, end] = h.editor.viewFrames();
        h.scroll(at, QPoint(-120, 0));
        const auto [later, laterEnd] = h.editor.viewFrames();
        QVERIFY(later > start);
        QCOMPARE(laterEnd - later, end - start);
        // Shift turns a plain wheel into the same thing.
        h.scroll(at, QPoint(0, -120), Qt::ShiftModifier);
        QVERIFY(h.editor.viewFrames().first > later);
        h.scroll(at, QPoint(12000, 0));
        QCOMPARE(h.editor.viewFrames().first, 0.0);
        h.scroll(at, QPoint(-12000, 0));
        QCOMPARE(h.editor.viewFrames().second, 48.0);
    }

    void theMiddleButtonPans() {
        Harness h;
        h.scroll(h.editor.plotPoint(0, 24.0, 0.25), QPoint(0, 480));
        const auto [start, end] = h.editor.viewFrames();
        const QPointF from = h.editor.plotPoint(0, 24.0, 0.25);
        const QPointF to = from + QPointF(-50, 7);
        QTest::mousePress(&h.editor, Qt::MiddleButton, Qt::NoModifier, from.toPoint());
        QMouseEvent move(QEvent::MouseMove, to, h.editor.mapToGlobal(to), Qt::NoButton, Qt::MiddleButton, Qt::NoModifier);
        QApplication::sendEvent(&h.editor, &move);
        QTest::mouseRelease(&h.editor, Qt::MiddleButton, Qt::NoModifier, to.toPoint());
        QVERIFY(h.editor.viewFrames().first > start);
        QCOMPARE(h.editor.viewFrames().second - h.editor.viewFrames().first, end - start);
        QCOMPARE(h.edits.size(), 0);
    }

    void aKeyCanBeFramed() {
        Harness h;
        h.set(::kerf::dense(2400, 240));
        // The key at frame 480, with the keys either side for company.
        h.click(h.editor.keyPoint(0, 2));
        QCOMPARE(h.editor.selectedKey(), 2);
        QTest::keyClick(&h.editor, Qt::Key_F);
        const auto [start, end] = h.editor.viewFrames();
        QVERIFY(start < 240.0 && end > 720.0);
        QVERIFY(end - start < 700.0);
        QVERIFY(std::abs((start + end) / 2.0 - 480.0) < 1.0);
        // The key is in the middle of the plot, and can be taken there.
        const QPointF middle = h.editor.keyPoint(0, 2);
        QVERIFY(std::abs(middle.x() - h.editor.plotPoint(0, (start + end) / 2.0, 0.0).x()) < 0.5);
        h.click(middle);
        QCOMPARE(h.editor.selectedKey(), 2);
        // Shift+F is the whole clip again.
        QTest::keyClick(&h.editor, Qt::Key_F, Qt::ShiftModifier);
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 2399.0}));
    }

    void framingAKeyWithFarNeighboursStillBringsItCloser() {
        Harness h;
        QJsonObject far = clip();
        far.insert("props", QJsonObject{{"x", QJsonArray{key(0, 0.0), key(24, 0.5), key(48, 0.0)}}});
        h.set(far);
        h.click(h.editor.keyPoint(0, 1));
        h.editor.frameSelection();
        const auto [start, end] = h.editor.viewFrames();
        QVERIFY(end - start < 48.0);
        QVERIFY(std::abs((start + end) / 2.0 - 24.0) < 0.5);
    }

#ifdef Q_OS_MACOS
    // With a Hangul input source on, the F key comes as a letter of its own alphabet.
    void theFKeyWorksWhateverAlphabetIsOn() {
        Harness h;
        h.set(::kerf::dense(2400, 240));
        h.click(h.editor.keyPoint(0, 2));
        const auto press = [&h](Qt::KeyboardModifiers modifiers) {
            QKeyEvent key(QEvent::KeyPress, 0x3139, modifiers, 0, 0x03, 0, QStringLiteral("\u3139"));
            QApplication::sendEvent(&h.editor, &key);
        };
        press(Qt::NoModifier);
        QVERIFY(h.editor.viewFrames().second - h.editor.viewFrames().first < 2000.0);
        press(Qt::ShiftModifier);
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 2399.0}));
        // Another key of that alphabet does nothing.
        h.editor.frameSelection();
        const auto framed = h.editor.viewFrames();
        QKeyEvent other(QEvent::KeyPress, 0x3141, Qt::NoModifier, 0, 0x00, 0, QStringLiteral("\u3141"));
        QApplication::sendEvent(&h.editor, &other);
        QCOMPARE(h.editor.viewFrames(), framed);
    }
#endif

    void aDragWritesFourPlaces() {
        Harness h;
        const QPointF from = h.editor.keyPoint(0, 1);
        h.drag(from, from + QPointF(0, 37.3));
        const double value = h.lastKeys("x").at(1).toObject().value("value").toDouble();
        QCOMPARE(std::round(value * 10000.0) / 10000.0, value);
        // The keys it did not move keep every place they had.
        QJsonObject exact = clip();
        exact.insert("props", QJsonObject{{"x", QJsonArray{key(0, 0.123456789), key(24, 0.5), key(48, 0.0)}}});
        Harness e;
        e.set(exact);
        const QPointF grip = e.editor.keyPoint(0, 1);
        e.drag(grip, grip + QPointF(0, 20));
        QCOMPARE(e.lastKeys("x").at(0).toObject().value("value").toDouble(), 0.123456789);
    }

    void framingSeveralKeysShowsThemAll() {
        Harness h;
        h.set(::kerf::dense(2400, 240));
        h.click(h.editor.keyPoint(0, 3));
        h.click(h.editor.keyPoint(0, 5), Qt::ShiftModifier);
        h.editor.frameSelection();
        const auto [start, end] = h.editor.viewFrames();
        QVERIFY(start < 720.0 && start > 600.0);
        QVERIFY(end > 1200.0 && end < 1320.0);
        // Nothing selected, it shows the whole clip.
        h.click(h.editor.plotPoint(0, start + 1.0, 0.5));
        QCOMPARE(h.editor.selectedKey(), -1);
        h.editor.frameSelection();
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 2399.0}));
    }

    void aDragWhileZoomedMovesInFrames() {
        Harness h;
        h.set(::kerf::dense(2400, 240));
        h.click(h.editor.keyPoint(0, 2));
        QTest::keyClick(&h.editor, Qt::Key_F);
        const QPointF from = h.editor.keyPoint(0, 2);
        h.drag(from, QPointF(h.editor.plotPoint(0, 500.0, 0.0).x(), from.y()));
        QCOMPARE(h.lastKeys("x").at(2).toObject().value("at").toInt(), 500);
    }

    void aKeyOutOfViewIsNotThereToTake() {
        Harness h;
        h.click(h.editor.keyPoint(0, 2));
        h.scroll(h.editor.keyPoint(0, 0), QPoint(0, 1200));
        // It is off the plot's right, however the pointer gets at where it would be.
        QVERIFY(h.editor.keyPoint(0, 2).x() > h.editor.width());
        const QPointF inPlot = h.editor.plotPoint(0, h.editor.viewFrames().second - 0.5, 0.0);
        h.click(inPlot);
        QCOMPARE(h.editor.selectedKey(), -1);
        QCOMPARE(h.edits.size(), 0);
    }

    void theViewOutlastsANewStateButNotAClip() {
        Harness h;
        h.scroll(h.editor.plotPoint(0, 24.0, 0.25), QPoint(0, 480));
        const auto zoomed = h.editor.viewFrames();
        h.set(clip());
        QCOMPARE(h.editor.viewFrames(), zoomed);
        // Whole stays whole as the clip grows.
        Harness w;
        QJsonObject longer = clip();
        longer.insert("durationFrames", 97);
        w.set(longer);
        QCOMPARE(w.editor.viewFrames(), (std::pair<double, double>{0.0, 96.0}));
        // Another clip is seen whole.
        QJsonObject other = clip();
        other.insert("id", "c2");
        h.set(other);
        h.bus.setSelection({QStringLiteral("c2")});
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 48.0}));
    }

    void theBarsButtonsFrameAndFit() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.editor.findChild<QAbstractButton *>(QStringLiteral("graphFrame"))->click();
        QVERIFY(h.editor.viewFrames().second < 48.0);
        h.editor.findChild<QAbstractButton *>(QStringLiteral("graphFit"))->click();
        QCOMPARE(h.editor.viewFrames(), (std::pair<double, double>{0.0, 48.0}));
    }

    // ------------------------------------------------------------- many keys

    void shiftClickingAddsAKeyAndTakesItAway() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 2), Qt::ShiftModifier);
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{0, 2}));
        QCOMPARE(h.editor.selectedKey(), 2);
        h.click(h.editor.keyPoint(0, 2), Qt::ShiftModifier);
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{0}));
        QCOMPARE(h.editor.selectedKey(), 0);
        h.click(h.editor.keyPoint(0, 0), Qt::ShiftModifier);
        QVERIFY(h.editor.selectedKeys(0).isEmpty());
        QCOMPARE(h.editor.selectedKey(), -1);
        QCOMPARE(h.edits.size(), 0);
    }

    void aPlainClickOnASelectedKeyLeavesJustThatOne() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 1), Qt::ShiftModifier);
        h.click(h.editor.keyPoint(0, 0));
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{0}));
    }

    void aBoxSelectsTheKeysInIt() {
        Harness h;
        // From before x's middle key to past the clip's end, whichever number a key is of: the
        // box starts in the margin, since a key can sit on the plot's edge.
        const QPointF middle = h.editor.keyPoint(0, 1);
        const QPointF last = h.editor.keyPoint(0, 2);
        h.drag(middle + QPointF(-6, -6), last + QPointF(6, 6));
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{1, 2}));
        QCOMPARE(h.editor.selectedKeys(1), (QList<int>{1}));
        QVERIFY(h.editor.selectedKeys(2).isEmpty());
        QCOMPARE(h.edits.size(), 0);
        // With Shift it adds to what is selected.
        const QPointF first = h.editor.keyPoint(0, 0);
        h.drag(first + QPointF(-6, 6), first + QPointF(6, -6), Qt::ShiftModifier);
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{0, 1, 2}));
        // Without, it starts again; an empty box leaves nothing.
        const QPointF empty = h.editor.plotPoint(0, 12.0, 0.45);
        h.drag(empty, empty + QPointF(8, 8));
        QVERIFY(h.editor.selectedKeys(0).isEmpty());
        QCOMPARE(h.editor.selectedKey(), -1);
    }

    void aBoxReachesAcrossNumbers() {
        Harness h;
        // Everything.
        const QRectF plot(h.editor.plotPoint(0, 0.0, 0.0), h.editor.plotPoint(0, 48.0, 0.0));
        h.drag(QPointF(plot.left() - 8, 60), QPointF(plot.right() + 8, h.editor.height() - 2));
        QCOMPARE(h.editor.selectedKeys(0).size(), 3);
        QCOMPARE(h.editor.selectedKeys(1).size(), 2);
        QCOMPARE(h.editor.selectedKeys(2).size(), 2);
    }

    void selectedKeysMoveTogetherAsOneEdit() {
        Harness h;
        h.click(h.editor.keyPoint(0, 1));
        h.click(h.editor.keyPoint(2, 1), Qt::ShiftModifier);
        QCOMPARE(h.editor.selectedChannel(), 2);
        const QPointF from = h.editor.keyPoint(2, 1);
        const double frames = 6.0;
        const QPointF to(h.editor.plotPoint(2, 12.0 + frames, 0.0).x(), from.y() - 30.0);
        h.drag(from, to);
        QCOMPARE(h.edits.size(), 1);
        const QJsonArray ops = h.lastOps();
        QCOMPARE(ops.size(), 2);
        const QJsonArray xKeys = ops.at(0).toObject().value("props").toObject().value("x").toObject().value("keys").toArray();
        const QJsonArray featherKeys = ops.at(1).toObject().value("params").toObject().value("feather").toObject().value("keys").toArray();
        QCOMPARE(xKeys.at(1).toObject().value("at").toInt(), 30);
        QCOMPARE(featherKeys.at(1).toObject().value("at").toInt(), 18);
        // Each up the plot by the same distance, on its own scale.
        QVERIFY(xKeys.at(1).toObject().value("value").toDouble() > 0.5);
        QVERIFY(featherKeys.at(1).toObject().value("value").toDouble() > 9.0);
        // The keys not selected are where they were.
        QCOMPARE(xKeys.at(0).toObject().value("at").toInt(), 0);
        QCOMPARE(xKeys.at(2).toObject().value("at").toInt(), 48);
        QCOMPARE(xKeys.at(2).toObject().value("value").toDouble(), 0.0);
    }

    void theTightestNeighbourStopsThemAll() {
        Harness h;
        // x's middle key has room back to frame 1, the feather's second back to frame 1 too,
        // but forward the feather's (12) could go to the end and x's only to 47.
        h.click(h.editor.keyPoint(0, 1));
        h.click(h.editor.keyPoint(2, 1), Qt::ShiftModifier);
        const QPointF from = h.editor.keyPoint(2, 1);
        h.drag(from, QPointF(h.editor.plotPoint(2, 48.0, 0.0).x(), from.y()));
        const QJsonArray ops = h.lastOps();
        const QJsonArray xKeys = ops.at(0).toObject().value("props").toObject().value("x").toObject().value("keys").toArray();
        const QJsonArray featherKeys = ops.at(1).toObject().value("params").toObject().value("feather").toObject().value("keys").toArray();
        QCOMPARE(xKeys.at(1).toObject().value("at").toInt(), 47);
        // The same 23 frames, so they stay as far apart as they were.
        QCOMPARE(featherKeys.at(1).toObject().value("at").toInt(), 35);
        QVERIFY(featherKeys.at(1).toObject().value("at").toInt() > featherKeys.at(0).toObject().value("at").toInt());
    }

    void keysSelectedSideBySideGoPastEachOthersPlaces() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 1), Qt::ShiftModifier);
        h.drag(h.editor.keyPoint(0, 1), h.editor.plotPoint(0, 34.0, 0.5));
        const QJsonArray keys = h.lastKeys("x");
        QCOMPARE(keys.at(0).toObject().value("at").toInt(), 10);
        QCOMPARE(keys.at(1).toObject().value("at").toInt(), 34);
        QCOMPARE(keys.at(2).toObject().value("at").toInt(), 48);
    }

    void aKeyStaysInTheRangeItsNumberAllows() {
        EditorBus bus;
        GraphEditor editor(nullptr, &bus);
        editor.resize(1000, 320);
        QSignalSpy edits(&editor, &GraphEditor::editRequested);
        const QJsonObject track{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{clip()}}};
        const QJsonObject project{{"tracks", QJsonArray{track}},
                                  {"properties", QJsonObject{{"video", QJsonArray{QJsonObject{{"name", "x"}, {"min", -0.25}, {"max", 0.6}}}}}},
                                  {"maskProperties", QJsonArray{QJsonObject{{"name", "feather"}, {"min", 0.0}, {"max", 9.5}}}}};
        editor.setEffectCatalog(QJsonArray{QJsonObject{{"id", "blur.gaussian"}, {"params", QJsonArray{QJsonObject{{"name", "radius"}, {"min", 0.0}, {"max", 41.0}}}}}});
        bus.setSnapshot(QJsonObject{{"revision", 1}}, project, 24.0, QSize(1920, 1080));
        bus.setSelection({QStringLiteral("c1")});
        const auto drag = [&](QPointF from, QPointF to, Qt::KeyboardModifiers modifiers = Qt::NoModifier) {
            QTest::mousePress(&editor, Qt::LeftButton, modifiers, from.toPoint());
            QMouseEvent move(QEvent::MouseMove, to, editor.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, modifiers);
            QApplication::sendEvent(&editor, &move);
            QTest::mouseRelease(&editor, Qt::LeftButton, modifiers, to.toPoint());
        };
        const auto keysOf = [](const QJsonObject &op, const char *name) {
            const QJsonObject values = op.contains("props") ? op.value("props").toObject() : op.value("params").toObject();
            return values.value(QString::fromLatin1(name)).toObject().value("keys").toArray();
        };
        // Far past the top of the plot, and down past the bottom: each held at its limit.
        const QPointF top = editor.keyPoint(0, 1);
        drag(top, top + QPointF(0, -400));
        QCOMPARE(keysOf(edits.last().at(0).toJsonArray().first().toObject(), "x").at(1).toObject().value("value").toDouble(), 0.6);
        const QPointF again = editor.keyPoint(0, 1);
        drag(again, again + QPointF(0, 2000));
        QCOMPARE(keysOf(edits.last().at(0).toJsonArray().first().toObject(), "x").at(1).toObject().value("value").toDouble(), -0.25);
        // Moved with others, each stops at its own: an effect's and a mask's range count too.
        QTest::mouseClick(&editor, Qt::LeftButton, Qt::NoModifier, editor.channelPoint(1).toPoint());
        QTest::mouseClick(&editor, Qt::LeftButton, Qt::NoModifier, editor.keyPoint(1, 1).toPoint());
        QTest::mouseClick(&editor, Qt::LeftButton, Qt::ShiftModifier, editor.keyPoint(2, 1).toPoint());
        const QPointF grip = editor.keyPoint(2, 1);
        drag(grip, grip + QPointF(0, -600));
        const QJsonArray ops = edits.last().at(0).toJsonArray();
        QCOMPARE(ops.size(), 2);
        QCOMPARE(keysOf(ops.at(0).toObject(), "radius").at(1).toObject().value("value").toDouble(), 41.0);
        QCOMPARE(keysOf(ops.at(1).toObject(), "feather").at(1).toObject().value("value").toDouble(), 9.5);
    }

    void aBoxKeepsTheNumberInHandWhileItCatchesOne() {
        Harness h;
        QCOMPARE(h.editor.selectedChannel(), 0);
        // Over x's middle key (24) and the feather's second (12), reaching the feather's first.
        const QPointF from = h.editor.plotPoint(0, 8.0, 0.55);
        const QPointF to = h.editor.plotPoint(0, 28.0, 0.3);
        QTest::mousePress(&h.editor, Qt::LeftButton, Qt::NoModifier, from.toPoint());
        const QPointF part = h.editor.plotPoint(0, 16.0, 0.3);
        QMouseEvent first(QEvent::MouseMove, part, h.editor.mapToGlobal(part), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(&h.editor, &first);
        // So far it has the feather's only, and that is the number in hand.
        QCOMPARE(h.editor.selectedChannel(), 2);
        QMouseEvent second(QEvent::MouseMove, to, h.editor.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(&h.editor, &second);
        QTest::mouseRelease(&h.editor, Qt::LeftButton, Qt::NoModifier, to.toPoint());
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{1}));
        QCOMPARE(h.editor.selectedKeys(2), (QList<int>{1}));
        // x again, as it was when the box began.
        QCOMPARE(h.editor.selectedChannel(), 0);
    }

    void shiftKeepsAGroupToOneWay() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 1), Qt::ShiftModifier);
        const QPointF from = h.editor.keyPoint(0, 1);
        h.drag(from, from + QPointF(120, 9), Qt::ShiftModifier);
        const QJsonArray keys = h.lastKeys("x");
        QVERIFY(keys.at(1).toObject().value("at").toInt() > 24);
        QCOMPARE(keys.at(0).toObject().value("value").toDouble(), 0.0);
        QCOMPARE(keys.at(1).toObject().value("value").toDouble(), 0.5);
    }

    void deleteTakesEveryKeySelectedAsOneEdit() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 2), Qt::ShiftModifier);
        h.click(h.editor.keyPoint(2, 1), Qt::ShiftModifier);
        h.click(h.editor.keyPoint(2, 0), Qt::ShiftModifier);
        QTest::keyClick(&h.editor, Qt::Key_Delete);
        QCOMPARE(h.edits.size(), 1);
        const QJsonArray ops = h.lastOps();
        QCOMPARE(ops.size(), 2);
        QCOMPARE(ops.at(0).toObject().value("props").toObject().value("x").toObject().value("keys").toArray().size(), 1);
        // All of a number's keys gone leaves a value.
        QCOMPARE(ops.at(1).toObject().value("params").toObject().value("feather").toDouble(), 1.0);
        QCOMPARE(h.editor.selectedKey(), -1);
    }

    void aCurveGoesToEveryKeySelected() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(0, 1), Qt::ShiftModifier);
        h.editor.setInterp(QStringLiteral("ease"));
        const QJsonArray keys = h.lastKeys("x");
        QCOMPARE(keys.at(0).toObject().value("interp").toString(), QStringLiteral("ease"));
        QCOMPARE(keys.at(1).toObject().value("interp").toString(), QStringLiteral("ease"));
        QCOMPARE(keys.at(2).toObject().value("interp").toString(), QStringLiteral("linear"));
    }

    void severalKeysHaveNoHandlesToShape() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        QVERIFY(h.editor.handlePoints());
        h.click(h.editor.keyPoint(0, 1), Qt::ShiftModifier);
        QVERIFY(!h.editor.handlePoints());
    }

    void aNewStateKeepsEveryKeySelected() {
        Harness h;
        h.click(h.editor.keyPoint(0, 0));
        h.click(h.editor.keyPoint(2, 1), Qt::ShiftModifier);
        h.set(clip());
        QCOMPARE(h.editor.selectedKeys(0), (QList<int>{0}));
        QCOMPARE(h.editor.selectedKeys(2), (QList<int>{1}));
        QCOMPARE(h.editor.selectedChannel(), 2);
        QCOMPARE(h.editor.selectedKey(), 1);
    }

    // ------------------------------------------------------------ on the picture

    void aPathIsDrawnThroughItsPoints() {
        PathStage s;
        QVERIFY(s.viewer.showsMotionPath());
        const QRectF canvas = s.viewer.pictureRect();
        const QList<QPointF> points = s.viewer.motionPathPoints();
        QCOMPARE(points.size(), 3);
        QVERIFY(QLineF(points.at(0), canvas.center()).length() < 0.01);
        QVERIFY(QLineF(points.at(1), canvas.center() + QPointF(canvas.width() * 0.25, -canvas.height() * 0.25)).length() < 0.01);
        // The curve is the compositor's, on the same canvas, and goes through every point.
        const graph::MotionPath path({{0.0, 0.0}, {0.25, -0.25}, {0.5, 0.0}}, 1920.0 / 1080.0);
        const QList<QPointF> curve = s.viewer.motionPathCurve();
        const QList<QPointF> samples = path.samples();
        QCOMPARE(curve.size(), samples.size());
        for (qsizetype index = 0; index < curve.size(); ++index) {
            const QPointF wanted = canvas.center() + QPointF(samples.at(index).x() * canvas.width(), samples.at(index).y() * canvas.height());
            QVERIFY(QLineF(curve.at(index), wanted).length() < 0.01);
        }
        for (const QPointF &point : points) {
            double nearest = 1e9;
            for (const QPointF &along : curve) nearest = std::min(nearest, QLineF(point, along).length());
            QVERIFY(nearest < 0.01);
        }
        // Halfway along by distance, here its middle point by symmetry.
        QVERIFY(QLineF(s.viewer.motionPathMarker(), points.at(1)).length() < 1.0);
    }

    void aPathShowsOnThePicture() {
        PathStage s;
        s.viewer.show();
        const QList<QPointF> points = s.viewer.motionPathPoints();
        const QImage drawn = s.viewer.grab().toImage();
        const qreal ratio = drawn.devicePixelRatio();
        const QColor fill = editorTheme().maskHandleFill;
        QCOMPARE(drawn.pixelColor((points.at(0) * ratio).toPoint()), fill);
        QCOMPARE(drawn.pixelColor((points.at(2) * ratio).toPoint()), fill);
        ViewerWidget none;
        none.resize(960, 600);
        none.setProjectInfo(1920, 1080);
        const QImage plain = none.grab().toImage();
        QVERIFY(plain.pixelColor((points.at(0) * ratio).toPoint()) != fill);
    }

    void draggingAPointIsOneEditThatWritesThePath() {
        PathStage s;
        const QList<QPointF> points = s.viewer.motionPathPoints();
        const QRectF canvas = s.viewer.pictureRect();
        const QPointF to = points.at(1) + QPointF(canvas.width() * 0.1, canvas.height() * 0.05);
        QTest::mousePress(&s.viewer, Qt::LeftButton, Qt::NoModifier, points.at(1).toPoint());
        QMouseEvent move(QEvent::MouseMove, to, s.viewer.mapToGlobal(to), Qt::NoButton, Qt::LeftButton, Qt::NoModifier);
        QApplication::sendEvent(&s.viewer, &move);
        // Nothing until it is let go, and the curve follows meanwhile.
        QCOMPARE(s.edits.size(), 0);
        QVERIFY(QLineF(s.viewer.motionPathPoints().at(1), to).length() < 2.0);
        // A state arriving during a drag does not take the point from under it.
        s.viewer.setMotionPath(QStringLiteral("c1"), QStringLiteral("0,0 0.25,-0.25 0.5,0"), QPointF(0.5, 0.5), 0.5);
        QVERIFY(QLineF(s.viewer.motionPathPoints().at(1), to).length() < 2.0);
        QTest::mouseRelease(&s.viewer, Qt::LeftButton, Qt::NoModifier, to.toPoint());
        QCOMPARE(s.edits.size(), 1);
        const QJsonArray ops = s.edits.last().at(0).toJsonArray();
        QCOMPARE(ops.size(), 1);
        const QJsonObject op = ops.first().toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("c1"));
        const auto written = graph::parsePath(op.value("props").toObject().value("path").toString());
        QVERIFY(written);
        QCOMPARE(written->size(), 3);
        QCOMPARE(written->at(0), QPointF(0.0, 0.0));
        QCOMPARE(written->at(2), QPointF(0.5, 0.0));
        QVERIFY(std::abs(written->at(1).x() - 0.35) < 0.005);
        QVERIFY(std::abs(written->at(1).y() - -0.2) < 0.005);
    }

    void aPointDoesNotJumpToWhereItWasTakenHold() {
        PathStage s;
        const QPointF point = s.viewer.motionPathPoints().at(1);
        const QPointF grip = point + QPointF(4, 3);
        const QPointF to = grip + QPointF(40, 0);
        s.drag(grip, to);
        QVERIFY(QLineF(s.viewer.motionPathPoints().at(1), point + QPointF(40, 0)).length() < 1.5);
    }

    void aClickOnAPointWritesNothing() {
        PathStage s;
        const QPointF point = s.viewer.motionPathPoints().at(0);
        s.drag(point, point + QPointF(1, 0));
        QCOMPARE(s.edits.size(), 0);
    }

    void aPathThatDoesNotReadOrAClipGoneShowsNone() {
        PathStage s;
        s.viewer.setMotionPath(QStringLiteral("c1"), QStringLiteral("0,0"), QPointF(0.5, 0.5), 0.0);
        QVERIFY(!s.viewer.showsMotionPath());
        QVERIFY(s.viewer.motionPathPoints().isEmpty());
        s.viewer.setMotionPath(QStringLiteral("c1"), QStringLiteral("0,0 1,1"), QPointF(0.5, 0.5), 0.0);
        QVERIFY(s.viewer.showsMotionPath());
        s.viewer.setMotionPath(QString(), QStringLiteral("0,0 1,1"), QPointF(0.5, 0.5), 0.0);
        QVERIFY(!s.viewer.showsMotionPath());
        // With no path, a click on the picture is the play button again.
        QSignalSpy plays(&s.viewer, &ViewerWidget::playRequested);
        QTest::mouseClick(&s.viewer, Qt::LeftButton, Qt::NoModifier, s.viewer.videoRect().center().toPoint());
        QCOMPARE(plays.size(), 1);
    }

    void aClickBesideAPointStillPlays() {
        PathStage s;
        QSignalSpy plays(&s.viewer, &ViewerWidget::playRequested);
        const QPointF away = s.viewer.motionPathPoints().at(0) + QPointF(-120, 60);
        QTest::mouseClick(&s.viewer, Qt::LeftButton, Qt::NoModifier, away.toPoint());
        QCOMPARE(plays.size(), 1);
        QCOMPARE(s.edits.size(), 0);
    }

    void aPathHoldsThePictureStill() {
        PathStage s;
        QSignalSpy scrubs(&s.viewer, &ViewerWidget::scrubbed);
        const QPointF over = s.viewer.videoRect().center() + QPointF(30, 30);
        QMouseEvent move(QEvent::MouseMove, over, s.viewer.mapToGlobal(over), Qt::NoButton, Qt::NoButton, Qt::NoModifier);
        QApplication::sendEvent(&s.viewer, &move);
        QCOMPARE(scrubs.size(), 0);
    }

    void aSelectedClipShowsItsPathInTheWindow() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        const QJsonObject onPath{{"id", "c1"},
                                 {"kind", "solid"},
                                 {"timelineStart", 0.0},
                                 {"duration", 2.0},
                                 {"durationFrames", 48},
                                 {"props", QJsonObject{{"path", "0,0 0.25,0"}, {"x", 0.1}, {"along", QJsonArray{key(0, 0.0), key(48, 1.0)}}}}};
        const QJsonObject plain{{"id", "c2"}, {"kind", "solid"}, {"timelineStart", 2.0}, {"duration", 2.0}, {"durationFrames", 48}, {"props", QJsonObject{}}};
        window.m_project = QJsonObject{{"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"clips", QJsonArray{onPath, plain}}}}}};
        window.m_selectedClip = QStringLiteral("c1");
        window.m_selectedClips = {QStringLiteral("c1")};
        window.m_connected = true;
        window.m_playhead = 1.0;
        window.updateViewerMasks();
        QVERIFY(window.m_viewer->showsMotionPath());
        const QList<QPointF> points = window.m_viewer->motionPathPoints();
        QCOMPARE(points.size(), 2);
        const QRectF canvas = window.m_viewer->pictureRect();
        QVERIFY(std::abs(points.at(0).x() - (canvas.left() + 0.6 * canvas.width())) < 0.01);
        // Halfway through the clip, halfway along.
        QVERIFY(std::abs(window.m_viewer->motionPathMarker().x() - (canvas.left() + 0.725 * canvas.width())) < 0.5);
        window.m_selectedClip = QStringLiteral("c2");
        window.m_selectedClips = {QStringLiteral("c2")};
        window.updateViewerMasks();
        QVERIFY(!window.m_viewer->showsMotionPath());
    }

    void aPointDraggedReachesTheWindow() {
        QSettings().remove(QStringLiteral("workspace"));
        const auto restore = qScopeGuard([] { QSettings().remove(QStringLiteral("workspace")); });
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.m_lastCommand = {};
        emit window.m_viewer->editRequested(QJsonArray{QJsonObject{{"op", "set"}, {"clip", "c1"}, {"props", QJsonObject{{"path", "0,0 1,1"}}}}});
        QVERIFY(!window.m_lastCommand.isEmpty());
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
