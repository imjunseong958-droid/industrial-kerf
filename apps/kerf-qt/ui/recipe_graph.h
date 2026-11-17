#pragma once

// A shot's recipe as a graph: how the shot is made, laid out to be seen and
// set. The bridge reads the recipe and says what it is (`recipe`, from
// kerf_engine::recipe::graph): a node for each step, a `render` that draws a
// guide from a sequence or a `generate` that asks a model, and for each thing
// of the project a step reads, a sequence, a picture, a signal clip, the way a
// clip goes, one of the cast; and an edge for each reading, with what goes
// along it. The last step's file is the shot, so the graph ends in its takes.
//
//   court ──▶ Depth guide ──────────▶ guide ┐
//             still ──────────────────▶ ref ┤
//   ball (where it goes) ───────────▶ moves ├─ video ──▶ bounce: take 2 of 3
//   bounce (a signal) ─────────────▶ signal ┘
//
// The canvas is free-form: a card is where it was last dragged to, kept with
// the editor's settings on this computer and never in the recipe, which
// collaborators share. Until one is moved, the cards stand in columns by how
// far each is from what it reads, the takes at the right.
//
// What it looks like (the agreed look): a dark canvas with a dot a step;
// rounded cards with an icon and a title, a quiet second line and a small tag
// that says what the node gives; curved edges in the colour of what goes
// along them (tokens::nodes: a picture in motion dusty blue, a still sage,
// the way a thing goes violet, a number over time ochre); and a legend.
//
// This file is what the graph is and the canvas that draws it. What the
// chosen node holds is set in RecipeInspector (recipe_inspector.h), and the
// panel the two open in is RecipeGraphPanel (recipe_graph_panel.h).

#include "design_tokens.h"
#include "test_friends.h"

#include <QHash>
#include <QJsonObject>
#include <QList>
#include <QPainterPath>
#include <QPointF>
#include <QRectF>
#include <QSizeF>
#include <QString>
#include <QStringList>
#include <QWidget>

class QTimer;

namespace kerf {

namespace ui {
class GhostButton;
class IconButton;
} // namespace ui

namespace graph {

// What goes along an edge, and what a node gives.
enum class Gives { Video, Image, Signal, Path, Words, File };
// What a node is.
enum class Kind { Sequence, Render, Model, Run, File, Signal, Moves, Cast, Style, Takes };

struct Node {
    // What it is told apart by: a step by the file it makes (`made:guide.mp4`),
    // what is read by its name (`signal:bounce`), the shot's takes `takes`.
    QString id;
    Kind kind = Kind::File;
    QString title;
    QString detail;
    Gives gives = Gives::File;
    // The recipe's line it is, from 1; 0 for what is not a step.
    int line = 0;
    // Everything the bridge said of it, by name.
    QJsonObject holds;
    bool isStep() const { return line > 0; }
};

struct Edge {
    QString from;
    QString to;
    // The name the reading step gives it (`guide`, `ref`, `signal`), and
    // which of the line's words of that name it is.
    QString port;
    int nth = 0;
    Gives gives = Gives::File;
};

// A media's recipe, as a `recipe` event says it.
struct Recipe {
    QString media;
    // The recipe as the document names it, and the shot's name.
    QString recipe;
    QString name;
    // Its text, as its file holds it: what a change is made to.
    QString text;
    QList<Node> nodes;
    QList<Edge> edges;
    // Why the text is no recipe; why it cannot be made as things stand; why
    // the person's models file does not read. Each empty where there is none.
    QString error;
    QString cannot;
    QString modelsError;
    // What is worth knowing before it is made.
    QStringList notes;
    // Whether the bridge has said it: until then it is the shot's name alone.
    bool said = true;

    bool isValid() const { return !media.isEmpty(); }
    const Node *node(const QString &id) const;
    // The edges into `id`, in the order the step reads them.
    QList<Edge> into(const QString &id) const;
    static Recipe read(const QJsonObject &event);
};

// The colour of what goes along an edge, and its name as a tag and the
// legend say it.
QColor colorOf(Gives gives);
QString nameOf(Gives gives);
// The icon a node's card leads with.
QString glyphOf(const Node &node);
// How large a node's card is: taller where it draws a signal's curve or a path.
QSizeF cardSize(const Node &node);
// Whether something is wrong with what the node stands for, which its card
// marks: a file or a clip that is not there, a model that is not set up.
bool isMissing(const Node &node);
// The names under which `to` could read what `from` gives, in the order to
// offer them: a signal as `signal`, the way a clip goes as `moves`, a picture
// in motion as a `guide` or a `ref`, a still as `first`, `last` or `ref`, a
// sequence as what a guide is drawn from. None where it cannot: what is not
// a step reads nothing, and a step reads only what a step before it made.
QStringList portsFor(const Recipe &recipe, const Node &from, const Node &to);
// What a line writes to read `from`: a file a step made in braces, a file of
// the project by its path, a clip or one of the cast by its name.
QString valueOf(const Node &from);
// What a port's name stands for, in a few words.
QString portSays(const QString &port);
// Where each card's top left goes while none has been moved by hand: columns
// by how far a node is from what it reads, what is read beside what reads it,
// the takes at the right, each column centred on the same line.
QHash<QString, QPointF> arranged(const Recipe &recipe);

} // namespace graph

// The canvas: the recipe's cards and edges, panned, zoomed, and moved about.
class RecipeGraphView final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit RecipeGraphView(QWidget *parent = nullptr);

    void setRecipe(const graph::Recipe &recipe);
    const graph::Recipe &recipe() const { return m_recipe; }
    // What the cards' places are kept under: the project and the recipe, so
    // each shot keeps its own arrangement. Empty keeps none.
    void setPlacementKey(const QString &key);

    QString selected() const { return m_selected; }
    void select(const QString &id);

    // The shot is being made: its takes card says so, with how long it has
    // gone, under the stripes of what is being made (MOTION_UI.md, Making a shot).
    void setMaking(bool making, int take = 0, qint64 since = 0);
    bool isMaking() const { return m_making; }

    // How round the canvas's two lower corners are, where it fills the foot
    // of a rounded panel: nothing is drawn outside them.
    void setFootRadius(qreal radius);
    // How much of the canvas's right is under something else (the inspector):
    // the cards are fitted into what is left.
    void setCovered(int right);
    // The whole graph in view, no larger than life.
    void fit();
    void zoomBy(qreal factor, const QPointF &about);
    qreal zoom() const { return m_zoom; }

    // In this widget's coordinates: a card, where an edge runs, the legend.
    QRectF cardRect(const QString &id) const;
    QPainterPath edgePath(int index) const;
    QRectF legendRect() const;
    // The card beside the legend that holds the zoom's buttons.
    QRectF zoomRect() const;
    // The node whose card is at `at`; empty for none.
    QString nodeAt(const QPointF &at) const;
    // The node whose port, where its edges leave it, is at `at`; empty for none.
    QString portAt(const QPointF &at) const;
    // Where that port is, in this widget.
    QPointF portPoint(const QString &id) const { return toView(outPort(id)); }
    // The types the legend names, in its order.
    QList<graph::Gives> legend() const;

signals:
    void selectionChanged(const QString &id);
    // A card was double-clicked.
    void nodeActivated(const QString &id);
    // An edge was drawn from `from`'s port and let go over `to`, which could
    // read it (graph::portsFor), at `at` on the screen.
    void wired(const QString &from, const QString &to, const QPoint &at);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void leaveEvent(QEvent *event) override;
    bool event(QEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    // A card's rect on the canvas, before the view's pan and zoom.
    QRectF sceneRect(const QString &id) const;
    QPointF toView(const QPointF &scene) const { return scene * m_zoom + m_offset; }
    QPointF toScene(const QPointF &view) const { return (view - m_offset) / m_zoom; }
    // Where an edge leaves its source and enters its target, on the canvas.
    QPointF outPort(const QString &id) const;
    QPointF inPort(const graph::Edge &edge) const;
    QPainterPath scenePath(const graph::Edge &edge) const;
    void paintCard(QPainter &painter, const graph::Node &node) const;
    void paintLegend(QPainter &painter) const;
    // Kept placements, read and written.
    void loadPlaced();
    void savePlaced() const;
    void followMaking();
    void placeControls();

    graph::Recipe m_recipe;
    // Where each card is: as arranged, then as moved by hand.
    QHash<QString, QPointF> m_places;
    QHash<QString, QPointF> m_placed;
    QString m_placementKey;
    QString m_selected;
    QString m_hovered;
    qreal m_zoom = 1.0;
    QPointF m_offset;
    int m_covered = 0;
    qreal m_footRadius = 0;
    // Fitted, until the view is panned or zoomed by hand: a resize fits again.
    bool m_fitted = true;
    // A press, and what it turned into.
    enum class Drag { None, Pan, Card, Wire };
    Drag m_drag = Drag::None;
    bool m_dragMoved = false;
    QPointF m_pressView;
    QPointF m_pressOffset;
    QPointF m_pressPlace;
    // An edge being drawn: from which node, to where the pointer is, and the
    // card under it that could read it.
    QString m_wireFrom;
    QPointF m_wireAt;
    QString m_wireOver;
    // The port under the pointer, which a press would draw an edge from.
    QString m_hoveredPort;
    bool m_making = false;
    int m_makingTake = 0;
    qint64 m_makingSince = 0;
    QTimer *m_ticker = nullptr;
    ui::IconButton *m_zoomOut = nullptr;
    ui::IconButton *m_zoomIn = nullptr;
    ui::GhostButton *m_fit = nullptr;
};

} // namespace kerf
