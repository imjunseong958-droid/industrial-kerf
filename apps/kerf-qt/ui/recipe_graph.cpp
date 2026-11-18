// A shot's recipe as a graph, and the canvas that draws it (recipe_graph.h).

#include "recipe_graph.h"
#include "activity.h"
#include "components.h"
#include "icons.h"
#include "motion_path.h"

#include <QCryptographicHash>
#include <QFontMetricsF>
#include <QJsonArray>
#include <QJsonDocument>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QNativeGestureEvent>
#include <QPainter>
#include <QSettings>
#include <QTimer>
#include <QVariantMap>
#include <QWheelEvent>

#include <algorithm>
#include <cmath>

namespace kerf {

using namespace tokens;

namespace graph {

namespace {

Gives givesOf(const QString &name) {
    static const QHash<QString, Gives> named{{QStringLiteral("video"), Gives::Video},   {QStringLiteral("image"), Gives::Image},
                                             {QStringLiteral("signal"), Gives::Signal}, {QStringLiteral("path"), Gives::Path},
                                             {QStringLiteral("words"), Gives::Words},   {QStringLiteral("file"), Gives::File}};
    return named.value(name, Gives::File);
}

Kind kindOf(const QString &name) {
    static const QHash<QString, Kind> named{
        {QStringLiteral("sequence"), Kind::Sequence}, {QStringLiteral("render"), Kind::Render}, {QStringLiteral("model"), Kind::Model},
        {QStringLiteral("run"), Kind::Run},           {QStringLiteral("join"), Kind::Join},
        {QStringLiteral("file"), Kind::File},     {QStringLiteral("signal"), Kind::Signal},
        {QStringLiteral("moves"), Kind::Moves},       {QStringLiteral("cast"), Kind::Cast},     {QStringLiteral("style"), Kind::Style},
        {QStringLiteral("takes"), Kind::Takes}};
    return named.value(name, Kind::File);
}

// Whether a card draws something under its two lines: a signal's number over
// its time, or the path a clip is led along.
bool drawsCurve(const Node &node) {
    if (node.kind == Kind::Signal) return !node.holds.value(QStringLiteral("values")).toArray().isEmpty();
    if (node.kind == Kind::Moves) return !node.holds.value(QStringLiteral("path")).toString().isEmpty();
    return false;
}

} // namespace

const Node *Recipe::node(const QString &id) const {
    for (const Node &node : nodes)
        if (node.id == id) return &node;
    return nullptr;
}

QList<Edge> Recipe::into(const QString &id) const {
    QList<Edge> found;
    for (const Edge &edge : edges)
        if (edge.to == id) found << edge;
    return found;
}

Recipe Recipe::read(const QJsonObject &event) {
    Recipe recipe;
    recipe.media = event.value(QStringLiteral("media")).toString();
    recipe.recipe = event.value(QStringLiteral("recipe")).toString();
    recipe.name = event.value(QStringLiteral("name")).toString();
    recipe.text = event.value(QStringLiteral("text")).toString();
    recipe.error = event.value(QStringLiteral("error")).toString();
    recipe.cannot = event.value(QStringLiteral("cannot")).toString();
    recipe.modelsError = event.value(QStringLiteral("modelsError")).toString();
    for (const QJsonValue note : event.value(QStringLiteral("notes")).toArray()) recipe.notes << note.toString();
    for (const QJsonValue value : event.value(QStringLiteral("nodes")).toArray()) {
        const QJsonObject object = value.toObject();
        Node node;
        node.id = object.value(QStringLiteral("id")).toString();
        node.kind = kindOf(object.value(QStringLiteral("kind")).toString());
        node.title = object.value(QStringLiteral("title")).toString();
        node.detail = object.value(QStringLiteral("detail")).toString();
        node.gives = givesOf(object.value(QStringLiteral("gives")).toString());
        node.line = object.value(QStringLiteral("line")).toInt();
        node.holds = object;
        if (!node.id.isEmpty()) recipe.nodes << node;
    }
    for (const QJsonValue value : event.value(QStringLiteral("edges")).toArray()) {
        const QJsonObject object = value.toObject();
        Edge edge;
        edge.from = object.value(QStringLiteral("from")).toString();
        edge.to = object.value(QStringLiteral("to")).toString();
        edge.port = object.value(QStringLiteral("port")).toString();
        edge.nth = object.value(QStringLiteral("nth")).toInt();
        edge.gives = givesOf(object.value(QStringLiteral("gives")).toString());
        // An edge to or from what is not there is not drawn.
        if (recipe.node(edge.from) && recipe.node(edge.to)) recipe.edges << edge;
    }
    return recipe;
}

InputKind inputKind(const QJsonObject &input) {
    if (input.contains(QStringLiteral("link"))) return InputKind::Link;
    return input.value(QStringLiteral("blanks")).toArray().isEmpty() ? InputKind::Value : InputKind::Blank;
}

QString valueText(const QJsonValue &value) {
    if (value.isString()) return value.toString();
    if (value.isBool()) return value.toBool() ? QStringLiteral("true") : QStringLiteral("false");
    if (value.isDouble()) return QString::number(value.toDouble(), 'g', 12);
    if (value.isNull() || value.isUndefined()) return {};
    return QString::fromUtf8(QJsonDocument(value.isArray() ? QJsonDocument(value.toArray()) : QJsonDocument(value.toObject())).toJson(QJsonDocument::Compact));
}

Workflow Workflow::read(const QJsonObject &event) {
    Workflow workflow;
    workflow.media = event.value(QStringLiteral("media")).toString();
    workflow.line = event.value(QStringLiteral("line")).toInt();
    workflow.model = event.value(QStringLiteral("model")).toString();
    workflow.name = event.value(QStringLiteral("name")).toString();
    workflow.origin = event.value(QStringLiteral("origin")).toString();
    workflow.file = event.value(QStringLiteral("file")).toString();
    workflow.about = event.value(QStringLiteral("about")).toString();
    workflow.text = event.value(QStringLiteral("text")).toString();
    workflow.blanks = event.value(QStringLiteral("blanks")).toObject();
    Recipe &graph = workflow.graph;
    graph.media = workflow.media;
    graph.name = workflow.model;
    graph.recipe = workflow.isOwn() ? workflow.file : workflow.name;
    graph.text = workflow.text;
    graph.parts = true;
    const auto idOf = [](const QString &id) { return QStringLiteral("part:%1").arg(id); };
    for (const QJsonValue value : event.value(QStringLiteral("nodes")).toArray()) {
        const QJsonObject object = value.toObject();
        Node node;
        const QString id = object.value(QStringLiteral("id")).toString();
        node.id = idOf(id);
        node.kind = Kind::Part;
        // By what ComfyUI's own editor calls it where its export says, else by its kind.
        const QString kind = object.value(QStringLiteral("class")).toString();
        node.title = object.value(QStringLiteral("title")).toString(kind);
        node.detail = node.title == kind ? QString() : kind;
        node.gives = Gives::File;
        node.holds = object;
        if (!id.isEmpty()) graph.nodes << node;
    }
    for (const Node &node : std::as_const(graph.nodes)) {
        for (const QJsonValue value : node.holds.value(QStringLiteral("inputs")).toArray()) {
            const QJsonObject input = value.toObject();
            const QJsonArray link = input.value(QStringLiteral("link")).toArray();
            if (link.isEmpty()) continue;
            Edge edge;
            edge.from = idOf(link.at(0).toString());
            edge.to = node.id;
            edge.port = input.value(QStringLiteral("name")).toString();
            edge.gives = Gives::File;
            if (graph.node(edge.from)) graph.edges << edge;
        }
    }
    return workflow;
}

QColor colorOf(Gives gives) {
    switch (gives) {
    case Gives::Video: return color(tokens::nodes::video);
    case Gives::Image: return color(tokens::nodes::image);
    case Gives::Signal: return color(tokens::nodes::signal);
    case Gives::Path: return color(tokens::nodes::path);
    case Gives::Words: return color(tokens::nodes::words);
    case Gives::File: return color(tokens::nodes::file);
    }
    return color(tokens::nodes::file);
}

QString nameOf(Gives gives) {
    switch (gives) {
    case Gives::Video: return QStringLiteral("video");
    case Gives::Image: return QStringLiteral("image");
    case Gives::Signal: return QStringLiteral("signal");
    case Gives::Path: return QStringLiteral("path");
    case Gives::Words: return QStringLiteral("words");
    case Gives::File: return QStringLiteral("file");
    }
    return QStringLiteral("file");
}

QString glyphOf(const Node &node) {
    switch (node.kind) {
    case Kind::Sequence: return QStringLiteral("g-timeline");
    case Kind::Render: return QStringLiteral("g-picture");
    case Kind::Model: return QStringLiteral("g-model");
    case Kind::Run: return QStringLiteral("g-run");
    case Kind::Join: return QStringLiteral("g-video");
    case Kind::File: return node.gives == Gives::Image ? QStringLiteral("g-image")
                          : node.gives == Gives::Video ? QStringLiteral("g-video") : QStringLiteral("g-file");
    case Kind::Signal: return QStringLiteral("g-signal");
    case Kind::Moves: return QStringLiteral("g-mpen");
    case Kind::Cast: return QStringLiteral("g-user");
    case Kind::Style: return QStringLiteral("g-fx");
    case Kind::Takes: return QStringLiteral("g-video");
    case Kind::Part: return QStringLiteral("g-fx");
    }
    return QStringLiteral("g-file");
}

bool isMissing(const Node &node) {
    if (node.kind == Kind::Model) return !node.holds.value(QStringLiteral("setUp")).toBool(true);
    return node.holds.contains(QStringLiteral("there")) && !node.holds.value(QStringLiteral("there")).toBool();
}

QSizeF cardSize(const Node &node) {
    // A workflow's node is its name and a row for each of its inputs.
    if (node.kind == Kind::Part)
        return QSizeF(layout::graphPartWidth,
                      layout::graphPartHead + layout::graphPartRow * node.holds.value(QStringLiteral("inputs")).toArray().size() + 8);
    qreal height = layout::graphCardHeight;
    // A model says a third thing: its seed and what else its line sets.
    if (node.kind == Kind::Model) height += 18;
    if (drawsCurve(node)) height += layout::graphCardCurve;
    return QSizeF(layout::graphCardWidth, height);
}

QStringList portsFor(const Recipe &recipe, const Node &from, const Node &to) {
    Q_UNUSED(recipe);
    if (from.id == to.id || !to.isStep()) return {};
    // A step reads what a step before it made, never one after.
    if (from.isStep() && from.line >= to.line) return {};
    if (to.kind == Kind::Render) return from.kind == Kind::Sequence ? QStringList{QStringLiteral("sequence")} : QStringList{};
    if (to.kind != Kind::Model) return {};
    switch (from.kind) {
    case Kind::Signal: return {QStringLiteral("signal")};
    case Kind::Moves: return {QStringLiteral("moves")};
    case Kind::Cast: return {QStringLiteral("cast")};
    case Kind::Style: return {QStringLiteral("style")};
    case Kind::Sequence:
    case Kind::Takes: return {};
    default: break;
    }
    if (from.gives == Gives::Video) {
        QStringList ports{QStringLiteral("guide")};
        // The guides it reads beside the first, by what each is of.
        for (const QJsonValue read : to.holds.value(QStringLiteral("reads")).toArray())
            if (read.toString().startsWith(QStringLiteral("guide_"))) ports << read.toString();
        // As a first frame it is where it ended: the shot goes on from it.
        return ports << QStringLiteral("ref") << QStringLiteral("first");
    }
    if (from.gives == Gives::Image) return {QStringLiteral("first"), QStringLiteral("last"), QStringLiteral("ref")};
    return {QStringLiteral("ref")};
}

QString valueOf(const Node &from) {
    switch (from.kind) {
    case Kind::Signal:
    case Kind::Moves: return from.holds.value(QStringLiteral("clip")).toString();
    case Kind::Cast:
    case Kind::Style: return from.holds.value(QStringLiteral("name")).toString();
    case Kind::File: return from.holds.value(QStringLiteral("path")).toString();
    case Kind::Sequence: return from.holds.value(QStringLiteral("sequence")).toString();
    case Kind::Takes: return {};
    default: break;
    }
    // A step's file, by the name it has in braces: a program may make several, the first of them.
    const QJsonValue makes = from.holds.value(QStringLiteral("makes"));
    const QString name = makes.isArray() ? makes.toArray().first().toString() : makes.toString();
    return name.isEmpty() ? QString() : QStringLiteral("{%1}").arg(name);
}

QString portSays(const QString &port) {
    static const QHash<QString, QString> said{
        {QStringLiteral("guide"), QStringLiteral("a picture in motion to follow")},
        {QStringLiteral("first"), QStringLiteral("the first frame")},
        {QStringLiteral("last"), QStringLiteral("the last frame")},
        {QStringLiteral("ref"), QStringLiteral("who or what is in it, or how it looks")},
        {QStringLiteral("signal"), QStringLiteral("when, and how much")},
        {QStringLiteral("moves"), QStringLiteral("where a thing goes")},
        {QStringLiteral("cast"), QStringLiteral("who or what it shows")},
        {QStringLiteral("style"), QStringLiteral("how it looks")},
        {QStringLiteral("sequence"), QStringLiteral("what the guide is drawn from")}};
    if (port.startsWith(QStringLiteral("guide_"))) return QStringLiteral("another picture in motion to follow, of its %1").arg(port.mid(6));
    return said.value(port);
}

namespace {

// A card's height once it has room for each thing it reads to come in apart.
qreal heightFor(const Recipe &recipe, const Node &node) {
    // A workflow's node has a row for each input already.
    if (node.kind == Kind::Part) return cardSize(node).height();
    return std::max(cardSize(node).height(), 16.0 + 24.0 * recipe.into(node.id).size());
}

} // namespace

QHash<QString, QPointF> arranged(const Recipe &recipe) {
    QHash<QString, int> column;
    QHash<QString, int> order;
    for (int index = 0; index < recipe.nodes.size(); ++index) {
        column.insert(recipe.nodes.at(index).id, 0);
        order.insert(recipe.nodes.at(index).id, index);
    }
    // How far each is from what it reads: the longest way to it.
    for (int pass = 0; pass < recipe.nodes.size(); ++pass) {
        bool moved = false;
        for (const Edge &edge : recipe.edges)
            if (column.value(edge.to) < column.value(edge.from) + 1) {
                column[edge.to] = column.value(edge.from) + 1;
                moved = true;
            }
        if (!moved) break;
    }
    // What nothing comes into stands beside the first thing that reads it,
    // not at the far left with a long way to go.
    for (const Node &node : recipe.nodes) {
        if (!recipe.into(node.id).isEmpty()) continue;
        int nearest = -1;
        for (const Edge &edge : recipe.edges)
            if (edge.from == node.id && (nearest < 0 || column.value(edge.to) < nearest)) nearest = column.value(edge.to);
        if (nearest > 0) column[node.id] = nearest - 1;
    }
    int last = 0;
    for (const int at : std::as_const(column)) last = std::max(last, at);
    // In a column, top to bottom, in the order of what each feeds: by where
    // its reader stands, then by which of the reader's inputs it is.
    QHash<QString, int> row;
    QHash<QString, QPointF> places;
    for (int at = last; at >= 0; --at) {
        QList<const Node *> here;
        for (const Node &node : recipe.nodes)
            if (column.value(node.id) == at) here << &node;
        const auto key = [&](const Node *node) {
            int best = INT_MAX;
            for (const Edge &edge : recipe.edges) {
                if (edge.from != node->id) continue;
                const QList<Edge> into = recipe.into(edge.to);
                int input = 0;
                for (int index = 0; index < into.size(); ++index)
                    if (into.at(index).from == node->id) input = index;
                best = std::min(best, row.value(edge.to) * 64 + input);
            }
            return best;
        };
        std::stable_sort(here.begin(), here.end(), [&](const Node *a, const Node *b) {
            const int ka = key(a);
            const int kb = key(b);
            return ka != kb ? ka < kb : order.value(a->id) < order.value(b->id);
        });
        qreal total = 0;
        for (const Node *node : here) total += heightFor(recipe, *node);
        total += layout::graphRowGap * std::max<qsizetype>(0, here.size() - 1);
        qreal y = -total / 2.0;
        for (int index = 0; index < here.size(); ++index) {
            const Node *node = here.at(index);
            row.insert(node->id, index);
            const qreal pitch = cardSize(*node).width() + (node->kind == Kind::Part ? layout::graphPartGap : layout::graphColumnGap);
            places.insert(node->id, QPointF(at * pitch, std::round(y)));
            y += heightFor(recipe, *node) + layout::graphRowGap;
        }
    }
    return places;
}

} // namespace graph

using graph::Edge;
using graph::Gives;
using graph::Kind;
using graph::Node;

namespace {

constexpr qreal kZoomLeast = 0.35;
constexpr qreal kZoomMost = 2.0;
// The smallest a graph is shown at by itself: smaller than this its words are
// not read, so a graph too wide for the canvas starts at its left at this
// size, to be moved along, and Fit takes all of it in.
constexpr qreal kZoomRead = 0.7;
// A press that moves less than this is a click.
constexpr qreal kDragFrom = 3.0;
// How near the pointer has to be to a port to be on it, in points on the screen.
constexpr qreal kPortReach = 9.0;
// The legend: a line of each kind and its name, in a card at the canvas's foot.
constexpr int kLegendHeight = 28;
constexpr int kLegendSample = 16;
// A workflow's legend: what its rows are.
const QString kLegendLink = QStringLiteral("link");
const QString kLegendBlank = QStringLiteral("{blank}");
const QString kLegendBlankSays = QStringLiteral("the recipe fills");
const QString kLegendValue = QStringLiteral("value");
const QString kLegendValueSays = QStringLiteral("set here");

QString settingsKey(const QString &key) {
    return QStringLiteral("graph/placed/%1")
        .arg(QString::fromLatin1(QCryptographicHash::hash(key.toUtf8(), QCryptographicHash::Sha1).toHex().left(16)));
}

QColor withAlpha(QColor color, qreal alpha) {
    color.setAlphaF(alpha);
    return color;
}

// A model's third line: its seed, how long, how large, and what else its line sets.
QString modelLine(const Node &node) {
    QStringList parts;
    const QJsonObject &holds = node.holds;
    if (!holds.value(QStringLiteral("seed")).isNull()) parts << QStringLiteral("seed %1").arg(holds.value(QStringLiteral("seed")).toVariant().toLongLong());
    if (!holds.value(QStringLiteral("seconds")).isNull()) parts << QStringLiteral("%1 s").arg(holds.value(QStringLiteral("seconds")).toDouble());
    if (!holds.value(QStringLiteral("size")).isNull()) parts << holds.value(QStringLiteral("size")).toString();
    for (const QJsonValue value : holds.value(QStringLiteral("knobs")).toArray()) {
        const QJsonObject knob = value.toObject();
        if (!knob.value(QStringLiteral("value")).isNull())
            parts << QStringLiteral("%1 %2").arg(knob.value(QStringLiteral("name")).toString(), knob.value(QStringLiteral("value")).toString());
    }
    return parts.join(QStringLiteral(" · "));
}

} // namespace

RecipeGraphView::RecipeGraphView(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("recipeGraph"));
    setAccessibleName(QStringLiteral("Graph"));
    setFocusPolicy(Qt::StrongFocus);
    setMouseTracking(true);
    // Beside the legend: smaller, larger, and the whole graph in view again.
    m_zoomOut = new ui::IconButton(QStringLiteral("g-zout"), ui::IconButton::Size::S24, this);
    m_zoomOut->setObjectName(QStringLiteral("graphZoomOut"));
    m_zoomOut->setCheckable(false);
    m_zoomOut->setFocusPolicy(Qt::NoFocus);
    m_zoomOut->setAccessibleName(QStringLiteral("Zoom out"));
    m_zoomOut->setTip(QStringLiteral("Zoom out"), QStringLiteral("-"));
    connect(m_zoomOut, &QAbstractButton::clicked, this, [this] { zoomBy(1 / 1.2, QPointF((width() - m_covered) / 2.0, height() / 2.0)); });
    m_zoomIn = new ui::IconButton(QStringLiteral("g-zin"), ui::IconButton::Size::S24, this);
    m_zoomIn->setObjectName(QStringLiteral("graphZoomIn"));
    m_zoomIn->setCheckable(false);
    m_zoomIn->setFocusPolicy(Qt::NoFocus);
    m_zoomIn->setAccessibleName(QStringLiteral("Zoom in"));
    m_zoomIn->setTip(QStringLiteral("Zoom in"), QStringLiteral("+"));
    connect(m_zoomIn, &QAbstractButton::clicked, this, [this] { zoomBy(1.2, QPointF((width() - m_covered) / 2.0, height() / 2.0)); });
    m_fit = new ui::GhostButton(QStringLiteral("Fit"), this);
    m_fit->setObjectName(QStringLiteral("graphFit"));
    m_fit->setFocusPolicy(Qt::NoFocus);
    m_fit->setTip(QStringLiteral("The whole graph in view"), QStringLiteral("F"));
    connect(m_fit, &QAbstractButton::clicked, this, [this] { fit(); });
    m_ticker = new QTimer(this);
    connect(m_ticker, &QTimer::timeout, this, [this] {
        // Only the takes card moves: its stripes, and its clock.
        update(cardRect(QStringLiteral("takes")).toAlignedRect().adjusted(-2, -2, 2, 2));
    });
}

void RecipeGraphView::setRecipe(const graph::Recipe &recipe) {
    QStringList before;
    for (const Node &node : m_recipe.nodes) before << node.id;
    QStringList now;
    for (const Node &node : recipe.nodes) now << node.id;
    // A recipe's next version (`bounce@2.kerfrecipe`, as an agent's edit leaves it) is the same
    // shot, by its name: the view stays where the person has it.
    const bool another = m_recipe.media != recipe.media || m_recipe.name != recipe.name || m_recipe.parts != recipe.parts;
    m_recipe = recipe;
    m_places = graph::arranged(m_recipe);
    for (auto placed = m_placed.constBegin(); placed != m_placed.constEnd(); ++placed)
        if (m_places.contains(placed.key())) m_places[placed.key()] = placed.value();
    if (!m_recipe.node(m_selected)) {
        m_selected.clear();
        emit selectionChanged(m_selected);
    }
    m_hovered.clear();
    setAccessibleName(m_recipe.name.isEmpty() ? QStringLiteral("Graph") : QStringLiteral("Graph of %1").arg(m_recipe.name));
    placeControls();
    // Another shot, or other nodes, is another picture to take in.
    if (another) {
        m_fitted = true;
        m_fitWhole = false;
    }
    if (m_fitted && (another || before != now)) settle();
    update();
}

void RecipeGraphView::setPlacementKey(const QString &key) {
    if (key == m_placementKey) return;
    m_placementKey = key;
    loadPlaced();
    m_places = graph::arranged(m_recipe);
    for (auto placed = m_placed.constBegin(); placed != m_placed.constEnd(); ++placed)
        if (m_places.contains(placed.key())) m_places[placed.key()] = placed.value();
    update();
}

void RecipeGraphView::loadPlaced() {
    m_placed.clear();
    if (m_placementKey.isEmpty()) return;
    const QVariantMap kept = QSettings().value(settingsKey(m_placementKey)).toMap();
    for (auto at = kept.constBegin(); at != kept.constEnd(); ++at) m_placed.insert(at.key(), at.value().toPointF());
}

void RecipeGraphView::savePlaced() const {
    if (m_placementKey.isEmpty()) return;
    QVariantMap kept;
    for (auto at = m_placed.constBegin(); at != m_placed.constEnd(); ++at) kept.insert(at.key(), at.value());
    QSettings settings;
    if (kept.isEmpty()) settings.remove(settingsKey(m_placementKey));
    else settings.setValue(settingsKey(m_placementKey), kept);
}

void RecipeGraphView::select(const QString &id) {
    const QString now = m_recipe.node(id) ? id : QString();
    if (now == m_selected) return;
    m_selected = now;
    update();
    emit selectionChanged(m_selected);
}

void RecipeGraphView::setMaking(bool making, int take, qint64 since) {
    if (making == m_making && take == m_makingTake && since == m_makingSince) return;
    m_making = making;
    m_makingTake = take;
    m_makingSince = since;
    followMaking();
    update();
}

void RecipeGraphView::followMaking() {
    // Still under Reduce Motion, where only the clock goes on; and nothing is
    // drawn again while it cannot be seen.
    if (m_making && isVisible()) m_ticker->start(ui::reducedMotion() ? motion::clockFrame : motion::stripeFrame);
    else m_ticker->stop();
}

void RecipeGraphView::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    followMaking();
    if (m_fitted) settle();
}

void RecipeGraphView::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    followMaking();
}

void RecipeGraphView::setFootRadius(qreal radius) {
    m_footRadius = radius;
    update();
}

void RecipeGraphView::setCovered(int right) {
    if (right == m_covered) return;
    m_covered = right;
    if (m_fitted) settle();
    update();
}

void RecipeGraphView::fit() {
    m_fitWhole = true;
    settle();
}

void RecipeGraphView::settle() {
    m_fitted = true;
    QRectF all;
    for (const Node &node : m_recipe.nodes) all |= sceneRect(node.id);
    if (all.isEmpty() || width() <= 0 || height() <= 0) {
        m_zoom = 1.0;
        m_offset = QPointF(width() / 2.0, height() / 2.0);
        update();
        return;
    }
    // Room round the cards, the legend under them, and what covers the right.
    const QRectF room = QRectF(rect()).adjusted(32, 24, -32 - m_covered, -(kLegendHeight + 2 * layout::graphPad + 8));
    const qreal zoom = std::min({1.0, room.width() / all.width(), room.height() / all.height()});
    m_zoom = std::clamp(zoom, kZoomLeast, kZoomMost);
    m_offset = room.center() - all.center() * m_zoom;
    if (!m_fitWhole && m_zoom < kZoomRead) {
        // Too wide or too tall to be read whole: where it starts, at a size that is.
        m_zoom = kZoomRead;
        const bool wide = all.width() * m_zoom > room.width();
        const bool tall = all.height() * m_zoom > room.height();
        m_offset = QPointF(wide ? room.left() - all.left() * m_zoom : room.center().x() - all.center().x() * m_zoom,
                           tall ? room.top() - all.top() * m_zoom : room.center().y() - all.center().y() * m_zoom);
    }
    update();
}

void RecipeGraphView::zoomBy(qreal factor, const QPointF &about) {
    const qreal zoom = std::clamp(m_zoom * factor, kZoomLeast, kZoomMost);
    if (qFuzzyCompare(zoom, m_zoom)) return;
    // What is under `about` stays under it.
    const QPointF scene = toScene(about);
    m_zoom = zoom;
    m_offset = about - scene * m_zoom;
    m_fitted = false;
    update();
}

QRectF RecipeGraphView::sceneRect(const QString &id) const {
    const Node *node = m_recipe.node(id);
    if (!node || !m_places.contains(id)) return {};
    return QRectF(m_places.value(id), QSizeF(graph::cardSize(*node).width(), graph::heightFor(m_recipe, *node)));
}

QRectF RecipeGraphView::cardRect(const QString &id) const {
    const QRectF scene = sceneRect(id);
    return scene.isNull() ? QRectF() : QRectF(toView(scene.topLeft()), scene.size() * m_zoom);
}

QPointF RecipeGraphView::outPort(const QString &id) const {
    const QRectF card = sceneRect(id);
    // A workflow's node gives from beside its name, over its rows.
    if (const Node *node = m_recipe.node(id); node && node->kind == Kind::Part)
        return QPointF(card.right(), card.top() + layout::graphPartHead / 2.0 + 2);
    return QPointF(card.right(), card.center().y());
}

QPointF RecipeGraphView::inPort(const Edge &edge) const {
    const QRectF card = sceneRect(edge.to);
    if (const Node *to = m_recipe.node(edge.to); to && to->kind == Kind::Part) {
        // At the row of the input it is.
        const QJsonArray inputs = to->holds.value(QStringLiteral("inputs")).toArray();
        int row = 0;
        for (int at = 0; at < inputs.size(); ++at)
            if (inputs.at(at).toObject().value(QStringLiteral("name")).toString() == edge.port) row = at;
        return QPointF(card.left(), card.top() + layout::graphPartHead + layout::graphPartRow * (row + 0.5));
    }
    const QList<Edge> into = m_recipe.into(edge.to);
    int index = 0;
    for (int at = 0; at < into.size(); ++at)
        if (into.at(at).from == edge.from && into.at(at).port == edge.port && into.at(at).nth == edge.nth) index = at;
    return QPointF(card.left(), card.top() + card.height() * (index + 0.5) / std::max<qsizetype>(1, into.size()));
}

QPainterPath RecipeGraphView::scenePath(const Edge &edge) const {
    const QPointF from = outPort(edge.from);
    const QPointF to = inPort(edge);
    // It leaves and arrives level, and turns in the room between the columns.
    const qreal reach = std::max(48.0, std::abs(to.x() - from.x()) * 0.6);
    QPainterPath path(from);
    path.cubicTo(from + QPointF(reach, 0), to - QPointF(reach, 0), to);
    return path;
}

QPainterPath RecipeGraphView::edgePath(int index) const {
    if (index < 0 || index >= m_recipe.edges.size()) return {};
    QTransform view;
    view.translate(m_offset.x(), m_offset.y());
    view.scale(m_zoom, m_zoom);
    return view.map(scenePath(m_recipe.edges.at(index)));
}

QList<Gives> RecipeGraphView::legend() const {
    // The four a shot is told by, always; words and other files where the graph has them.
    QList<Gives> shown{Gives::Video, Gives::Image, Gives::Path, Gives::Signal};
    for (const Gives more : {Gives::Words, Gives::File})
        if (std::any_of(m_recipe.edges.cbegin(), m_recipe.edges.cend(), [more](const Edge &edge) { return edge.gives == more; })) shown << more;
    return shown;
}

QRectF RecipeGraphView::legendRect() const {
    const QFontMetricsF metrics(tokens::font(meta));
    qreal width = 12;
    if (m_recipe.parts) {
        // A link, a blank and a value, as a workflow's rows show them.
        const QFontMetricsF mono(tokens::font(monoSmall));
        width += kLegendSample + 6 + metrics.horizontalAdvance(kLegendLink) + 14;
        width += mono.horizontalAdvance(kLegendBlank) + 10 + 6 + metrics.horizontalAdvance(kLegendBlankSays) + 14;
        width += mono.horizontalAdvance(kLegendValue) + 6 + metrics.horizontalAdvance(kLegendValueSays) + 14;
        return QRectF(layout::graphPad, height() - layout::graphPad - kLegendHeight, std::ceil(width - 2), kLegendHeight);
    }
    for (const Gives gives : legend()) width += kLegendSample + 6 + metrics.horizontalAdvance(graph::nameOf(gives)) + 14;
    return QRectF(layout::graphPad, height() - layout::graphPad - kLegendHeight, std::ceil(width - 2), kLegendHeight);
}

QRectF RecipeGraphView::zoomRect() const {
    const QRectF legend = legendRect();
    const qreal width = 4 + 24 + 24 + m_fit->sizeHint().width() + 4;
    return QRectF(legend.right() + 8, legend.top(), width, legend.height());
}

void RecipeGraphView::placeControls() {
    // With no cards there is nothing to zoom, and no legend to stand beside.
    for (QWidget *control : {static_cast<QWidget *>(m_zoomOut), static_cast<QWidget *>(m_zoomIn), static_cast<QWidget *>(m_fit)})
        control->setVisible(!m_recipe.nodes.isEmpty());
    const QRectF card = zoomRect();
    const int top = int(card.top()) + (kLegendHeight - 24) / 2;
    m_zoomOut->move(int(card.left()) + 4, top);
    m_zoomIn->move(m_zoomOut->x() + 24, top);
    m_fit->setGeometry(m_zoomIn->x() + 24, top, m_fit->sizeHint().width(), 24);
}

QString RecipeGraphView::portAt(const QPointF &at) const {
    for (auto node = m_recipe.nodes.crbegin(); node != m_recipe.nodes.crend(); ++node) {
        // The takes give nothing on: they are where the graph ends. A
        // workflow's links are its own shape, and are not drawn here.
        if (node->kind == Kind::Takes || node->kind == Kind::Part) continue;
        const QPointF port = toView(outPort(node->id));
        if (std::hypot(port.x() - at.x(), port.y() - at.y()) <= kPortReach) return node->id;
    }
    return {};
}

QString RecipeGraphView::nodeAt(const QPointF &at) const {
    for (auto node = m_recipe.nodes.crbegin(); node != m_recipe.nodes.crend(); ++node)
        if (cardRect(node->id).contains(at)) return node->id;
    return {};
}

void RecipeGraphView::paintPart(QPainter &p, const Node &node) const {
    const QRectF card = sceneRect(node.id);
    const qreal radius = radius::card;
    const bool selected = node.id == m_selected;
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::nodes::node));
    p.drawRoundedRect(card, radius, radius);
    ui::paintInsetLine(p, card, radius, color(node.id == m_hovered && !selected ? lineh : line2));
    if (selected) ui::paintFocusRing(p, card, radius);
    // Its name, and its number in the graph at the end.
    const QFont tagFont = tokens::font(monoSmall);
    const QString number = QStringLiteral("#%1").arg(node.holds.value(QStringLiteral("id")).toString());
    const qreal numberWidth = std::ceil(QFontMetricsF(tagFont).horizontalAdvance(number));
    p.setFont(tagFont);
    p.setPen(color(ink3));
    p.drawText(QRectF(card.right() - 10 - numberWidth, card.top() + 4, numberWidth, layout::graphPartHead - 4), Qt::AlignRight | Qt::AlignVCenter, number);
    const QFont titleFont = tokens::font(controlStrong);
    const qreal titleWidth = card.width() - 20 - numberWidth - 8;
    p.setFont(titleFont);
    p.setPen(color(ink));
    p.drawText(QRectF(card.left() + 10, card.top() + 4, titleWidth, layout::graphPartHead - 4), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(titleFont).elidedText(node.title, Qt::ElideRight, titleWidth));
    p.fillRect(QRectF(card.left() + 1, card.top() + layout::graphPartHead, card.width() - 2, 1), color(line));
    // A row an input: its name, and what it is. A link's comes in at the row;
    // a blank is the recipe's, in the accent's ink on a wash of it; a value is
    // the workflow's own, plain.
    const QFont nameFont = tokens::font(meta);
    const QFont valueFont = tokens::font(monoSmall);
    const QFontMetricsF nameMetrics(nameFont);
    const QFontMetricsF valueMetrics(valueFont);
    const QJsonArray inputs = node.holds.value(QStringLiteral("inputs")).toArray();
    for (int at = 0; at < inputs.size(); ++at) {
        const QJsonObject input = inputs.at(at).toObject();
        const QRectF row(card.left() + 10, card.top() + layout::graphPartHead + 2 + layout::graphPartRow * at, card.width() - 20, layout::graphPartRow);
        const QString name = input.value(QStringLiteral("name")).toString();
        const graph::InputKind kind = graph::inputKind(input);
        p.setFont(nameFont);
        p.setPen(color(kind == graph::InputKind::Link ? ink2 : ink3));
        const qreal nameWidth = std::min(row.width() * 0.5, std::ceil(nameMetrics.horizontalAdvance(name)));
        p.drawText(QRectF(row.left(), row.top(), nameWidth, row.height()), Qt::AlignLeft | Qt::AlignVCenter,
                   nameMetrics.elidedText(name, Qt::ElideRight, nameWidth));
        if (kind == graph::InputKind::Link) continue;
        const qreal room = row.width() - nameWidth - 10;
        const QString said = valueMetrics.elidedText(graph::valueText(input.value(QStringLiteral("value"))), Qt::ElideRight,
                                                     kind == graph::InputKind::Blank ? room - 10 : room);
        const qreal width = std::ceil(valueMetrics.horizontalAdvance(said));
        p.setFont(valueFont);
        if (kind == graph::InputKind::Blank) {
            const QRectF pill(row.right() - width - 10, row.top() + 2, width + 10, row.height() - 4);
            p.setPen(Qt::NoPen);
            p.setBrush(color(accentWash));
            p.drawRoundedRect(pill, 4, 4);
            p.setPen(color(accentInk));
            p.drawText(pill, Qt::AlignCenter, said);
        } else {
            p.setPen(color(tokens::ink2));
            p.drawText(QRectF(row.right() - width, row.top(), width, row.height()), Qt::AlignRight | Qt::AlignVCenter, said);
        }
    }
}

void RecipeGraphView::paintCard(QPainter &p, const Node &node) const {
    if (node.kind == Kind::Part) {
        paintPart(p, node);
        return;
    }
    const QRectF card = sceneRect(node.id);
    const qreal radius = radius::panel;
    const QColor type = graph::colorOf(node.gives);
    const bool selected = node.id == m_selected;
    const bool making = m_making && node.kind == Kind::Takes;

    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::nodes::node));
    p.drawRoundedRect(card, radius, radius);
    if (making) {
        // What is being made has the stripes of a making, here as on its clips.
        QPainterPath clip;
        clip.addRoundedRect(card, radius, radius);
        p.save();
        p.setClipPath(clip, Qt::IntersectClip);
        kerf::activity::paintStripes(p, card, kerf::activity::stripePhase(kerf::activity::clock()), color(tokens::activity::gpuStripe));
        p.restore();
    }
    // While an edge is drawn, the card under it that could read it says so.
    const bool takes = m_drag == Drag::Wire && node.id == m_wireOver;
    ui::paintInsetLine(p, card, radius, color(takes ? accentInk : node.id == m_hovered && !selected ? lineh : line2), takes ? 1.5 : 1);
    if (selected) ui::paintFocusRing(p, card, radius);

    // The icon, on a wash of what the node gives.
    const QRectF tile(card.left() + 12, card.top() + 12, 28, 28);
    p.setPen(Qt::NoPen);
    p.setBrush(withAlpha(type, 0.16));
    p.drawRoundedRect(tile, radius::card, radius::card);
    icons::paint(p, graph::glyphOf(node), tile, type, 16);

    // The tag: what it gives, at the card's top right.
    const QFont tagFont = tokens::font(monoSmall);
    const QFontMetricsF tagMetrics(tagFont);
    const QString tag = graph::nameOf(node.gives);
    const qreal tagWidth = std::ceil(tagMetrics.horizontalAdvance(tag)) + 12;
    const QRectF tagRect(card.right() - 10 - tagWidth, card.top() + 11, tagWidth, 16);
    p.setBrush(withAlpha(type, 0.14));
    p.drawRoundedRect(tagRect, 8, 8);
    p.setFont(tagFont);
    p.setPen(type);
    p.drawText(tagRect, Qt::AlignCenter, tag);
    // What is wrong with it, said by a dot before the tag: not there in
    // brick, wanting a look in ochre.
    qreal titleRight = tagRect.left() - 8;
    const bool wants = node.kind == Kind::Takes && !node.holds.value(QStringLiteral("wants")).isNull();
    if (graph::isMissing(node) || wants) {
        const bool gone = graph::isMissing(node) && node.kind != Kind::Model;
        p.setPen(Qt::NoPen);
        p.setBrush(color(gone ? status::failed : status::request));
        p.drawEllipse(QPointF(titleRight - 3, tagRect.center().y()), 3, 3);
        titleRight -= 14;
    }

    const qreal textLeft = tile.right() + 10;
    const QFont titleFont = tokens::font(bodyName);
    p.setFont(titleFont);
    p.setPen(color(ink));
    p.drawText(QRectF(textLeft, card.top() + 9, titleRight - textLeft, 20), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(titleFont).elidedText(node.title, Qt::ElideRight, titleRight - textLeft));
    const QFont detailFont = tokens::font(meta);
    QString detail = node.detail.simplified();
    if (making) {
        detail = QStringLiteral("Making take %1 · %2").arg(m_makingTake).arg(kerf::activity::elapsedText(kerf::activity::clock() - m_makingSince));
    } else if (node.kind == Kind::Model && detail.isEmpty()) {
        detail = QStringLiteral("no prompt");
    }
    const qreal detailWidth = card.right() - 12 - textLeft;
    p.setFont(detailFont);
    // Over the stripes it is said in ink: amber on amber is not read.
    p.setPen(color(making ? ink : ink3));
    p.drawText(QRectF(textLeft, card.top() + 29, detailWidth, 16), Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(detailFont).elidedText(detail, Qt::ElideRight, detailWidth));
    qreal under = card.top() + 47;
    if (node.kind == Kind::Model) {
        const QFont third = tokens::font(monoSmall);
        p.setFont(third);
        p.setPen(color(ink3));
        const QString line = modelLine(node);
        p.drawText(QRectF(textLeft, under, detailWidth, 16), Qt::AlignLeft | Qt::AlignVCenter,
                   QFontMetricsF(third).elidedText(line.isEmpty() ? QStringLiteral("as the model comes") : line, Qt::ElideRight, detailWidth));
        under += 18;
    }
    if (!graph::drawsCurve(node)) return;
    // Under the lines: a signal's number over its clip, 0 at the foot and 1
    // at the top, or the path a clip is led along, from its ring to its dot.
    const QRectF plot(card.left() + 12, under + 3, card.width() - 24, layout::graphCardCurve - 12);
    p.setBrush(withAlpha(color(surroundDim), 0.6));
    p.setPen(Qt::NoPen);
    p.drawRoundedRect(plot.adjusted(-4, -3, 4, 3), radius::small, radius::small);
    if (node.kind == Kind::Signal) {
        const QJsonArray values = node.holds.value(QStringLiteral("values")).toArray();
        QPolygonF curve;
        for (int index = 0; index < values.size(); ++index)
            curve << QPointF(plot.left() + plot.width() * index / std::max<qsizetype>(1, values.size() - 1),
                             plot.bottom() - plot.height() * std::clamp(values.at(index).toDouble(), 0.0, 1.0));
        if (curve.size() == 1) curve << QPointF(plot.right(), curve.first().y());
        QPolygonF below = curve;
        below << QPointF(plot.right(), plot.bottom()) << QPointF(plot.left(), plot.bottom());
        p.setBrush(withAlpha(type, 0.16));
        p.drawPolygon(below);
        p.setBrush(Qt::NoBrush);
        p.setPen(QPen(type, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p.drawPolyline(curve);
        return;
    }
    const double aspect = std::max(0.1, node.holds.value(QStringLiteral("aspect")).toDouble(16.0 / 9.0));
    const auto points = motionpath::parsePath(node.holds.value(QStringLiteral("path")).toString());
    if (!points || points->isEmpty()) return;
    QList<QPointF> line = points->size() >= 2 ? motionpath::MotionPath(*points, aspect).samples() : *points;
    // In frame widths across and frame heights down, as the clip's x and y are.
    QRectF bounds;
    for (const QPointF &point : std::as_const(line)) bounds |= QRectF(QPointF(point.x() * aspect, point.y()), QSizeF(1e-6, 1e-6));
    const qreal fit = std::min(plot.width() / std::max(bounds.width(), 1e-3), plot.height() / std::max(bounds.height(), 1e-3));
    QPolygonF drawn;
    for (const QPointF &point : std::as_const(line))
        drawn << plot.center() + (QPointF(point.x() * aspect, point.y()) - bounds.center()) * fit;
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(type, 1.25, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    p.drawPolyline(drawn);
    p.setBrush(color(tokens::nodes::node));
    p.drawEllipse(drawn.first(), 2.5, 2.5);
    p.setBrush(type);
    p.drawEllipse(drawn.last(), 2.5, 2.5);
}

void RecipeGraphView::paintLegend(QPainter &p) const {
    const QRectF card = legendRect();
    p.setPen(Qt::NoPen);
    p.setBrush(color(popup));
    p.drawRoundedRect(card, radius::card, radius::card);
    ui::paintInsetLine(p, card, radius::card, color(line2));
    // The zoom's own card beside it, under its buttons.
    const QRectF zoom = zoomRect();
    p.setBrush(color(popup));
    p.drawRoundedRect(zoom, radius::card, radius::card);
    ui::paintInsetLine(p, zoom, radius::card, color(line2));
    const QFont font = tokens::font(meta);
    const QFontMetricsF metrics(font);
    p.setFont(font);
    qreal x = card.left() + 10;
    if (m_recipe.parts) {
        const QFont mono = tokens::font(monoSmall);
        const QFontMetricsF monoMetrics(mono);
        const auto says = [&](const QString &text) {
            p.setFont(font);
            p.setPen(color(ink2));
            const qreal width = metrics.horizontalAdvance(text);
            p.drawText(QRectF(x, card.top(), width + 2, card.height()), Qt::AlignLeft | Qt::AlignVCenter, text);
            x += width + 14;
        };
        p.setPen(QPen(graph::colorOf(Gives::File), 1.5, Qt::SolidLine, Qt::RoundCap));
        p.drawLine(QPointF(x, card.center().y()), QPointF(x + kLegendSample, card.center().y()));
        x += kLegendSample + 6;
        says(kLegendLink);
        const qreal blank = monoMetrics.horizontalAdvance(kLegendBlank) + 10;
        const QRectF pill(x, card.center().y() - 8, blank, 16);
        p.setPen(Qt::NoPen);
        p.setBrush(color(accentWash));
        p.drawRoundedRect(pill, 4, 4);
        p.setFont(mono);
        p.setPen(color(accentInk));
        p.drawText(pill, Qt::AlignCenter, kLegendBlank);
        x += blank + 6;
        says(kLegendBlankSays);
        p.setFont(mono);
        p.setPen(color(ink2));
        const qreal value = monoMetrics.horizontalAdvance(kLegendValue);
        p.drawText(QRectF(x, card.top(), value + 2, card.height()), Qt::AlignLeft | Qt::AlignVCenter, kLegendValue);
        x += value + 6;
        says(kLegendValueSays);
        return;
    }
    for (const Gives gives : legend()) {
        const QString name = graph::nameOf(gives);
        p.setPen(QPen(graph::colorOf(gives), 1.5, Qt::SolidLine, Qt::RoundCap));
        p.drawLine(QPointF(x, card.center().y()), QPointF(x + kLegendSample, card.center().y()));
        x += kLegendSample + 6;
        p.setPen(color(ink2));
        const qreal width = metrics.horizontalAdvance(name);
        p.drawText(QRectF(x, card.top(), width + 2, card.height()), Qt::AlignLeft | Qt::AlignVCenter, name);
        x += width + 14;
    }
}

void RecipeGraphView::paintEvent(QPaintEvent *) {
    QPainter p(this);
    if (m_footRadius > 0) {
        // Square at the top, where the panel's head is; round at the foot, as the panel is.
        const QRectF all = QRectF(rect());
        QPainterPath foot;
        foot.moveTo(all.topLeft());
        foot.lineTo(all.topRight());
        foot.lineTo(all.right(), all.bottom() - m_footRadius);
        foot.quadTo(all.bottomRight(), QPointF(all.right() - m_footRadius, all.bottom()));
        foot.lineTo(all.left() + m_footRadius, all.bottom());
        foot.quadTo(all.bottomLeft(), QPointF(all.left(), all.bottom() - m_footRadius));
        foot.closeSubpath();
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setClipPath(foot);
        p.setRenderHint(QPainter::Antialiasing, false);
    }
    p.fillRect(rect(), color(tokens::nodes::canvas));
    // The canvas's grid: a dot a step, while they are far enough apart to be dots.
    const qreal step = layout::graphGrid * m_zoom;
    if (step >= 10) {
        QList<QPointF> dots;
        for (qreal y = std::fmod(m_offset.y(), step); y < height(); y += step)
            for (qreal x = std::fmod(m_offset.x(), step); x < width(); x += step) dots << QPointF(std::round(x), std::round(y));
        p.setPen(QPen(color(tokens::nodes::dots), 1.5, Qt::SolidLine, Qt::RoundCap));
        p.drawPoints(dots.constData(), int(dots.size()));
    }
    if (m_recipe.nodes.isEmpty()) {
        // Nothing to lay out: say why, where the cards would be.
        const QString why = !m_recipe.error.isEmpty() ? QStringLiteral("This recipe does not read: %1").arg(m_recipe.error)
                            : !m_recipe.isValid() ? QString()
                            : m_recipe.said ? QStringLiteral("This recipe has no steps") : QStringLiteral("Reading %1…").arg(m_recipe.recipe);
        p.setFont(tokens::font(body));
        p.setPen(color(m_recipe.error.isEmpty() ? ink3 : status::request));
        p.drawText(QRectF(rect()).adjusted(48, 48, -48 - m_covered, -48), Qt::AlignCenter | Qt::TextWordWrap, why);
        return;
    }
    p.setRenderHint(QPainter::Antialiasing, true);
    p.save();
    p.translate(m_offset);
    p.scale(m_zoom, m_zoom);
    // Edges under the cards. With a node chosen, what does not touch it falls back.
    for (const Edge &edge : std::as_const(m_recipe.edges)) {
        const bool touches = m_selected.isEmpty() || edge.from == m_selected || edge.to == m_selected;
        QColor line = graph::colorOf(edge.gives);
        if (!touches) line.setAlphaF(0.35);
        QPen pen(line, 1.5, Qt::SolidLine, Qt::RoundCap);
        // What a model has said it has no use for goes to it dashed.
        if (const Node *to = m_recipe.node(edge.to); to && to->kind == Kind::Model && to->holds.contains(QStringLiteral("reads"))) {
            bool reads = edge.port == QLatin1String("cast") || edge.port == QLatin1String("style");
            for (const QJsonValue read : to->holds.value(QStringLiteral("reads")).toArray()) reads = reads || read.toString() == edge.port;
            if (!reads) pen.setDashPattern({3, 3});
        }
        p.setPen(pen);
        p.setBrush(Qt::NoBrush);
        p.drawPath(scenePath(edge));
    }
    for (const Node &node : std::as_const(m_recipe.nodes)) paintCard(p, node);
    // Where each edge leaves and arrives, over the cards' edges; and by what
    // name the step reads it, beside where it comes in.
    const QFont portFont = tokens::font(meta);
    const QFontMetricsF portMetrics(portFont);
    for (const Edge &edge : std::as_const(m_recipe.edges)) {
        const QColor type = graph::colorOf(edge.gives);
        const QPointF from = outPort(edge.from);
        const QPointF to = inPort(edge);
        p.setPen(QPen(type, 1.5));
        p.setBrush(color(tokens::nodes::node));
        p.drawEllipse(from, 3.5, 3.5);
        p.setPen(Qt::NoPen);
        p.setBrush(type);
        p.drawEllipse(to, 3.5, 3.5);
        // A guide's sequence and the shot's take are the one thing their node
        // reads; a model's inputs are told apart by name. Too small to read, none is.
        const Node *reader = m_recipe.node(edge.to);
        if (m_zoom < 0.7 || !reader || (reader->kind != Kind::Model && reader->kind != Kind::Run && reader->kind != Kind::Join)) continue;
        const bool touches = m_selected.isEmpty() || edge.from == m_selected || edge.to == m_selected;
        const qreal width = std::ceil(portMetrics.horizontalAdvance(edge.port));
        // On the canvas's own colour, so an edge that passes behind it does not strike it through.
        const QRectF label(to.x() - 9 - width - 6, to.y() - 19, width + 8, 14);
        p.setPen(Qt::NoPen);
        p.setBrush(withAlpha(color(tokens::nodes::canvas), 0.86));
        p.drawRoundedRect(label, 4, 4);
        p.setFont(portFont);
        p.setPen(color(touches ? ink3 : dis));
        p.drawText(label, Qt::AlignCenter, edge.port);
    }
    // The port under the pointer, a little larger: an edge is drawn from it.
    const QString from = m_drag == Drag::Wire ? m_wireFrom : m_hoveredPort;
    if (const Node *node = m_recipe.node(from)) {
        const QColor type = graph::colorOf(node->gives);
        p.setPen(QPen(type, 1.5));
        p.setBrush(m_drag == Drag::Wire ? type : color(tokens::nodes::node));
        p.drawEllipse(outPort(from), 5, 5);
    }
    p.restore();
    if (m_drag == Drag::Wire) {
        // The edge being drawn, from its port to the pointer, in what it would carry.
        if (const Node *node = m_recipe.node(m_wireFrom)) {
            const QPointF start = toView(outPort(m_wireFrom));
            const qreal reach = std::max(32.0, std::abs(m_wireAt.x() - start.x()) * 0.5);
            QPainterPath path(start);
            path.cubicTo(start + QPointF(reach, 0), m_wireAt - QPointF(reach, 0), m_wireAt);
            QPen pen(graph::colorOf(node->gives), 1.5, Qt::SolidLine, Qt::RoundCap);
            // Until it is over something that could read it, it is only being held.
            if (m_wireOver.isEmpty()) pen.setDashPattern({3, 3});
            p.setPen(pen);
            p.setBrush(Qt::NoBrush);
            p.drawPath(path);
        }
    }
    paintLegend(p);
}

void RecipeGraphView::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    placeControls();
    if (m_fitted) settle();
}

void RecipeGraphView::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton && event->button() != Qt::MiddleButton) return;
    setFocus(Qt::MouseFocusReason);
    m_pressView = event->position();
    m_pressOffset = m_offset;
    m_dragMoved = false;
    // A press on a port draws an edge from it.
    if (const QString port = event->button() == Qt::LeftButton ? portAt(event->position()) : QString(); !port.isEmpty()) {
        m_drag = Drag::Wire;
        m_wireFrom = port;
        m_wireAt = event->position();
        m_wireOver.clear();
        update();
        return;
    }
    const QString hit = event->button() == Qt::LeftButton ? nodeAt(event->position()) : QString();
    if (hit.isEmpty()) {
        m_drag = Drag::Pan;
        return;
    }
    m_drag = Drag::Card;
    m_pressPlace = m_places.value(hit);
    select(hit);
}

void RecipeGraphView::mouseMoveEvent(QMouseEvent *event) {
    const QPointF moved = event->position() - m_pressView;
    if (m_drag == Drag::None) {
        const QString port = portAt(event->position());
        const QString under = port.isEmpty() ? nodeAt(event->position()) : QString();
        if (under != m_hovered || port != m_hoveredPort) {
            m_hovered = under;
            m_hoveredPort = port;
            setCursor(port.isEmpty() ? Qt::ArrowCursor : Qt::CrossCursor);
            update();
        }
        return;
    }
    if (m_drag == Drag::Wire) {
        m_wireAt = event->position();
        // The card under the pointer, if it could read what is being held.
        const graph::Node *from = m_recipe.node(m_wireFrom);
        const graph::Node *over = m_recipe.node(nodeAt(event->position()));
        m_wireOver = from && over && !graph::portsFor(m_recipe, *from, *over).isEmpty() ? over->id : QString();
        update();
        return;
    }
    if (!m_dragMoved && std::hypot(moved.x(), moved.y()) < kDragFrom) return;
    m_dragMoved = true;
    if (m_drag == Drag::Pan) {
        setCursor(Qt::ClosedHandCursor);
        m_offset = m_pressOffset + moved;
        m_fitted = false;
    } else if (!m_selected.isEmpty()) {
        // A card goes where it is dragged, on whole points.
        const QPointF place = m_pressPlace + moved / m_zoom;
        m_places[m_selected] = QPointF(std::round(place.x()), std::round(place.y()));
    }
    update();
}

void RecipeGraphView::mouseReleaseEvent(QMouseEvent *event) {
    const Drag drag = std::exchange(m_drag, Drag::None);
    unsetCursor();
    if (drag == Drag::Wire) {
        const QString from = std::exchange(m_wireFrom, QString());
        const QString to = std::exchange(m_wireOver, QString());
        m_hoveredPort.clear();
        update();
        if (!to.isEmpty()) emit wired(from, to, mapToGlobal(event->position().toPoint()));
        return;
    }
    if (drag == Drag::Card && m_dragMoved && !m_selected.isEmpty()) {
        m_placed.insert(m_selected, m_places.value(m_selected));
        savePlaced();
    } else if (drag == Drag::Pan && !m_dragMoved && event->button() == Qt::LeftButton) {
        // A click on the canvas lets go of the chosen node.
        select(QString());
    }
    m_dragMoved = false;
}

void RecipeGraphView::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return;
    const QString hit = nodeAt(event->position());
    if (hit.isEmpty()) fit();
    else emit nodeActivated(hit);
}

void RecipeGraphView::wheelEvent(QWheelEvent *event) {
    // Two fingers on a trackpad move the canvas; a wheel, or either with ⌘,
    // zooms about the pointer.
    const bool trackpad = !event->pixelDelta().isNull() || event->phase() != Qt::NoScrollPhase;
    if (trackpad && !(event->modifiers() & Qt::ControlModifier)) {
        const QPointF by = event->pixelDelta().isNull() ? QPointF(event->angleDelta()) / 4.0 : QPointF(event->pixelDelta());
        m_offset += by;
        m_fitted = false;
        update();
    } else {
        zoomBy(std::pow(1.0015, event->angleDelta().y()), event->position());
    }
    event->accept();
}

bool RecipeGraphView::event(QEvent *event) {
    if (event->type() == QEvent::NativeGesture) {
        auto *gesture = static_cast<QNativeGestureEvent *>(event);
        if (gesture->gestureType() == Qt::ZoomNativeGesture) {
            zoomBy(1.0 + gesture->value(), gesture->position());
            return true;
        }
    }
    return QWidget::event(event);
}

void RecipeGraphView::keyPressEvent(QKeyEvent *event) {
    const QPointF middle(width() / 2.0, height() / 2.0);
    switch (event->key()) {
    case Qt::Key_0:
    case Qt::Key_F: fit(); return;
    case Qt::Key_Plus:
    case Qt::Key_Equal: zoomBy(1.2, middle); return;
    case Qt::Key_Minus: zoomBy(1 / 1.2, middle); return;
    case Qt::Key_Left:
    case Qt::Key_Right:
    case Qt::Key_Up:
    case Qt::Key_Down: {
        // The arrows go to the nearest card that way.
        const QRectF from = sceneRect(m_selected);
        if (from.isNull()) {
            if (!m_recipe.nodes.isEmpty()) select(m_recipe.nodes.first().id);
            return;
        }
        const bool across = event->key() == Qt::Key_Left || event->key() == Qt::Key_Right;
        const int sign = event->key() == Qt::Key_Left || event->key() == Qt::Key_Up ? -1 : 1;
        QString best;
        qreal nearest = 0;
        for (const Node &node : std::as_const(m_recipe.nodes)) {
            const QPointF by = sceneRect(node.id).center() - from.center();
            const qreal along = (across ? by.x() : by.y()) * sign;
            const qreal aside = std::abs(across ? by.y() : by.x());
            if (node.id == m_selected || along <= 1 || aside > along * 2 + 40) continue;
            const qreal far = along + aside * 2;
            if (best.isEmpty() || far < nearest) {
                best = node.id;
                nearest = far;
            }
        }
        if (!best.isEmpty()) select(best);
        return;
    }
    default: QWidget::keyPressEvent(event);
    }
}

void RecipeGraphView::leaveEvent(QEvent *event) {
    QWidget::leaveEvent(event);
    if (!m_hovered.isEmpty()) {
        m_hovered.clear();
        update();
    }
}

} // namespace kerf
