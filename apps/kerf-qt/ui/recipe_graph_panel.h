#pragma once

// The panel a shot's graph opens in, over the window (ui/modal_scrim.h), as
// everything one goes into does:
//
//   ┌──────────────────────────────────────────────────────────────────────┐
//   │ ⌥ bounce  shots/bounce.kerfrecipe     Wants making…  Several ▾ [Make] ✕│  the head
//   ├──────────────────────────────────────────────────────────────────────┤
//   │                                                        ┌───────────┐ │
//   │   court ──▶ Depth guide ──┐                            │ video     │ │
//   │             still ────────┼──▶ video ──▶ bounce        │ Prompt    │ │  the canvas
//   │   bounce (a signal) ──────┘                            │ Seed  [7] │ │  (RecipeGraphView)
//   │                                                        │ Knobs     │ │  and, over its
//   │  — video — image — path — signal                       └───────────┘ │  right, the node
//   └──────────────────────────────────────────────────────────────────────┘  (RecipeInspector)
//
// The head says which shot it is and how it stands, and makes it: Make, or
// Make again once it has a take, Several for two to four takes with each seed
// one on from the last, and Stop while one runs. Making asks first, as the
// bin's Make from Recipe… does, since a recipe is commands someone wrote.
//
// A model its entry runs with kerf-comfy opens further, into the workflow it
// is run with (Open workflow in its card, or a double-click on it): the same
// canvas then shows the ComfyUI graph, a card a node and a row an input, with
// the blanks the recipe fills marked and every other value to set in the
// node's card. The head's arrow goes back to the shot's graph. A value set
// there is kept in a copy of the workflow in the project, beside the recipe,
// which the step's line then names; ComfyUI itself is another program, asked
// over HTTP when the shot is made, and nothing of it is changed.
//
// It keeps nothing of the recipe. What is set is said as a signal; the window
// writes it and hands back the graph as it now is (ui/wiring/graph_wiring.h).

#include "recipe_graph.h"
#include "test_friends.h"

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QJsonValue>
#include <QString>
#include <QWidget>

class QLabel;

namespace kerf {

class MediaArtwork;
class RecipeInspector;
namespace ui {
class Button;
class GhostButton;
class IconButton;
} // namespace ui

class RecipeGraphPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit RecipeGraphPanel(QWidget *parent = nullptr);

    RecipeGraphView *view() const { return m_view; }
    RecipeInspector *inspector() const { return m_inspector; }
    // What comes up into place as the panel opens: everything under its head.
    QWidget *body() const { return m_body; }

    void setProjectDirectory(const QString &directory);
    // What the cards' places are kept under, for this shot.
    void setPlacementKey(const QString &key);
    // The shot's graph. While a workflow shows, it is kept for when that closes.
    void setRecipe(const graph::Recipe &recipe);
    const graph::Recipe &recipe() const { return m_shot; }
    // The workflow behind one of the shot's models, shown in place of the
    // shot's graph until it is closed.
    void setWorkflow(const graph::Workflow &workflow);
    void closeWorkflow();
    bool showsWorkflow() const { return m_workflow.isValid(); }
    const graph::Workflow &workflow() const { return m_workflow; }
    // A making of the shot under way: the take it is making, since when (on
    // activity::clock()), and what its steps last said.
    void setMaking(bool making, int take = 0, qint64 since = 0, const QString &line = {});
    bool isMaking() const { return m_making; }
    // Whether a making can be asked for now, and why not where it cannot.
    void setCanMake(bool can, const QString &why = {});

    // What each take reads held to its guide, by its number, which holds best
    // (0 for none), and how the reading stands (RecipeInspector::setTakeReadings).
    void setTakeReadings(const QHash<int, QJsonObject> &readings, int best, const QString &state);

    ui::Button *makeButton() const { return m_make; }
    ui::GhostButton *severalButton() const { return m_several; }
    ui::GhostButton *addButton() const { return m_add; }
    ui::Button *stopButton() const { return m_stop; }
    QLabel *statusLabel() const { return m_status; }

signals:
    void closeRequested();
    // The recipe's line `line` is to give `key` as `value` (its `nth` word of
    // that name), or with `removed` to give it no longer.
    void edited(int line, const QString &key, int nth, const QString &value);
    void removed(int line, const QString &key, int nth);
    void signalDrawn(const QString &clip, const QJsonArray &keys);
    void takeChosen(int number, const QString &path);
    // Hold the shot's takes to its guide.
    void takesReadRequested();
    // Add a step to the recipe, by its kind (`guide`, `model`, `join`); and
    // take the step on `line` out.
    void stepAddRequested(const QString &kind);
    void stepRemoveRequested(int line);
    // Go back to the recipe take `number` was made from.
    void takeRestoreRequested(int number);
    // Make the shot, `rolls` takes of it; and stop the making that runs.
    void makeRequested(int rolls);
    void stopRequested();
    void castRequested();
    void modelsRequested();
    // Open the workflow the model on `line` is run with; and, from one that
    // shows, go back to the shot's graph.
    void workflowRequested(int line);
    void workflowClosed();
    // Set a value of the workflow that shows: input `input` of its node `node`.
    void workflowEdited(const QString &node, const QString &input, const QJsonValue &value);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void placeInspector();
    void refreshHead();
    // An edge drawn from `from` to `to`: the line's word that reads it, asked
    // for by name where more than one would do.
    void wire(const QString &from, const QString &to, const QPoint &at);
    void read(const graph::Node &to, const QString &port, const QString &value);

    RecipeGraphView *m_view = nullptr;
    RecipeInspector *m_inspector = nullptr;
    QWidget *m_body = nullptr;
    graph::Recipe m_shot;
    // The pictures of the shot's takes and of what its steps drew, by their
    // path in the project, decoded off the paint path as the bin's are.
    MediaArtwork *m_art = nullptr;
    QString m_directory;
    void requestPictures();
    graph::Workflow m_workflow;
    QString m_placementKey;
    ui::IconButton *m_back = nullptr;
    QLabel *m_title = nullptr;
    QLabel *m_path = nullptr;
    QLabel *m_status = nullptr;
    ui::GhostButton *m_several = nullptr;
    ui::GhostButton *m_add = nullptr;
    ui::Button *m_make = nullptr;
    ui::Button *m_stop = nullptr;
    ui::IconButton *m_close = nullptr;
    bool m_making = false;
    int m_makingTake = 0;
    QString m_makingLine;
    bool m_canMake = true;
    QString m_cannotWhy;
};

} // namespace kerf
