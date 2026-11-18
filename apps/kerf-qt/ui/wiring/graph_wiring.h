#pragma once

// A shot's graph and the engine: what is asked of the bridge to show how a
// shot is made, and what is sent back when something in it is set. A friend of
// MainWindow, as each region's wiring is.
//
// Engine paths:
//   the graph        recipe_get {media} -> recipe {media, recipe, name, text, nodes, edges, error?, cannot?, notes, document?}
//                    (a media made by steps in the document: document true, recipe "", name its id,
//                    text its steps after a `kerf-recipe 1` line; edits below carry that text as `was`)
//   a field set      recipe_edit {media, was, line, key, nth, value?} -> recipe {…, edited}
//                    (`was` is the text the graph was read from: a file changed since is left alone)
//   a step added     recipe_edit {media, was, add: guide|model|join} -> recipe {…, edited, added: <its line>}
//   a step deleted   recipe_edit {media, was, line, remove: true} -> recipe {…, edited}
//   a take's recipe restored   recipe_edit {media, was, restore: <take>} -> recipe {…, edited}
//   a signal drawn   edit {ops: [set <clip> value {keys}]}, as the timeline's own drawing
//   a clip from footage     shot_picture {clip, of} -> shot_picture {path}, the clip's first frame to point on;
//                           shot_follow {clip, of, points, outside} -> state, then shot_followed {says}, or error
//   a signal from a sound   signal_from_sound {clip, of} -> state, the clip's value keyed where the sound's sounds start
//   a take chosen    media_replace {media, path}, as the bin's Takes
//   Make, Stop       the registry's media.make and media.makeCancel, as the bin's
//   a model's workflow   workflow_get {media, line} -> workflow {media, line, model, name, origin, file?,
//                        text, nodes, blanks}, for a model its entry runs with kerf-comfy
//   a value set in it    workflow_edit {media, line, was, node, input, value} -> workflow, then recipe
//   a link pointed elsewhere   workflow_edit {media, line, was, node, input, link: [from, output]} -> the same
//                        (the first writes the shot's own copy beside its recipe and names it on the line)
//   takes held to their guide   takes_read {media} -> take_read {media, take, overlap, clips} for each
//                        take as it is read, then takes_read {media, best?}; no model is run
//
// The graph is read again whenever what it shows may have changed: the
// project (a signal's keys, a clip moved, a take made or chosen), the cast, a
// making that ended.

#include "test_friends.h"

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>

namespace kerf {

class EditorBus;
class MainWindow;
class ModalScrim;
class RecipeGraphPanel;

class GraphWiring final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    GraphWiring(MainWindow *window, EditorBus *bus);

    // Makes the panel and the scrim it opens in; MainWindow::buildGraph.
    void build();
    // What the shell lays over the window while a graph is open.
    QWidget *cover() const;
    RecipeGraphPanel *panel() const { return m_panel; }

    // Opens the graph of how `media` is made over the window, asking the bridge
    // for it; a media made neither from a recipe nor by steps in the document
    // opens nothing.
    void open(const QString &media);
    // The same once the project has said that media, with `node` chosen once
    // its graph is said, or with `workflow:<line>` the workflow of the model on
    // that line opened: for a run that starts on a graph (`--graph`).
    void openWhenReady(const QString &media, const QString &node = {});
    void close();
    bool isOpen() const;
    // The media whose graph is open; empty for none.
    QString media() const { return isOpen() ? m_media : QString(); }

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    // Asks the bridge for the open graph again.
    void request();
    // A change to the recipe's text, where the text is still what was read.
    void edit(QJsonObject change);
    // Asks the bridge for the workflow that shows, or is to show, again.
    void requestWorkflow();
    void followMaking();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    ModalScrim *m_scrim = nullptr;
    RecipeGraphPanel *m_panel = nullptr;
    QString m_media;
    // A graph asked for before the project was there to open it, and the node to choose in it.
    QString m_pendingMedia;
    QString m_pendingNode;
    // The revision the graph was last asked for at.
    quint64 m_asked = 0;
    // The graph was wanted while nothing could be asked (not yet connected):
    // it is asked for as soon as something can.
    bool m_owed = false;
    // The `generate` line whose workflow shows, or was asked for; 0 for none.
    int m_workflowLine = 0;
    // What each of the shot's takes read held to its guide, by its number,
    // which holds best, and how the reading stands (RecipeInspector).
    QHash<int, QJsonObject> m_readings;
    int m_best = 0;
    QString m_readState;
    void showReadings();
};

} // namespace kerf
