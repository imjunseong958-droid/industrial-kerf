#pragma once

// What the node chosen in a shot's graph holds, said and set: a card over the
// canvas's right, in the fill of what opens from a press (tokens::popup).
//
//   a model      its prompt, its seed (and another, at a press), how long and
//                how large, each knob its line sets or its model has said it
//                takes, and what it reads, each of which can be let go
//   a guide      the sequence it draws, what of it (the picture, or a pass),
//                at what size and rate
//   a signal     its number over its clip, to draw anew by dragging across it
//   the takes    each take there is, the one the shot is ticked, and why the
//                shot wants making where it does
//   the rest     what it is: a sequence, a picture, the way a clip goes, one
//                of the cast
//
// Nothing is kept here. A change is said (`edited`, `removed`, `signalDrawn`,
// `takeChosen`) and the window writes it to the recipe's text or to the
// project; the graph that comes back is what the card then shows. A field
// being typed in is left alone by what comes back for another.

#include "recipe_graph.h"
#include "test_friends.h"

#include <QHash>
#include <QJsonArray>
#include <QString>
#include <QWidget>

class QAbstractButton;
class QLabel;
class QLineEdit;
class QPlainTextEdit;
class QVBoxLayout;

namespace kerf {

class SignalCurve;
namespace ui {
class FadeScrollArea;
}

class RecipeInspector final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit RecipeInspector(QWidget *parent = nullptr);

    // The project's folder, where a picture a node names is.
    void setProjectDirectory(const QString &directory) { m_directory = directory; }
    // The node `id` of `recipe`. With none chosen it says the shot itself:
    // its recipe, why it cannot be made where it cannot, and what is worth
    // knowing before it is.
    void setNode(const graph::Recipe &recipe, const QString &id);
    QString nodeId() const { return m_id; }
    // The workflow being shown, where the canvas shows a model's and not the
    // shot's graph: its nodes are then Parts, and with none chosen the card
    // says the workflow itself and what fills each of its blanks.
    void setWorkflow(const graph::Workflow &workflow);
    // While the shot is being made its takes are not to be chosen among.
    void setMaking(bool making);
    // How tall the card wants to be for what it holds, at most `most`.
    int heightFor(int most) const;

signals:
    // Have line `line` of the recipe give `key` as `value`: the `nth` word of
    // that name. `removed` takes the word off the line.
    void edited(int line, const QString &key, int nth, const QString &value);
    void removed(int line, const QString &key, int nth);
    // A signal clip's number, drawn: its keys, by frame of the clip.
    void signalDrawn(const QString &clip, const QJsonArray &keys);
    // Have the shot be take `number`, whose file the document names as `path`.
    void takeChosen(int number, const QString &path);
    // Open the project's cast, and the person's models, where they are set.
    void castRequested();
    void modelsRequested();
    // Open the workflow the model on line `line` is run with.
    void workflowRequested(int line);
    // Set input `input` of the workflow's node `node` (its id there) to `value`.
    void partEdited(const QString &node, const QString &input, const QJsonValue &value);
    // Go back to the workflow as it comes: the line's `workflow=` is taken off.
    void workflowDropped(int line);
    // What it holds changed, and with it perhaps how tall it wants to be.
    void contentChanged();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    // What decides which widgets the card has: the same again only takes new values.
    QString shape(const graph::Recipe &recipe, const graph::Node &node) const;
    void rebuild(const graph::Recipe &recipe, const graph::Node &node);
    void rebuildShot(const graph::Recipe &recipe);
    // A new, empty column in place of what the card held.
    void startContent();
    void refresh(const graph::Node &node);
    // The value each field shows of `node`, by the field's name.
    QHash<QString, QString> values(const graph::Node &node) const;
    void buildModel(const graph::Recipe &recipe, const graph::Node &node);
    void buildRender(const graph::Node &node);
    void buildSignal(const graph::Node &node);
    void buildTakes(const graph::Node &node);
    void buildReads(const graph::Recipe &recipe, const graph::Node &node);
    void buildPart(const graph::Recipe &recipe, const graph::Node &node);
    void rebuildWorkflow();
    // What fills a blank, and what the line gives for it now, in a line.
    QString blankSays(const QString &name) const;
    // A row: a name, and what sets it.
    void addRow(const QString &label, QWidget *control, int labelWidth = 0);
    void addSection(const QString &title);
    QLabel *addNote(const QString &text, QRgb ink = tokens::ink3);
    // A one-line field that sets `key` on the node's line when it is left
    // changed; emptied, it takes the word off.
    QLineEdit *addField(const QString &name, const QString &label, const QString &key, const QString &placeholder = {});
    void commit(QObject *field, int line, const QString &key, const QString &now);

    QString m_directory;
    QString m_id;
    QString m_shape;
    int m_line = 0;
    bool m_making = false;
    ui::FadeScrollArea *m_scroll = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_column = nullptr;
    // The fields, by name, and the values the card was last built from.
    QHash<QString, QLineEdit *> m_fields;
    QHash<QString, QString> m_values;
    QPlainTextEdit *m_prompt = nullptr;
    SignalCurve *m_curve = nullptr;
    QList<QWidget *> m_takeRows;
    graph::Workflow m_workflow;
    // A Part's yes-or-no inputs, by the field's name.
    QHash<QString, QAbstractButton *> m_switches;
};

} // namespace kerf
