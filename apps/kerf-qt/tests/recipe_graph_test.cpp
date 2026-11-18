// A shot's graph: its recipe's steps and what they read, drawn wired; a node's
// fields written back to the recipe, a signal drawn, the shot made and its
// takes chosen. Fed `recipe` events shaped as the bridge sends them
// (kerf_engine::recipe::graph).
//
// With KERF_SCREENSHOT_DIR set, the graph is saved as a person would see it:
// graph-bounce.png and one for each node chosen.

#include "fixtures.h"
#include "test_registry.h"

#include "command_registry.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "main_window.h"
#include "timeline_widget.h"
#include "ui/components.h"
#include "ui/modal_scrim.h"
#include "ui/recipe_graph.h"
#include "ui/recipe_graph_panel.h"
#include "ui/recipe_inspector.h"
#include "ui/signal_curve.h"
#include "ui/stage_shell.h"
#include "ui/wiring/graph_wiring.h"

#include <QAction>
#include <QCoreApplication>
#include <QDir>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QPlainTextEdit>
#include <QSettings>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

namespace kerf {

using namespace test;
using graph::Gives;
using graph::Kind;

namespace {

// The agreed example, as the recipe's file holds it: a basketball bouncing on
// a court. The court's sequence drawn as a depth guide; a picture of the ball
// from an image model; where the ball goes and when it meets the floor, from
// the project; all into a video model, whose file is the shot.
const char *kBounce = "kerf-recipe 1\n"
                      "# A basketball bouncing on a court.\n"
                      "render court {depth.mp4} pass=depth size=832x480 rate=16\n"
                      "# The ball, made once and handed on.\n"
                      "generate model=still prompt=\"an orange basketball, studio light\" seed=3 out={ball.png}\n"
                      "generate model=video prompt=\"a basketball bounces on a court\" guide={depth.mp4} ref={ball.png} moves=ball signal=bounce seconds=4 seed=7 strength=0.8\n";

QJsonArray bounceValues() {
    // 0 on the floor's frames, rising to 1 where the ball meets it, 96 frames.
    QJsonArray values;
    for (int frame = 0; frame < 96; ++frame) {
        const int beat = frame % 32;
        values.append(beat >= 12 && beat < 16 ? 1.0 : beat == 11 || beat == 16 ? 0.5 : 0.0);
    }
    return values;
}

// The `recipe` event of that shot, as the bridge says it. `takes` is how many
// takes there are, the last the one the shot is.
QJsonObject bounceRecipe(int takes = 2, const QString &text = QString::fromLatin1(kBounce)) {
    QJsonArray made;
    for (int number = 1; number <= takes; ++number)
        made.append(QJsonObject{{"number", number}, {"path", QStringLiteral("shots/bounce.takes/%1.mp4").arg(number, 3, 10, QLatin1Char('0'))},
                                {"chosen", number == takes}, {"rolled", number - 1}, {"kept", number > 1 || takes == 1},
                                {"changed", number == 1 ? QJsonArray{"the recipe"} : QJsonArray{}}});
    const QJsonArray nodes{
        QJsonObject{{"id", "sequence:court"}, {"kind", "sequence"}, {"title", "The court"}, {"detail", "1920×1080 · 24 fps · 4 s"},
                    {"gives", "video"}, {"sequence", "court"}, {"there", true}},
        QJsonObject{{"id", "made:depth.mp4"}, {"kind", "render"}, {"title", "Depth guide"}, {"detail", "832×480 · 16 fps"}, {"gives", "video"},
                    {"line", 3}, {"sequence", "court"}, {"makes", "depth.mp4"}, {"pass", "depth"}, {"size", "832x480"}, {"rate", "16"},
                    {"matte", QJsonValue()}},
        QJsonObject{{"id", "made:ball.png"}, {"kind", "model"}, {"title", "still"}, {"detail", "an orange basketball, studio light"},
                    {"gives", "image"}, {"line", 5}, {"model", "still"}, {"setUp", true}, {"prompt", "an orange basketball, studio light"},
                    {"seed", 3}, {"seconds", QJsonValue()}, {"size", QJsonValue()}, {"makes", "ball.png"}, {"knobs", QJsonArray{}}},
        QJsonObject{{"id", "moves:ball"}, {"kind", "moves"}, {"title", "ball"}, {"detail", "where it goes in court"}, {"gives", "path"},
                    {"clip", "ball"}, {"there", true}, {"sequence", "court"}, {"aspect", 16.0 / 9.0},
                    {"path", "-0.42,-0.3 -0.28,0.32 -0.12,-0.12 0.04,0.32 0.2,0.02 0.34,0.32 0.44,0.2"}},
        QJsonObject{{"id", "signal:bounce"}, {"kind", "signal"}, {"title", "bounce"}, {"detail", "court · 96 frames at 24"},
                    {"gives", "signal"}, {"clip", "bounce"}, {"sequence", "court"}, {"frames", 96}, {"fps", 24.0}, {"values", bounceValues()},
                    {"there", true}},
        QJsonObject{{"id", "made:out"}, {"kind", "model"}, {"title", "video"}, {"detail", "a basketball bounces on a court"},
                    {"gives", "video"}, {"line", 6}, {"model", "video"}, {"setUp", true}, {"prompt", "a basketball bounces on a court"},
                    {"seed", 7}, {"seconds", 4.0}, {"size", QJsonValue()}, {"makes", "out"},
                    {"about", "A shot that follows a depth guide, a reference picture, a signal and where things go."},
                    {"reads", QJsonArray{"guide", "ref", "signal", "moves"}}, {"modelSize", "832x480"}, {"modelRate", 16},
                    {"modelSeconds", 5.0625},
                    {"knobs", QJsonArray{QJsonObject{{"name", "strength"}, {"value", "0.8"}, {"default", 1.0}},
                                         QJsonObject{{"name", "steps"}, {"value", QJsonValue()}, {"default", 20}},
                                         QJsonObject{{"name", "cfg"}, {"value", QJsonValue()}, {"default", 6.0}}}}},
        QJsonObject{{"id", "takes"}, {"kind", "takes"}, {"title", "bounce"},
                    {"detail", takes > 0 ? QStringLiteral("take %1 of %1").arg(takes) : QStringLiteral("no take yet")}, {"gives", "video"},
                    {"media", "m4"}, {"takes", made},
                    {"wants", takes > 0 ? QJsonValue() : QJsonValue(QStringLiteral("its file is not there yet"))}}};
    const auto edge = [](const char *from, const char *to, const char *port, const char *gives, int nth = 0) {
        return QJsonObject{{"from", from}, {"to", to}, {"port", port}, {"nth", nth}, {"gives", gives}};
    };
    return QJsonObject{{"type", "recipe"}, {"media", "m4"}, {"recipe", "shots/bounce.kerfrecipe"}, {"name", "bounce"}, {"text", text},
                       {"nodes", nodes},
                       {"edges", QJsonArray{edge("sequence:court", "made:depth.mp4", "sequence", "video"),
                                            edge("made:depth.mp4", "made:out", "guide", "video"),
                                            edge("made:ball.png", "made:out", "ref", "image"),
                                            edge("moves:ball", "made:out", "moves", "path"),
                                            edge("signal:bounce", "made:out", "signal", "signal"),
                                            edge("made:out", "takes", "take", "video")}},
                       {"notes", QJsonArray{}}};
}

// A project whose media m4 is that shot, on the timeline as the clip `shot`,
// beside one that no recipe makes.
QJsonObject bounceState(int revision = 2, const QString &recipe = QStringLiteral("shots/bounce.kerfrecipe")) {
    const auto clip = [](const char *id, const char *media, double start, double duration) {
        return QJsonObject{{"id", id}, {"kind", "media"}, {"sourceId", media}, {"timelineStart", start}, {"duration", duration},
                           {"startFrame", std::llround(start * 24)}, {"durationFrames", std::llround(duration * 24)}, {"enabled", true}};
    };
    return QJsonObject{{"type", "state"},
                       {"snapshot", QJsonObject{{"revision", revision}, {"duration", 8},
                           {"timeline", QJsonObject{{"name", "Court"}, {"fps", 24}, {"width", 1920}, {"height", 1080}, {"duration", 8},
                               {"sources", QJsonArray{
                                   QJsonObject{{"id", "m1"}, {"path", "media/crowd.mov"}, {"width", 1920}, {"height", 1080}, {"duration", 4}, {"fps", 24}},
                                   QJsonObject{{"id", "m4"}, {"path", "shots/bounce.takes/002.mp4"}, {"width", 832}, {"height", 480}, {"duration", 4},
                                               {"fps", 16}, {"recipe", recipe},
                                               {"takes", QJsonArray{QJsonObject{{"number", 1}, {"path", "shots/bounce.takes/001.mp4"}, {"chosen", false}},
                                                                    QJsonObject{{"number", 2}, {"path", "shots/bounce.takes/002.mp4"}, {"chosen", true}}}}}}},
                               {"tracks", QJsonArray{QJsonObject{{"id", "V1"}, {"type", "video"}, {"enabled", true},
                                                                 {"clips", QJsonArray{clip("crowd", "m1", 0, 4), clip("shot", "m4", 4, 4)}}}}},
                               {"markers", QJsonArray{}}}}}}};
}

// The workflow the video model of that shot is run with, as the bridge says it
// (`workflow`, from kerf_workflow::detail::described): a small ComfyUI graph with
// links, blanks the recipe fills, and values of its own. `own` is the shot's
// own copy of it, a file of the project its line names.
QJsonObject shotWorkflow(bool own = false, const QString &sampler = QStringLiteral("uni_pc")) {
    const auto link = [](const char *name, const char *to) { return QJsonObject{{"name", name}, {"link", QJsonArray{to, 0}}}; };
    const auto blank = [](const char *name, const char *value, QJsonArray blanks) {
        return QJsonObject{{"name", name}, {"value", value}, {"blanks", blanks}};
    };
    const auto value = [](const char *name, const QJsonValue &held) { return QJsonObject{{"name", name}, {"value", held}}; };
    const QString text = QStringLiteral("{\"kerf-comfy\": 1, \"workflow\": {\"8\": {\"inputs\": {\"sampler_name\": \"%1\"}}}}").arg(sampler);
    return QJsonObject{
        {"type", "workflow"}, {"media", "m4"}, {"line", 6}, {"model", "video"}, {"name", "shot"},
        {"origin", own ? "own" : "bundled"}, {"file", own ? QJsonValue("shots/bounce.video.comfy.json") : QJsonValue()},
        {"about", "A shot that follows a guide video, a reference picture, both, or the prompt alone."},
        {"fps", 16.0}, {"frames", QJsonObject{{"most", 81}, {"step", 4}}}, {"size", QJsonArray{832, 480}}, {"text", text},
        {"nodes", QJsonArray{
            QJsonObject{{"id", "1"}, {"class", "UNETLoader"},
                        {"inputs", QJsonArray{blank("unet_name", "{unet}", {"unet"}), value("weight_dtype", "default")}}},
            QJsonObject{{"id", "4"}, {"class", "CLIPTextEncode"}, {"title", "Prompt"},
                        {"inputs", QJsonArray{link("clip", "1"), blank("text", "{prompt}", {"prompt"})}}},
            QJsonObject{{"id", "6"}, {"class", "LoadVideo"}, {"inputs", QJsonArray{blank("file", "{guide}", {"guide"})}}},
            QJsonObject{{"id", "8"}, {"class", "KSampler"},
                        {"inputs", QJsonArray{link("model", "1"), link("positive", "4"), link("guide", "6"), blank("seed", "{seed}", {"seed"}),
                                              blank("steps", "{steps}", {"steps"}), value("cfg", 6.0), value("sampler_name", sampler),
                                              value("denoise", 1), value("tiled", false)}}},
            QJsonObject{{"id", "9"}, {"class", "SaveVideo"}, {"inputs", QJsonArray{link("video", "8"), value("filename_prefix", "kerf")}}}}},
        {"blanks", QJsonObject{
            {"unet", QJsonObject{{"by", "set"}, {"says", "wan2.1_vace_1.3B_fp16.safetensors unless the line says unet="},
                                 {"default", "wan2.1_vace_1.3B_fp16.safetensors"}, {"given", QJsonValue()}}},
            {"prompt", QJsonObject{{"by", "line"}, {"says", "the line's prompt, with the cast and styles it names"},
                                   {"given", "a basketball bounces on a court"}}},
            {"guide", QJsonObject{{"by", "file"}, {"says", "the file the line gives as guide="}, {"given", "{depth.mp4}"}}},
            {"seed", QJsonObject{{"by", "line"}, {"says", "the line's seed, or one picked where it gives none"}, {"given", "7"}}},
            {"steps", QJsonObject{{"by", "set"}, {"says", "20 unless the line says steps="}, {"default", 20}, {"given", QJsonValue()}}}}}};
}

// The shot's recipe with its video model run by kerf-comfy, as the bridge marks it.
QJsonObject bounceWithRunner(const QString &workflow = QString()) {
    QJsonObject recipe = bounceRecipe();
    QJsonArray nodes = recipe.value("nodes").toArray();
    QJsonObject video = nodes.at(5).toObject();
    video.insert("runner", QJsonObject{{"kind", "comfy"}, {"workflow", "shot"}});
    video.insert("workflow", workflow.isEmpty() ? QJsonValue() : QJsonValue(workflow));
    nodes.replace(5, video);
    recipe.insert("nodes", nodes);
    return recipe;
}

// The middle of an edge, where it is clear of both cards.
QPointF middleOf(const QPainterPath &path) { return path.pointAtPercent(0.5); }

// Whether `pixel` is `ink` laid over `ground` at a third of its strength or
// more, and nothing else: a thin line drawn across a pixel's edge is the
// ink's colour thinned, not another colour.
bool isInkOn(QRgb pixel, const QColor &ground, const QColor &ink) {
    const double axis[3] = {double(ink.red() - ground.red()), double(ink.green() - ground.green()), double(ink.blue() - ground.blue())};
    const double seen[3] = {double(qRed(pixel) - ground.red()), double(qGreen(pixel) - ground.green()), double(qBlue(pixel) - ground.blue())};
    const double length = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
    const double share = (axis[0] * seen[0] + axis[1] * seen[1] + axis[2] * seen[2]) / length;
    double off = 0;
    for (int channel = 0; channel < 3; ++channel) off += std::abs(seen[channel] - share * axis[channel]);
    return share >= 0.33 && share <= 1.1 && off < 18;
}

} // namespace

class RecipeGraphTest final : public QObject {
    Q_OBJECT

private:
    // The editor on the bounce project, connected, with the graph of m4 open
    // and said by the bridge.
    static void openBounce(MainWindow &window, int takes = 2) {
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(bounceState());
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        window.showGraph(QStringLiteral("m4"));
        window.handleEvent(bounceRecipe(takes));
        window.m_errorBanner->hide();
        QCoreApplication::processEvents();
    }
    static RecipeGraphPanel *panelOf(MainWindow &window) { return window.m_graphWiring->panel(); }
    // The field named `name` of the node's card.
    static QLineEdit *field(MainWindow &window, const QString &name) {
        return panelOf(window)->inspector()->findChild<QLineEdit *>(QStringLiteral("graphField.%1").arg(name));
    }
    // Type `text` into `edit` in place of what it holds, and leave it.
    static void typeInto(QLineEdit *edit, const QString &text) {
        edit->setFocus();
        edit->selectAll();
        QTest::keyClicks(edit, text);
        if (text.isEmpty()) QTest::keyClick(edit, Qt::Key_Backspace);
        QTest::keyClick(edit, Qt::Key_Return);
    }
    static bool save(QWidget &window, const QString &name) {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        for (int i = 0; i < 3; ++i) QCoreApplication::processEvents();
        return shots.isEmpty() || window.grab().toImage().save(QDir(shots).filePath(name + QStringLiteral(".png")));
    }

private slots:
    void init() { QSettings().remove(QStringLiteral("graph")); }
    void cleanup() { QSettings().remove(QStringLiteral("graph")); }

    // What the bridge says is read as nodes and edges, and laid out by how far
    // each is from what it reads: what is read beside what reads it, the takes last.
    void aRecipeEventReadsAsAGraphLaidOutInColumns() {
        const graph::Recipe recipe = graph::Recipe::read(bounceRecipe());
        QVERIFY(recipe.isValid());
        QCOMPARE(recipe.name, QStringLiteral("bounce"));
        QCOMPARE(recipe.text, QString::fromLatin1(kBounce));
        QCOMPARE(recipe.nodes.size(), 7);
        QCOMPARE(recipe.edges.size(), 6);
        const graph::Node *video = recipe.node(QStringLiteral("made:out"));
        QVERIFY(video && video->kind == Kind::Model && video->line == 6 && video->gives == Gives::Video);
        QCOMPARE(recipe.node(QStringLiteral("signal:bounce"))->gives, Gives::Signal);
        QCOMPARE(recipe.node(QStringLiteral("moves:ball"))->kind, Kind::Moves);
        QCOMPARE(recipe.into(QStringLiteral("made:out")).size(), 4);
        // An edge to what is not there is not kept.
        QJsonObject broken = bounceRecipe();
        QJsonArray edges = broken.value("edges").toArray();
        edges.append(QJsonObject{{"from", "made:nothing"}, {"to", "made:out"}, {"port", "ref"}, {"nth", 1}, {"gives", "image"}});
        broken.insert("edges", edges);
        QCOMPARE(graph::Recipe::read(broken).edges.size(), 6);

        const QHash<QString, QPointF> places = graph::arranged(recipe);
        const auto x = [&](const char *id) { return places.value(QString::fromLatin1(id)).x(); };
        const auto y = [&](const char *id) { return places.value(QString::fromLatin1(id)).y(); };
        QVERIFY(x("sequence:court") < x("made:depth.mp4"));
        // Everything the video model reads stands in the column before it, in the order it reads them.
        for (const char *read : {"made:depth.mp4", "made:ball.png", "moves:ball", "signal:bounce"}) QCOMPARE(x(read), x("made:depth.mp4"));
        QVERIFY(y("made:depth.mp4") < y("made:ball.png") && y("made:ball.png") < y("moves:ball") && y("moves:ball") < y("signal:bounce"));
        QVERIFY(x("made:depth.mp4") < x("made:out") && x("made:out") < x("takes"));
        // What each type is called and coloured: the faded palette's own, and no other.
        QCOMPARE(graph::colorOf(Gives::Video), tokens::color(tokens::accentInk));
        QCOMPARE(graph::colorOf(Gives::Image), tokens::color(tokens::pos));
        QCOMPARE(graph::colorOf(Gives::Path), tokens::color(tokens::playhead));
        QCOMPARE(graph::colorOf(Gives::Signal), tokens::color(tokens::status::request));
        QCOMPARE(graph::nameOf(Gives::Path), QStringLiteral("path"));
        // A card that draws a curve is taller than one that does not.
        QVERIFY(graph::cardSize(*recipe.node(QStringLiteral("signal:bounce"))).height() >
                graph::cardSize(*recipe.node(QStringLiteral("sequence:court"))).height());
    }

    // The canvas: every card whole and apart from the others, each edge in the
    // colour of what goes along it, and a legend of the four.
    void theCanvasDrawsCardsEdgesAndALegend() {
        RecipeGraphView view;
        view.resize(1100, 640);
        view.setRecipe(graph::Recipe::read(bounceRecipe()));
        view.show();
        QVERIFY(QTest::qWaitForWindowExposed(&view));
        const graph::Recipe &recipe = view.recipe();
        QList<QRectF> cards;
        for (const graph::Node &node : recipe.nodes) {
            const QRectF card = view.cardRect(node.id);
            QVERIFY2(QRectF(view.rect()).contains(card), qPrintable(node.id));
            for (const QRectF &other : std::as_const(cards)) QVERIFY2(!card.intersects(other), qPrintable(node.id));
            cards << card;
            QCOMPARE(view.nodeAt(card.center()), node.id);
        }
        QCOMPARE(view.nodeAt(QPointF(4, 4)), QString());
        const QImage shown = view.grab().toImage();
        // Each edge is drawn in its type's colour, where it runs between the cards.
        for (int index = 0; index < recipe.edges.size(); ++index) {
            const QPointF middle = middleOf(view.edgePath(index));
            const QColor want = graph::colorOf(recipe.edges.at(index).gives);
            bool found = false;
            for (int dy = -2; dy <= 2 && !found; ++dy)
                for (int dx = -2; dx <= 2 && !found; ++dx)
                    found = isInkOn(shown.pixel(middle.toPoint() + QPoint(dx, dy)), tokens::color(tokens::nodes::canvas), want);
            QVERIFY2(found, qPrintable(QStringLiteral("edge %1 to %2").arg(recipe.edges.at(index).from, recipe.edges.at(index).to)));
        }
        // The canvas is the dark well, and the cards a step up from it.
        QVERIFY(nearColor(shown.pixel(3, 3), tokens::color(tokens::nodes::canvas).rgb(), 6));
        QVERIFY(nearColor(shown.pixel(view.cardRect(QStringLiteral("takes")).center().toPoint() + QPoint(60, 18)),
                          tokens::color(tokens::nodes::node).rgb(), 4));
        // The legend names the four a shot is told by, at the canvas's foot.
        QCOMPARE(view.legend(), (QList<Gives>{Gives::Video, Gives::Image, Gives::Path, Gives::Signal}));
        const QRectF legend = view.legendRect();
        QVERIFY(QRectF(view.rect()).contains(legend) && legend.bottom() > view.height() - 24);
        TextRecorder texts;
        view.render(&texts);
        for (const char *said : {"video", "image", "path", "signal", "Depth guide", "The court", "bounce", "guide", "ref", "moves"})
            QVERIFY2(texts.colorOf(QString::fromLatin1(said)).isValid(), said);
        // Words and other files join the legend only where the graph has them.
        QJsonObject withCast = bounceRecipe();
        QJsonArray nodes = withCast.value("nodes").toArray();
        nodes.append(QJsonObject{{"id", "cast:Ada"}, {"kind", "cast"}, {"title", "Ada"}, {"detail", "a keeper"}, {"gives", "words"}, {"there", true}});
        QJsonArray edges = withCast.value("edges").toArray();
        edges.append(QJsonObject{{"from", "cast:Ada"}, {"to", "made:out"}, {"port", "cast"}, {"nth", 0}, {"gives", "words"}});
        withCast.insert("nodes", nodes);
        withCast.insert("edges", edges);
        view.setRecipe(graph::Recipe::read(withCast));
        QCOMPARE(view.legend().size(), 5);
        // A recipe that does not read has no cards, and says why where they would be.
        view.setRecipe(graph::Recipe::read(QJsonObject{{"type", "recipe"}, {"media", "m4"}, {"recipe", "shots/bounce.kerfrecipe"},
                                                      {"name", "bounce"}, {"text", "kerf-recipe 1\nrender\n"},
                                                      {"error", "line 2 of the recipe: `render` is …"}, {"nodes", QJsonArray{}},
                                                      {"edges", QJsonArray{}}}));
        TextRecorder refused;
        view.render(&refused);
        QCOMPARE(refused.colorOf(QStringLiteral("does not read")), tokens::color(tokens::status::request));
    }

    // The canvas is free-form: a card stays where it is dragged, for that shot,
    // and a click chooses it; the canvas itself is dragged about and zoomed.
    void cardsStayWhereTheyAreDragged() {
        const graph::Recipe recipe = graph::Recipe::read(bounceRecipe());
        QPointF moved;
        {
            RecipeGraphView view;
            view.resize(1100, 640);
            view.setPlacementKey(QStringLiteral("/p\nshots/bounce"));
            view.setRecipe(recipe);
            QSignalSpy chosen(&view, &RecipeGraphView::selectionChanged);
            const QRectF before = view.cardRect(QStringLiteral("signal:bounce"));
            mouse(&view, QEvent::MouseButtonPress, before.center());
            QCOMPARE(view.selected(), QStringLiteral("signal:bounce"));
            QCOMPARE(chosen.size(), 1);
            mouse(&view, QEvent::MouseMove, before.center() + QPointF(40, 60));
            mouse(&view, QEvent::MouseButtonRelease, before.center() + QPointF(40, 60));
            moved = view.cardRect(QStringLiteral("signal:bounce")).topLeft();
            QVERIFY(std::abs(moved.x() - before.left() - 40) <= 1.0 && std::abs(moved.y() - before.top() - 60) <= 1.0);
            // The others are where they were, and its edge follows it.
            QCOMPARE(view.cardRect(QStringLiteral("made:out")), view.cardRect(QStringLiteral("made:out")));
            QVERIFY(view.edgePath(4).elementAt(0).y > before.center().y() + 30);
            // A click on the canvas lets go of it; a drag there moves the canvas.
            const QPointF empty(20, 20);
            mouse(&view, QEvent::MouseButtonPress, empty);
            mouse(&view, QEvent::MouseButtonRelease, empty);
            QCOMPARE(view.selected(), QString());
            const QPointF takes = view.cardRect(QStringLiteral("takes")).topLeft();
            mouse(&view, QEvent::MouseButtonPress, empty);
            mouse(&view, QEvent::MouseMove, empty + QPointF(30, 12));
            mouse(&view, QEvent::MouseButtonRelease, empty + QPointF(30, 12));
            QCOMPARE(view.cardRect(QStringLiteral("takes")).topLeft(), takes + QPointF(30, 12));
            // Zooming keeps what is under the pointer under it.
            const QPointF about = view.cardRect(QStringLiteral("takes")).center();
            view.zoomBy(1.25, about);
            QVERIFY(std::abs(view.cardRect(QStringLiteral("takes")).center().x() - about.x()) < 0.5);
            // A double-click on the canvas takes the whole graph in again.
            view.fit();
            QVERIFY(view.zoom() <= 1.0);
        }
        // Opened again for the same shot, the card is where it was left; another shot's is not.
        RecipeGraphView again;
        again.resize(1100, 640);
        again.setPlacementKey(QStringLiteral("/p\nshots/bounce"));
        again.setRecipe(recipe);
        QVERIFY(std::abs(again.cardRect(QStringLiteral("signal:bounce")).left() - moved.x()) <= 1.0);
        RecipeGraphView other;
        other.resize(1100, 640);
        other.setPlacementKey(QStringLiteral("/p\nshots/other"));
        other.setRecipe(recipe);
        QVERIFY(std::abs(other.cardRect(QStringLiteral("signal:bounce")).top() - moved.y()) > 20);
    }

    // Nodes are wired by hand: an edge drawn from a node's port to a step that
    // could read it is one word on that step's line, asked for by name where
    // more than one name would read it.
    void anEdgeDrawnToAStepIsAWordOnItsLine() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        RecipeGraphView *view = panel->view();
        const graph::Recipe &recipe = view->recipe();
        const auto node = [&](const char *id) { return *recipe.node(QString::fromLatin1(id)); };
        // What could read what: a signal as a signal, a still as a frame or a reference,
        // a picture in motion as a guide; and never a step that comes after.
        QCOMPARE(graph::portsFor(recipe, node("signal:bounce"), node("made:ball.png")), QStringList{"signal"});
        QCOMPARE(graph::portsFor(recipe, node("made:ball.png"), node("made:out")), (QStringList{"first", "last", "ref"}));
        QCOMPARE(graph::portsFor(recipe, node("made:depth.mp4"), node("made:out")), (QStringList{"guide", "ref", "first"}));
        QCOMPARE(graph::portsFor(recipe, node("sequence:court"), node("made:depth.mp4")), QStringList{"sequence"});
        QVERIFY(graph::portsFor(recipe, node("made:out"), node("made:ball.png")).isEmpty());
        QVERIFY(graph::portsFor(recipe, node("sequence:court"), node("made:out")).isEmpty());
        QVERIFY(graph::portsFor(recipe, node("signal:bounce"), node("takes")).isEmpty());
        QCOMPARE(graph::valueOf(node("made:ball.png")), QStringLiteral("{ball.png}"));
        QCOMPARE(graph::valueOf(node("signal:bounce")), QStringLiteral("bounce"));

        // The signal, drawn to the still model: one name fits, and it is written.
        const QPointF port = view->portPoint(QStringLiteral("signal:bounce"));
        QCOMPARE(view->portAt(port), QStringLiteral("signal:bounce"));
        QCOMPARE(view->portAt(port + QPointF(40, 0)), QString());
        const QPointF still = view->cardRect(QStringLiteral("made:ball.png")).center();
        mouse(view, QEvent::MouseButtonPress, port);
        mouse(view, QEvent::MouseMove, (port + still) / 2);
        mouse(view, QEvent::MouseMove, still);
        mouse(view, QEvent::MouseButtonRelease, still);
        const QString was = QString::fromLatin1(kBounce);
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", was}, {"line", 5},
                                                    {"key", "signal"}, {"nth", 0}, {"value", "bounce"}}));
        // It chose no node and moved no card.
        QCOMPARE(view->selected(), QString());
        // Let go over the canvas, or over what cannot read it, nothing is written.
        window.m_lastCommand = {};
        mouse(view, QEvent::MouseButtonPress, port);
        mouse(view, QEvent::MouseMove, QPointF(30, 30));
        mouse(view, QEvent::MouseButtonRelease, QPointF(30, 30));
        mouse(view, QEvent::MouseButtonPress, view->portPoint(QStringLiteral("made:out")));
        mouse(view, QEvent::MouseMove, still);
        mouse(view, QEvent::MouseButtonRelease, still);
        QCOMPARE(window.m_lastCommand, QJsonObject{});

        // The still, drawn to the video model: three names would read it, and one is chosen.
        const QPointF video = view->cardRect(QStringLiteral("made:out")).center();
        mouse(view, QEvent::MouseButtonPress, view->portPoint(QStringLiteral("made:ball.png")));
        mouse(view, QEvent::MouseMove, video);
        mouse(view, QEvent::MouseButtonRelease, video);
        QCOMPARE(window.m_lastCommand, QJsonObject{});
        auto *menu = panel->findChild<QMenu *>(QStringLiteral("graphPortMenu"));
        QVERIFY(menu);
        QStringList names;
        for (const QAction *action : menu->actions())
            if (action->isEnabled()) names << action->text();
        QCOMPARE(names, (QStringList{"first", "last", "ref"}));
        // The model has said it reads no first frame, which the row says.
        QCOMPARE(ui::menuKey(menu->findChild<QAction *>(QStringLiteral("graphPort.first"))), QStringLiteral("the model has no use for it"));
        QCOMPARE(ui::menuKey(menu->findChild<QAction *>(QStringLiteral("graphPort.ref"))), QStringLiteral("who or what is in it, or how it looks"));
        // A reference is one more of them: the line has one already.
        menu->findChild<QAction *>(QStringLiteral("graphPort.ref"))->trigger();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", was}, {"line", 6},
                                                    {"key", "ref"}, {"nth", 1}, {"value", "{ball.png}"}}));
        menu->close();
    }

    // A shot made from a recipe opens as its graph: from its menu in the bin,
    // and by double-clicking its clip. Esc gives the window back.
    void aShotOpensAsItsGraphFromItsMenuAndItsClip() {
        MainWindow window{QString(), QString()};
        window.setConnection(QStringLiteral("ready"));
        window.handleEvent(bounceState());
        window.resize(1600, 1000);
        window.show();
        QVERIFY(QTest::qWaitForWindowExposed(&window));
        GraphWiring *wiring = window.m_graphWiring.get();
        QVERIFY(!wiring->isOpen());

        // Only a media a recipe makes has a graph.
        QMenu *menu = window.binMenu(QStringLiteral("m1"));
        QVERIFY(!menu->findChild<QAction *>(QStringLiteral("binGraph")));
        delete menu;
        menu = window.binMenu(QStringLiteral("m4"));
        QAction *open = menu->findChild<QAction *>(QStringLiteral("binGraph"));
        QVERIFY(open);
        QCOMPARE(open->text(), QStringLiteral("Open Graph"));
        open->trigger();
        delete menu;
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_get"}, {"media", "m4"}}));
        QVERIFY(wiring->isOpen());
        QCOMPARE(wiring->media(), QStringLiteral("m4"));
        // Over the window, as everything one goes into: the window dimmed, the panel in it.
        QVERIFY(window.m_shell->covered());
        QCOMPARE(window.m_shell->cover(), wiring->cover());
        RecipeGraphPanel *panel = wiring->panel();
        QVERIFY(panel->isVisible());
        // Until the bridge has said it, the shot's name over an empty canvas.
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphTitle"))->text(), QStringLiteral("bounce"));
        QVERIFY(panel->view()->recipe().nodes.isEmpty());
        window.handleEvent(bounceRecipe());
        QCOMPARE(panel->view()->recipe().nodes.size(), 7);
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphPath"))->text(), QStringLiteral("shots/bounce.kerfrecipe"));
        // Another shot's recipe, said while this one is open, is not this graph.
        QJsonObject other = bounceRecipe();
        other.insert("media", "m9");
        other.insert("nodes", QJsonArray{});
        window.handleEvent(other);
        QCOMPARE(panel->view()->recipe().nodes.size(), 7);

        // Esc, the panel's close, or a press on the dim gives the window back.
        QVERIFY(window.m_shell->dismiss());
        QVERIFY(!wiring->isOpen() && !window.m_shell->covered());
        // Its clip, double-clicked, opens it; a clip of footage opens nothing.
        emit window.m_timeline->clipActivated(QStringLiteral("crowd"));
        QVERIFY(!wiring->isOpen());
        window.m_lastCommand = {};
        emit window.m_timeline->clipActivated(QStringLiteral("shot"));
        QVERIFY(wiring->isOpen());
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_get"}, {"media", "m4"}}));
        panel->findChild<ui::IconButton *>(QStringLiteral("graphClose"))->click();
        QVERIFY(!wiring->isOpen());
        // Settings takes the window from an open graph, and the graph from Settings.
        window.showGraph(QStringLiteral("m4"));
        window.showSettings(QStringLiteral("models"));
        QVERIFY(window.m_shell->covered() && !wiring->isOpen());
        QCOMPARE(window.m_shell->cover(), window.m_preferences);
        window.showGraph(QStringLiteral("m4"));
        QVERIFY(wiring->isOpen() && !window.m_preferences->isVisible());
        // The project changing under an open graph has it read again: a signal's
        // keys, a clip's way, a take chosen, are all the project's.
        window.m_lastCommand = {};
        window.handleEvent(bounceState(3));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_get"}, {"media", "m4"}}));
        window.m_shell->dismiss();
        window.m_lastCommand = {};
        window.handleEvent(bounceState(4));
        QCOMPARE(window.m_lastCommand, QJsonObject{});
    }

    // An agent's edit is the project's: once it is accepted the media is made
    // from the recipe's next version, and the graph that is open shows that
    // one, with nothing the person had arranged or chosen lost.
    void anAgentsChangeShowsInTheGraphThatIsOpen() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        window.handleEvent(bounceWithRunner());
        RecipeGraphPanel *panel = panelOf(window);
        RecipeGraphView *view = panel->view();
        const QRectF before = view->cardRect(QStringLiteral("signal:bounce"));
        mouse(view, QEvent::MouseButtonPress, before.center());
        mouse(view, QEvent::MouseMove, before.center() + QPointF(40, 60));
        mouse(view, QEvent::MouseButtonRelease, before.center() + QPointF(40, 60));
        const QPointF moved = view->cardRect(QStringLiteral("signal:bounce")).topLeft();
        QVERIFY(moved != before.topLeft());
        view->select(QStringLiteral("made:out"));
        QCOMPARE(field(window, QStringLiteral("seed"))->text(), QStringLiteral("7"));
        const int edges = view->recipe().edges.size();

        // The agent set the seed, let the signal go and set a value of the workflow:
        // one revision, in which the media names the recipe's next version.
        window.m_lastCommand = {};
        window.handleEvent(bounceState(3, QStringLiteral("shots/bounce@2.kerfrecipe")));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_get"}, {"media", "m4"}}));
        QJsonObject changed = bounceWithRunner(QStringLiteral("bounce.video.comfy.json"));
        changed.insert("recipe", "shots/bounce@2.kerfrecipe");
        changed.insert("text", QString::fromLatin1(kBounce).replace(QStringLiteral("seed=7"), QStringLiteral("seed=8")));
        QJsonArray nodes = changed.value("nodes").toArray();
        QJsonObject video = nodes.at(5).toObject();
        video.insert("seed", 8);
        nodes.replace(5, video);
        changed.insert("nodes", nodes);
        QJsonArray wires;
        for (const QJsonValue &edge : changed.value("edges").toArray())
            if (edge.toObject().value("from").toString() != QStringLiteral("signal:bounce")) wires.append(edge);
        changed.insert("edges", wires);
        window.handleEvent(changed);

        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphPath"))->text(), QStringLiteral("shots/bounce@2.kerfrecipe"));
        QCOMPARE(view->recipe().edges.size(), edges - 1);
        // The node chosen is still chosen, and says what the agent set.
        QCOMPARE(view->selected(), QStringLiteral("made:out"));
        QCOMPARE(field(window, QStringLiteral("seed"))->text(), QStringLiteral("8"));
        QCOMPARE(panel->inspector()->findChild<QLabel *>(QStringLiteral("graphWorkflowSays"))->text().contains(QStringLiteral("bounce.video.comfy.json")), true);
        // A version of the recipe is the same shot: its cards are where they were put.
        QCOMPARE(view->cardRect(QStringLiteral("signal:bounce")).topLeft(), moved);
        // The person's next change is to the version the agent left.
        typeInto(field(window, QStringLiteral("seed")), QStringLiteral("9"));
        QCOMPARE(window.m_lastCommand.value("was").toString(), changed.value("text").toString());

        // Inside the model's workflow, that is read again too.
        panel->inspector()->findChild<ui::Button *>(QStringLiteral("graphOpenWorkflow"))->click();
        window.handleEvent(shotWorkflow(true));
        QVERIFY(panel->showsWorkflow());
        window.m_lastCommand = {};
        window.handleEvent(bounceState(4, QStringLiteral("shots/bounce@3.kerfrecipe")));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "workflow_get"}, {"media", "m4"}, {"line", 6}}));
        window.handleEvent(shotWorkflow(true, QStringLiteral("euler")));
        QCOMPARE(panel->workflow().text, shotWorkflow(true, QStringLiteral("euler")).value("text").toString());
    }

    // A node's prompt, its seed and a knob are set in its card, and each
    // change is sent as one word of one line of the recipe as it was read.
    void aNodesFieldsAreWrittenBackToTheRecipe() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        RecipeInspector *inspector = panel->inspector();
        // With no node chosen the card says the shot.
        QCOMPARE(inspector->nodeId(), QString());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphInspectorTitle"))->text(), QStringLiteral("bounce"));

        mouse(panel->view(), QEvent::MouseButtonPress, panel->view()->cardRect(QStringLiteral("made:out")).center());
        mouse(panel->view(), QEvent::MouseButtonRelease, panel->view()->cardRect(QStringLiteral("made:out")).center());
        QCOMPARE(inspector->nodeId(), QStringLiteral("made:out"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphInspectorTitle"))->text(), QStringLiteral("video"));
        const QString was = QString::fromLatin1(kBounce);
        const auto sent = [&](const QString &key, const QJsonValue &value) {
            QJsonObject edit{{"type", "recipe_edit"}, {"media", "m4"}, {"was", was}, {"line", 6}, {"key", key}, {"nth", 0}};
            if (!value.isUndefined()) edit.insert("value", value);
            return edit;
        };
        // The prompt: one line, however long; Return leaves it.
        auto *prompt = inspector->findChild<QPlainTextEdit *>(QStringLiteral("graphField.prompt"));
        QVERIFY(prompt);
        QCOMPARE(prompt->toPlainText(), QStringLiteral("a basketball bounces on a court"));
        prompt->setFocus();
        prompt->selectAll();
        QTest::keyClicks(prompt, QStringLiteral("a basketball bounces twice, then rolls"));
        QTest::keyClick(prompt, Qt::Key_Return);
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("prompt"), QStringLiteral("a basketball bounces twice, then rolls")));
        // Left as it is, nothing is sent.
        window.m_lastCommand = {};
        QTest::keyClick(prompt, Qt::Key_Return);
        QCOMPARE(window.m_lastCommand, QJsonObject{});

        // The seed, typed; emptied, it is taken off the line; and another at a press.
        QLineEdit *seed = field(window, QStringLiteral("seed"));
        QVERIFY(seed);
        QCOMPARE(seed->text(), QStringLiteral("7"));
        typeInto(seed, QStringLiteral("11"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("seed"), QStringLiteral("11")));
        typeInto(seed, QString());
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("seed"), QJsonValue(QJsonValue::Undefined)));
        inspector->findChild<ui::IconButton *>(QStringLiteral("graphSeedRoll"))->click();
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("seed"));
        bool whole = false;
        QVERIFY(window.m_lastCommand.value("value").toString().toULongLong(&whole) < (1ull << 31) && whole);
        QCOMPARE(seed->text(), window.m_lastCommand.value("value").toString());

        // A knob the line sets, and one the model has said it takes and the line leaves.
        QLineEdit *strength = field(window, QStringLiteral("knob.strength"));
        QLineEdit *steps = field(window, QStringLiteral("knob.steps"));
        QVERIFY(strength && steps);
        QCOMPARE(strength->text(), QStringLiteral("0.8"));
        QCOMPARE(strength->placeholderText(), QStringLiteral("1"));
        QCOMPARE(steps->text(), QString());
        QCOMPARE(steps->placeholderText(), QStringLiteral("20"));
        typeInto(strength, QStringLiteral("0.6"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("strength"), QStringLiteral("0.6")));
        typeInto(steps, QStringLiteral("30"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("steps"), QStringLiteral("30")));
        // Any other name goes to the model as it is written.
        QLineEdit *add = field(window, QStringLiteral("add"));
        typeInto(add, QStringLiteral("lora=ball.safetensors"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("lora"), QStringLiteral("ball.safetensors")));
        QCOMPARE(add->text(), QString());
        // What it reads is let go by taking its word off the line.
        inspector->findChild<ui::IconButton *>(QStringLiteral("graphLetGo.ref.0"))->click();
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("ref"), QJsonValue(QJsonValue::Undefined)));

        // The bridge answers with the graph as the file now has it; the card
        // takes its values, and the next change is to that text.
        const QString now = was.left(was.lastIndexOf(QStringLiteral("seed=7"))) + QStringLiteral("seed=11 strength=0.8\n");
        QJsonObject answered = bounceRecipe(2, now);
        QJsonArray nodes = answered.value("nodes").toArray();
        QJsonObject video = nodes.at(5).toObject();
        video.insert("seed", 11);
        nodes.replace(5, video);
        answered.insert("nodes", nodes);
        answered.insert("edited", true);
        window.handleEvent(answered);
        QCOMPARE(field(window, QStringLiteral("seed")), seed);
        QCOMPARE(seed->text(), QStringLiteral("11"));
        typeInto(strength, QStringLiteral("0.5"));
        QCOMPARE(window.m_lastCommand.value("was").toString(), now);
        // A field being typed in is the person's until they leave it.
        seed->setFocus();
        seed->selectAll();
        QTest::keyClicks(seed, QStringLiteral("42"));
        window.handleEvent(answered);
        QCOMPARE(seed->text(), QStringLiteral("42"));

        // Still being typed as another node is chosen, it sets its own node's
        // line as it gives way, and nothing of the node that takes its place.
        window.m_lastCommand = {};
        panel->view()->select(QStringLiteral("made:ball.png"));
        QCOMPARE(window.m_lastCommand.value("line").toInt(), 6);
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("seed"));
        QCOMPARE(window.m_lastCommand.value("value").toString(), QStringLiteral("42"));
        QCOMPARE(field(window, QStringLiteral("seed"))->text(), QStringLiteral("3"));

        // The guide's node: what of the sequence it draws, and at what size and rate.
        panel->view()->select(QStringLiteral("made:depth.mp4"));
        QCOMPARE(field(window, QStringLiteral("sequence"))->text(), QStringLiteral("court"));
        QCOMPARE(field(window, QStringLiteral("size"))->text(), QStringLiteral("832x480"));
        typeInto(field(window, QStringLiteral("rate")), QStringLiteral("24"));
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", now}, {"line", 3},
                                                    {"key", "rate"}, {"nth", 0}, {"value", "24"}}));
        QCOMPARE(inspector->findChild<ui::GhostButton *>(QStringLiteral("graphPass"))->text(), QStringLiteral("Depth"));
    }

    // A signal's curve is drawn in its node's card: a drag across it is the
    // clip's value in the project, keyed, as the timeline's own drawing is.
    void aSignalsCurveIsDrawnInItsCard() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        panel->view()->select(QStringLiteral("signal:bounce"));
        auto *curve = panel->inspector()->findChild<SignalCurve *>(QStringLiteral("signalCurve"));
        QVERIFY(curve && curve->isVisible());
        QCOMPARE(curve->frames(), 96);
        QCOMPARE(curve->values().size(), 96);
        const QRectF plot = curve->plot();
        // From the floor at its first frame up to the top at its last.
        mouse(curve, QEvent::MouseButtonPress, plot.bottomLeft());
        mouse(curve, QEvent::MouseMove, QPointF(plot.center().x(), plot.center().y()));
        mouse(curve, QEvent::MouseMove, plot.topRight());
        mouse(curve, QEvent::MouseButtonRelease, plot.topRight());
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("edit"));
        const QJsonObject op = window.m_lastCommand.value("ops").toArray().at(0).toObject();
        QCOMPARE(op.value("op").toString(), QStringLiteral("set"));
        QCOMPARE(op.value("clip").toString(), QStringLiteral("bounce"));
        const QJsonArray keys = op.value("props").toObject().value("value").toObject().value("keys").toArray();
        // A straight line is its two ends.
        QCOMPARE(keys.size(), 2);
        QCOMPARE(keys.first().toObject(), (QJsonObject{{"at", 0}, {"value", 0.0}, {"interp", "linear"}}));
        QCOMPARE(keys.last().toObject(), (QJsonObject{{"at", 95}, {"value", 1.0}, {"interp", "linear"}}));
        // A click draws nothing.
        window.m_lastCommand = {};
        mouse(curve, QEvent::MouseButtonPress, plot.center());
        mouse(curve, QEvent::MouseButtonRelease, plot.center());
        QCOMPARE(window.m_lastCommand, QJsonObject{});
        // The project says the curve again once it has it, and the card shows it.
        QJsonObject answered = bounceRecipe();
        QJsonArray nodes = answered.value("nodes").toArray();
        QJsonObject signal = nodes.at(4).toObject();
        signal.insert("values", QJsonArray{0.0, 1.0});
        nodes.replace(4, signal);
        answered.insert("nodes", nodes);
        window.handleEvent(answered);
        QCOMPARE(panel->inspector()->findChild<SignalCurve *>(QStringLiteral("signalCurve")), curve);
        QCOMPARE(curve->values(), (QJsonArray{0.0, 1.0}));
    }

    // The shot is made from the graph as from the bin: Make, or Make again once
    // it has a take, shows what would run first; Several rolls the seeds; a
    // making under way is stopped; and a take is chosen in the takes' card.
    void theShotIsMadeAndItsTakesChosenFromTheGraph() {
        MainWindow window{QString(), QString()};
        openBounce(window, 0);
        RecipeGraphPanel *panel = panelOf(window);
        QCOMPARE(panel->makeButton()->text(), QStringLiteral("Make"));
        QCOMPARE(panel->statusLabel()->text(), QStringLiteral("Wants making: its file is not there yet"));
        window.handleEvent(bounceRecipe(2));
        QCOMPARE(panel->makeButton()->text(), QStringLiteral("Make again"));
        QVERIFY(panel->makeButton()->isVisible() && panel->makeButton()->isEnabled() && !panel->stopButton()->isVisible());
        panel->makeButton()->click();
        // Nothing runs yet: the bridge is asked what would, and a person says yes to that.
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_make"}, {"media", "m4"}, {"confirmed", false}}));
        emit panel->makeRequested(3);
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_make"}, {"media", "m4"}, {"confirmed", false}, {"rolls", 3}}));

        // While it is made the head says what its steps say, and stops it.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "m4"}, {"take", 3}, {"line", "running, 12 s"}});
        QVERIFY(panel->isMaking() && panel->view()->isMaking());
        QCOMPARE(panel->statusLabel()->text(), QStringLiteral("Making take 3: running, 12 s"));
        QVERIFY(!panel->makeButton()->isVisible() && panel->stopButton()->isVisible());
        TextRecorder card;
        panel->view()->render(&card);
        QVERIFY(card.colorOf(QStringLiteral("Making take 3")).isValid());
        panel->stopButton()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_make_cancel"}, {"media", "m4"}}));
        // Made, its takes are read again.
        window.m_lastCommand = {};
        window.handleEvent(QJsonObject{{"type", "media_made"}, {"media", "m4"}, {"take", 3}});
        QVERIFY(!panel->isMaking() && panel->makeButton()->isVisible());
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_get"}, {"media", "m4"}}));
        window.handleEvent(bounceRecipe(3));

        // The takes' card: each take, the one the shot is ticked; another is chosen at a press.
        panel->view()->select(QStringLiteral("takes"));
        RecipeInspector *inspector = panel->inspector();
        auto *first = inspector->findChild<QAbstractButton *>(QStringLiteral("graphTake1"));
        auto *third = inspector->findChild<QAbstractButton *>(QStringLiteral("graphTake3"));
        QVERIFY(first && third);
        QVERIFY(third->isChecked() && !third->isEnabled());
        QVERIFY(!first->isChecked() && first->isEnabled());
        first->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_replace"}, {"media", "m4"}, {"path", "shots/bounce.takes/001.mp4"}}));
        // While one is being made none is chosen.
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "m4"}, {"take", 4}, {"line", ""}});
        QVERIFY(!first->isEnabled());
        window.handleEvent(QJsonObject{{"type", "media_make_cancelled"}, {"media", "m4"}});
        QVERIFY(first->isEnabled());
        // What cannot be made says so in the head, and why in the shot's card.
        QJsonObject cannot = bounceRecipe(3);
        cannot.insert("cannot", "no model `video` is set up: /home/you/.kerf/models.json names none");
        cannot.insert("notes", QJsonArray{"the guide `{depth.mp4}` is not drawn as model `video` reads it"});
        window.handleEvent(cannot);
        QCOMPARE(panel->statusLabel()->text(), QStringLiteral("Cannot be made as things stand"));
        panel->view()->select(QString());
        QVERIFY(inspector->findChild<QLabel *>(QStringLiteral("graphCannot"))->text().contains(QStringLiteral("no model `video` is set up")));
    }

    // The take the shot is, made from a recipe that has changed since, has its
    // recipe to go back to where the take kept it: the recipe written as that
    // making read it, seeds and workflow with it.
    void aTakesRecipeIsRestoredFromItsCard() {
        MainWindow window{QString(), QString()};
        openBounce(window, 3);
        RecipeGraphPanel *panel = panelOf(window);
        RecipeInspector *inspector = panel->inspector();
        panel->view()->select(QStringLiteral("takes"));
        const auto restore = [&] { return inspector->findChild<ui::Button *>(QStringLiteral("graphRestoreTake")); };
        // The shot is take 3, made from the recipe as it is: nothing to go back to.
        QVERIFY(!restore());
        // The recipe changed since: back to take 3's.
        QJsonObject changed = bounceRecipe(3);
        QJsonArray nodes = changed.value("nodes").toArray();
        QJsonObject takes = nodes.last().toObject();
        QJsonArray made = takes.value("takes").toArray();
        QJsonObject third = made.at(2).toObject();
        third.insert("changed", QJsonArray{"the recipe"});
        made.replace(2, third);
        takes.insert("takes", made);
        nodes.replace(nodes.size() - 1, takes);
        changed.insert("nodes", nodes);
        window.handleEvent(changed);
        QVERIFY(restore());
        QCOMPARE(restore()->text(), QStringLiteral("Restore take 3's recipe"));
        restore()->click();
        QCOMPARE(window.m_lastCommand,
                 (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", QString::fromLatin1(kBounce)}, {"restore", 3}}));
        // A take that kept nothing of its recipe, made before takes did, has none to restore.
        third.insert("kept", false);
        made.replace(2, third);
        takes.insert("takes", made);
        nodes.replace(nodes.size() - 1, takes);
        changed.insert("nodes", nodes);
        window.handleEvent(changed);
        QVERIFY(!restore());
    }

    // A step is added from the head, by its kind, and deleted from its card:
    // each a change to the recipe's text, and the step that was added is the
    // one whose card opens.
    void stepsAreAddedFromTheHeadAndDeletedFromTheirCard() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        QVERIFY(panel->addButton()->isVisible() && panel->addButton()->isEnabled());
        const QString was = QString::fromLatin1(kBounce);
        for (const QString kind : {QStringLiteral("guide"), QStringLiteral("model"), QStringLiteral("join")}) {
            emit panel->stepAddRequested(kind);
            QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", was}, {"add", kind}}));
        }
        // The bridge answers with the graph and the line it added: that step's card opens.
        panel->view()->select(QStringLiteral("takes"));
        QJsonObject added = bounceRecipe();
        added.insert("edited", true);
        added.insert("added", 5);
        window.handleEvent(added);
        QCOMPARE(panel->view()->selected(), QStringLiteral("made:ball.png"));
        // Its card ends with taking it out again, by its line.
        auto *remove = panel->inspector()->findChild<ui::GhostButton *>(QStringLiteral("graphDeleteStep"));
        QVERIFY(remove);
        remove->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", was}, {"line", 5}, {"remove", true}}));
        // What is not a step has nothing to delete, and a workflow's nodes are not the recipe's lines.
        for (const QString other : {QStringLiteral("signal:bounce"), QStringLiteral("takes"), QStringLiteral("sequence:court")}) {
            panel->view()->select(other);
            QVERIFY(!panel->inspector()->findChild<ui::GhostButton *>(QStringLiteral("graphDeleteStep")));
        }
        window.handleEvent(bounceWithRunner());
        panel->view()->select(QStringLiteral("made:out"));
        panel->inspector()->findChild<ui::Button *>(QStringLiteral("graphOpenWorkflow"))->click();
        window.handleEvent(shotWorkflow());
        QVERIFY(panel->showsWorkflow() && !panel->addButton()->isVisible());
        panel->view()->select(QStringLiteral("8"));
        QVERIFY(!panel->inspector()->findChild<ui::GhostButton *>(QStringLiteral("graphDeleteStep")));
        // While the shot is made no step is added.
        emit panel->workflowClosed();
        QVERIFY(panel->addButton()->isVisible());
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "m4"}, {"take", 3}, {"line", ""}});
        QVERIFY(!panel->addButton()->isVisible());
    }

    // A shot longer than a model makes at once is its parts and their join: the
    // join is a card of its own, a guide says which part of its sequence it
    // draws, and each is set in its card as any word of the recipe.
    void aLongerShotShowsItsPartsAndTheirJoin() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        QJsonObject recipe = bounceRecipe();
        QJsonArray nodes = recipe.value("nodes").toArray();
        QJsonObject guide = nodes.at(1).toObject();
        guide.insert("from", "5");
        guide.insert("seconds", "5.0625");
        nodes.replace(1, guide);
        QJsonObject part = nodes.at(5).toObject();
        part.insert("id", "made:b.mp4");
        part.insert("makes", "b.mp4");
        part.insert("from", 5.0);
        nodes.replace(5, part);
        nodes.insert(6, QJsonObject{{"id", "made:out"}, {"kind", "join"}, {"title", "Join"},
                                    {"detail", "2 parts, each going on from the last frame of the one before"}, {"gives", "video"},
                                    {"line", 8}, {"makes", "out"}, {"parts", QJsonArray{"a.mp4", "b.mp4"}}, {"lap", 1}});
        recipe.insert("nodes", nodes);
        QJsonArray edges;
        for (const QJsonValue value : recipe.value("edges").toArray()) {
            QJsonObject edge = value.toObject();
            if (edge.value("to").toString() == QLatin1String("made:out")) edge.insert("to", "made:b.mp4");
            if (edge.value("from").toString() == QLatin1String("made:out")) continue;
            edges.append(edge);
        }
        edges.append(QJsonObject{{"from", "made:b.mp4"}, {"to", "made:out"}, {"port", "part"}, {"nth", 1}, {"gives", "video"}});
        edges.append(QJsonObject{{"from", "made:out"}, {"to", "takes"}, {"port", "take"}, {"nth", 0}, {"gives", "video"}});
        recipe.insert("edges", edges);
        window.handleEvent(recipe);
        RecipeGraphPanel *panel = panelOf(window);
        QCOMPARE(panel->view()->recipe().node(QStringLiteral("made:out"))->kind, graph::Kind::Join);

        // The join: how many frames each part shares with the one before it, and what it reads.
        panel->view()->select(QStringLiteral("made:out"));
        QCOMPARE(field(window, QStringLiteral("lap"))->text(), QStringLiteral("1"));
        typeInto(field(window, QStringLiteral("lap")), QStringLiteral("2"));
        QCOMPARE(window.m_lastCommand.value("type").toString(), QStringLiteral("recipe_edit"));
        QCOMPARE(window.m_lastCommand.value("line").toInt(), 8);
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("lap"));
        QCOMPARE(window.m_lastCommand.value("value").toString(), QStringLiteral("2"));
        // A guide's part of its sequence, and where a part of the shot starts in its time.
        panel->view()->select(QStringLiteral("made:depth.mp4"));
        QCOMPARE(field(window, QStringLiteral("from"))->text(), QStringLiteral("5"));
        QCOMPARE(field(window, QStringLiteral("seconds"))->text(), QStringLiteral("5.0625"));
        typeInto(field(window, QStringLiteral("from")), QStringLiteral("10"));
        QCOMPARE(window.m_lastCommand.value("line").toInt(), 3);
        QCOMPARE(window.m_lastCommand.value("key").toString(), QStringLiteral("from"));
        panel->view()->select(QStringLiteral("made:b.mp4"));
        QCOMPARE(field(window, QStringLiteral("from"))->text(), QStringLiteral("5"));
    }

    // The takes' card holds each take to its guide when asked: where the take
    // has the shot's subject beside where it was asked for, read by the bridge
    // a take at a time with no model, the closest said. The same reading an
    // agent's `look` makes with `takes`.
    void takesAreHeldToTheirGuideInTheirCard() {
        MainWindow window{QString(), QString()};
        openBounce(window, 3);
        RecipeGraphPanel *panel = panelOf(window);
        RecipeInspector *inspector = panel->inspector();
        panel->view()->select(QStringLiteral("takes"));
        const auto hold = [&] { return inspector->findChild<ui::Button *>(QStringLiteral("graphHoldTakes")); };
        const auto says = [&] { return inspector->findChild<QLabel *>(QStringLiteral("graphHoldSays")); };
        const auto row = [&](int number) { return inspector->findChild<QAbstractButton *>(QStringLiteral("graphTake%1").arg(number)); };
        QVERIFY(hold() && hold()->isEnabled() && !says());
        QCOMPARE(hold()->text(), QStringLiteral("Hold to guide"));
        QCOMPARE(row(1)->accessibleDescription(), QStringLiteral("the recipe changed since"));
        hold()->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "takes_read"}, {"media", "m4"}}));
        // While it is read nothing more is asked, and each take says its number as it comes.
        QVERIFY(!hold()->isEnabled());
        QCOMPARE(hold()->text(), QStringLiteral("Reading…"));
        QVERIFY(says()->text().contains(QStringLiteral("following the subject")));
        const auto read = [](int take, double overlap, int frames) {
            return QJsonObject{{"type", "take_read"}, {"media", "m4"}, {"take", take}, {"overlap", overlap},
                               {"clips", QJsonArray{QJsonObject{{"clip", "ball"}, {"overlap", overlap}, {"off", 0.04}, {"size", 1.1},
                                                                {"frames", frames}, {"shown", 51}}}}};
        };
        window.handleEvent(read(1, 0.21, 12));
        QCOMPARE(row(1)->accessibleDescription(), QStringLiteral("0.21 of its guide · the recipe changed since"));
        QCOMPARE(row(2)->accessibleDescription(), QStringLiteral("roll 1"));
        // Another shot's reading is not this one's.
        QJsonObject other = read(2, 0.9, 51);
        other.insert("media", "m9");
        window.handleEvent(other);
        QCOMPARE(row(2)->accessibleDescription(), QStringLiteral("roll 1"));
        window.handleEvent(read(2, 0.62, 48));
        window.handleEvent(QJsonObject{{"type", "take_read"}, {"media", "m4"}, {"take", 3}, {"overlap", 0.0},
                                       {"clips", QJsonArray{QJsonObject{{"clip", "ball"}, {"overlap", 0.0},
                                                                        {"note", "`003.mp4` has nothing to follow where the guide has `ball`"}}}}});
        window.handleEvent(QJsonObject{{"type", "takes_read"}, {"media", "m4"}, {"best", 2}, {"takes", 3}, {"sequence", "court"},
                                       {"clips", QJsonArray{"ball"}}});
        QCOMPARE(row(2)->accessibleDescription(), QStringLiteral("0.62 of its guide, the closest · roll 1"));
        QCOMPARE(row(3)->accessibleDescription(), QStringLiteral("0.00 of its guide · roll 2"));
        QVERIFY(ui::tipName(row(2)).contains(QStringLiteral("ball: 0.62 of where it was asked for, followed on 48 of the 51 frames")));
        QVERIFY(ui::tipName(row(3)).contains(QStringLiteral("has nothing to follow")));
        QVERIFY(hold()->isEnabled());
        QCOMPARE(hold()->text(), QStringLiteral("Hold to guide again"));
        QVERIFY(says()->text().startsWith(QStringLiteral("1 is a take that has the subject where")));
        // The take that holds best is still chosen by a press, as any take.
        row(2)->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "media_replace"}, {"media", "m4"}, {"path", "shots/bounce.takes/002.mp4"}}));
        // The graph read again keeps what was read.
        window.handleEvent(bounceRecipe(3));
        QCOMPARE(row(2)->accessibleDescription(), QStringLiteral("0.62 of its guide, the closest · roll 1"));

        // A shot that says where nothing goes has nothing to hold a take to: said in the card, with no banner.
        hold()->click();
        window.m_errorBanner->hide();
        window.handleEvent(QJsonObject{{"type", "error"}, {"command", "takes_read"}, {"media", "m4"},
                                       {"message", "the recipe's model is not told where anything goes (`moves=<clip>`)"}});
        QVERIFY(says()->text().contains(QStringLiteral("moves=<clip>")));
        QVERIFY(hold()->isEnabled());
        QVERIFY(!window.m_errorBanner->isVisible());

        // A take's picture leads its row, and a guide shows as it was drawn for the take that shows.
        QImage picture(160, 90, QImage::Format_RGB32);
        picture.fill(Qt::darkGreen);
        const int plain = row(1)->height();
        inspector->setPictures({{QStringLiteral("shots/bounce.takes/001.mp4"), picture},
                                {QStringLiteral("shots/bounce.takes/003.work/depth.mp4"), picture}});
        inspector->setNode(panel->view()->recipe(), QStringLiteral("takes"));
        QVERIFY(row(1)->height() > plain);
        QCOMPARE(row(2)->height(), plain);
        QJsonObject drawn = bounceRecipe(3);
        QJsonArray nodes = drawn.value("nodes").toArray();
        QJsonObject guide = nodes.at(1).toObject();
        guide.insert("made", QJsonObject{{"take", 3}, {"path", "shots/bounce.takes/003.work/depth.mp4"}});
        nodes.replace(1, guide);
        drawn.insert("nodes", nodes);
        window.handleEvent(drawn);
        inspector->setPictures({{QStringLiteral("shots/bounce.takes/003.work/depth.mp4"), picture}});
        panel->view()->select(QStringLiteral("made:depth.mp4"));
        auto *shown = inspector->findChild<QLabel *>(QStringLiteral("graphGuidePicture"));
        QVERIFY(shown && !shown->pixmap().isNull());
        QCOMPARE(shown->accessibleName(), QStringLiteral("The guide as drawn for take 3"));
    }

    // A model its entry runs with kerf-comfy opens into the ComfyUI workflow it
    // is run with: every node a card, every input a row, the blanks the recipe
    // fills marked; the head's arrow goes back to the shot's graph.
    void aModelRunByKerfComfyOpensIntoItsWorkflow() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        RecipeGraphView *view = panel->view();
        RecipeInspector *inspector = panel->inspector();
        // A model any other program runs has no workflow to open, and says nothing of one.
        view->select(QStringLiteral("made:out"));
        QVERIFY(!inspector->findChild<ui::Button *>(QStringLiteral("graphOpenWorkflow")));
        emit view->nodeActivated(QStringLiteral("made:out"));
        QVERIFY(window.m_lastCommand.value("type").toString() != QStringLiteral("workflow_get"));
        QVERIFY(!panel->findChild<ui::IconButton *>(QStringLiteral("graphBack"))->isVisible());

        window.handleEvent(bounceWithRunner());
        auto *open = inspector->findChild<ui::Button *>(QStringLiteral("graphOpenWorkflow"));
        QVERIFY(open);
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphWorkflowSays"))->text(), QStringLiteral("shot, as it comes with kerf-comfy."));
        open->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "workflow_get"}, {"media", "m4"}, {"line", 6}}));
        // Until the bridge says it, the shot's graph stays.
        QVERIFY(!panel->showsWorkflow());
        // Another step's workflow, or another shot's, is not the one asked for.
        QJsonObject other = shotWorkflow();
        other.insert("line", 5);
        window.handleEvent(other);
        QVERIFY(!panel->showsWorkflow());

        window.handleEvent(shotWorkflow());
        QVERIFY(panel->showsWorkflow());
        QVERIFY(panel->findChild<ui::IconButton *>(QStringLiteral("graphBack"))->isVisible());
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphTitle"))->text(), QStringLiteral("bounce"));
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphPath"))->text(), QStringLiteral("video · shot"));
        // Every node is a card, and every link an edge to the row of the input it is.
        const graph::Recipe &shown = view->recipe();
        QVERIFY(shown.parts);
        QCOMPARE(shown.nodes.size(), 5);
        QCOMPARE(shown.edges.size(), 5);
        QCOMPARE(shown.node(QStringLiteral("part:4"))->title, QStringLiteral("Prompt"));
        QCOMPARE(shown.node(QStringLiteral("part:8"))->title, QStringLiteral("KSampler"));
        const QRectF sampler = view->cardRect(QStringLiteral("part:8"));
        QVERIFY(sampler.height() > view->cardRect(QStringLiteral("part:6")).height());
        for (int index = 0; index < shown.edges.size(); ++index) {
            const QPainterPath path = view->edgePath(index);
            const QRectF to = view->cardRect(shown.edges.at(index).to);
            const QPointF end = path.pointAtPercent(1.0);
            QVERIFY(std::abs(end.x() - to.left()) < 1.0 && end.y() > to.top() + 20 && end.y() < to.bottom());
        }
        // Every input is shown on its node's card: a blank in the accent's ink, a value plain.
        TextRecorder texts;
        view->render(&texts);
        for (const char *said : {"KSampler", "Prompt", "SaveVideo", "sampler_name", "uni_pc", "positive", "filename_prefix", "#8"})
            QVERIFY2(texts.colorOf(QString::fromLatin1(said)).isValid(), said);
        for (const char *blank : {"{prompt}", "{seed}", "{steps}", "{guide}", "{unet}"})
            QCOMPARE(texts.colorOf(QString::fromLatin1(blank)), tokens::color(tokens::accentInk));
        QCOMPARE(texts.colorOf(QStringLiteral("uni_pc")), tokens::color(tokens::ink2));
        // Its legend is of what its rows are, not of what a shot's edges carry.
        QVERIFY(texts.colorOf(QStringLiteral("the recipe fills")).isValid() && texts.colorOf(QStringLiteral("set here")).isValid());
        QVERIFY(!texts.colorOf(QStringLiteral("signal")).isValid());
        // No edge is drawn by hand in a workflow: its links are its own shape.
        QCOMPARE(view->portAt(view->portPoint(QStringLiteral("part:1"))), QString());

        // With no node chosen the card says the workflow, where it comes from, and what fills each blank.
        QCOMPARE(inspector->nodeId(), QString());
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphInspectorTitle"))->text(), QStringLiteral("shot"));
        QVERIFY(inspector->findChild<QLabel *>(QStringLiteral("graphWorkflowOrigin"))->text().startsWith(QStringLiteral("As it comes with kerf-comfy.")));
        QStringList blanks;
        for (const QLabel *label : inspector->findChildren<QLabel *>())
            if (label->property("graphBlank").isValid()) blanks << label->text();
        QVERIFY(blanks.contains(QStringLiteral("The line's prompt, with the cast and styles it names. Now: a basketball bounces on a court")));
        QVERIFY(blanks.contains(QStringLiteral("20 unless the line says steps=")));
        // The shot is made from here as from its graph.
        QVERIFY(panel->makeButton()->isVisible());

        // Closed and opened again, the shot's own graph is what opens.
        window.m_shell->dismiss();
        window.showGraph(QStringLiteral("m4"));
        QVERIFY(!panel->showsWorkflow());
        emit panel->workflowRequested(6);
        window.handleEvent(shotWorkflow());
        QVERIFY(panel->showsWorkflow());

        // The arrow goes back to the shot's graph, on the model it was the workflow of.
        panel->findChild<ui::IconButton *>(QStringLiteral("graphBack"))->click();
        QVERIFY(!panel->showsWorkflow());
        QVERIFY(!view->recipe().parts);
        QCOMPARE(view->selected(), QStringLiteral("made:out"));
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphPath"))->text(), QStringLiteral("shots/bounce.kerfrecipe"));
        // A double-click on the model goes in as well.
        window.m_lastCommand = {};
        mouse(view, QEvent::MouseButtonDblClick, view->cardRect(QStringLiteral("made:out")).center());
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "workflow_get"}, {"media", "m4"}, {"line", 6}}));
    }

    // In a workflow, a blank is the recipe's and a link the graph's shape; any
    // other value is set in its node's card, and is kept in the shot's own
    // copy of the workflow, which its line then names.
    void aWorkflowsValuesAreSetInTheShotsOwnCopy() {
        MainWindow window{QString(), QString()};
        openBounce(window);
        window.handleEvent(bounceWithRunner());
        RecipeGraphPanel *panel = panelOf(window);
        RecipeInspector *inspector = panel->inspector();
        emit panel->workflowRequested(6);
        window.handleEvent(shotWorkflow());
        QVERIFY(panel->showsWorkflow());
        const QString was = shotWorkflow().value("text").toString();
        panel->view()->select(QStringLiteral("part:8"));
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphInspectorTitle"))->text(), QStringLiteral("KSampler"));
        const auto sent = [&](const QString &input, const QJsonValue &value) {
            return QJsonObject{{"type", "workflow_edit"}, {"media", "m4"}, {"line", 6}, {"was", was}, {"node", "8"}, {"input", input}, {"value", value}};
        };
        // A blank is marked and is not a field: it is set on the recipe's line.
        QVERIFY(!field(window, QStringLiteral("input.steps")) && !field(window, QStringLiteral("input.seed")));
        auto *steps = inspector->findChild<QLabel *>(QStringLiteral("graphBlank.steps"));
        QVERIFY(steps);
        QCOMPARE(steps->text(), QStringLiteral("{steps}"));
        QCOMPARE(steps->palette().color(QPalette::WindowText), tokens::color(tokens::accentInk));
        // A link is the graph's shape: said, not set.
        QVERIFY(!field(window, QStringLiteral("input.model")));
        // A text the workflow sets, set to another.
        QLineEdit *sampler = field(window, QStringLiteral("input.sampler_name"));
        QVERIFY(sampler);
        QCOMPARE(sampler->text(), QStringLiteral("uni_pc"));
        typeInto(sampler, QStringLiteral("euler"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("sampler_name"), QStringLiteral("euler")));
        // A number stays a number, and what is no number is put back unsent.
        QLineEdit *cfg = field(window, QStringLiteral("input.cfg"));
        QCOMPARE(cfg->text(), QStringLiteral("6"));
        typeInto(cfg, QStringLiteral("4.5"));
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("cfg"), 4.5));
        window.m_lastCommand = {};
        typeInto(cfg, QStringLiteral("high"));
        QCOMPARE(window.m_lastCommand, QJsonObject{});
        QCOMPARE(cfg->text(), QStringLiteral("4.5"));
        // A yes or no is a switch.
        auto *tiled = inspector->findChild<QAbstractButton *>(QStringLiteral("graphField.input.tiled"));
        QVERIFY(tiled && tiled->isCheckable() && !tiled->isChecked());
        tiled->click();
        QCOMPARE(window.m_lastCommand, sent(QStringLiteral("tiled"), true));

        // The bridge answers with the shot's own copy, and the recipe whose line now names it.
        window.handleEvent(shotWorkflow(true, QStringLiteral("euler")));
        window.handleEvent(bounceWithRunner(QStringLiteral("bounce.video.comfy.json")));
        QVERIFY(panel->showsWorkflow());
        QCOMPARE(field(window, QStringLiteral("input.sampler_name")), sampler);
        QCOMPARE(sampler->text(), QStringLiteral("euler"));
        QCOMPARE(panel->findChild<QLabel *>(QStringLiteral("graphPath"))->text(), QStringLiteral("video · shots/bounce.video.comfy.json"));
        // The next change is to the copy as it now reads.
        typeInto(sampler, QStringLiteral("dpmpp_2m"));
        QCOMPARE(window.m_lastCommand.value("was").toString(), shotWorkflow(true, QStringLiteral("euler")).value("text").toString());
        // The workflow's own card says it is the shot's copy, and goes back to the one that comes.
        panel->view()->select(QString());
        QVERIFY(inspector->findChild<QLabel *>(QStringLiteral("graphWorkflowOrigin"))->text().contains(QStringLiteral("shots/bounce.video.comfy.json")));
        inspector->findChild<ui::GhostButton *>(QStringLiteral("graphWorkflowDrop"))->click();
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "recipe_edit"}, {"media", "m4"}, {"was", QString::fromLatin1(kBounce)}, {"line", 6},
                                                    {"key", "workflow"}, {"nth", 0}}));
        // The line changed under the workflow that shows: it is read again.
        QJsonObject edited = bounceWithRunner();
        edited.insert("edited", true);
        window.handleEvent(edited);
        QCOMPARE(window.m_lastCommand, (QJsonObject{{"type", "workflow_get"}, {"media", "m4"}, {"line", 6}}));
        // Back in the shot's graph, the model says which workflow it is run with.
        window.handleEvent(bounceWithRunner(QStringLiteral("bounce.video.comfy.json")));
        panel->findChild<ui::IconButton *>(QStringLiteral("graphBack"))->click();
        QCOMPARE(inspector->findChild<QLabel *>(QStringLiteral("graphWorkflowSays"))->text(), QStringLiteral("bounce.video.comfy.json, this shot's own copy."));
    }

    // KERF_SCREENSHOT_DIR=<dir> saves the graph over the editor, and with each
    // kind of node chosen, to look at.
    void theGraphForTheGallery() {
        const QString shots = qEnvironmentVariable("KERF_SCREENSHOT_DIR");
        if (shots.isEmpty()) QSKIP("KERF_SCREENSHOT_DIR is not set");
        MainWindow window{QString(), QString()};
        openBounce(window);
        RecipeGraphPanel *panel = panelOf(window);
        QVERIFY(save(window, QStringLiteral("graph-bounce")));
        for (const auto &[id, name] : {std::pair{"made:out", "model"}, std::pair{"signal:bounce", "signal"}, std::pair{"made:depth.mp4", "guide"},
                                       std::pair{"moves:ball", "path"}, std::pair{"takes", "takes"}}) {
            panel->view()->select(QString::fromLatin1(id));
            QVERIFY(save(window, QStringLiteral("graph-bounce-%1").arg(QString::fromLatin1(name))));
        }
        window.handleEvent(QJsonObject{{"type", "media_making"}, {"media", "m4"}, {"take", 3}, {"line", "running, 12 s"}});
        QVERIFY(save(window, QStringLiteral("graph-bounce-making")));
        QVERIFY(save(*panel, QStringLiteral("graph-bounce-panel")));
        window.handleEvent(QJsonObject{{"type", "media_make_cancelled"}, {"media", "m4"}});
        // A model's workflow: as it comes, and with a node chosen in the shot's own copy.
        window.handleEvent(bounceWithRunner());
        emit panel->workflowRequested(6);
        window.handleEvent(shotWorkflow());
        window.m_errorBanner->hide();
        window.showStatus(QString());
        QVERIFY(save(window, QStringLiteral("graph-workflow")));
        window.handleEvent(shotWorkflow(true, QStringLiteral("euler")));
        panel->view()->select(QStringLiteral("part:8"));
        QVERIFY(save(window, QStringLiteral("graph-workflow-node")));
    }
};

} // namespace kerf

KERF_TEST_CLASS(kerf::RecipeGraphTest)

#include "recipe_graph_test.moc"
