// A shot's graph and the engine (graph_wiring.h).

#include "graph_wiring.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "main_window.h"
#include "modal_scrim.h"
#include "recipe_graph_panel.h"
#include "stage_motion.h"
#include "stage_shell.h"
#include "ui/components.h"

#include <QFileInfo>
#include <QJsonArray>

#include <utility>

namespace kerf {

GraphWiring::GraphWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("graphWiring"));
}

QWidget *GraphWiring::cover() const { return m_scrim; }

void GraphWiring::build() {
    MainWindow *w = m_window;
    // It opens over the window, as everything one goes into does: Esc, its
    // close, or a press on the dim gives the window back as it was.
    m_scrim = new ModalScrim;
    m_scrim->setObjectName(QStringLiteral("graphScrim"));
    m_scrim->setAccessibleName(QStringLiteral("Graph"));
    m_scrim->setPanelSize(QSize(tokens::layout::graphPanelWidth, tokens::layout::graphPanelHeight));
    m_panel = new RecipeGraphPanel;
    m_scrim->setPanel(m_panel);
    connect(m_scrim, &ModalScrim::dismissed, this, &GraphWiring::close);
    connect(m_panel, &RecipeGraphPanel::closeRequested, this, &GraphWiring::close);

    connect(m_panel, &RecipeGraphPanel::edited, this, [this](int line, const QString &key, int nth, const QString &value) {
        edit(QJsonObject{{QStringLiteral("line"), line}, {QStringLiteral("key"), key}, {QStringLiteral("nth"), nth}, {QStringLiteral("value"), value}});
    });
    connect(m_panel, &RecipeGraphPanel::removed, this, [this](int line, const QString &key, int nth) {
        edit(QJsonObject{{QStringLiteral("line"), line}, {QStringLiteral("key"), key}, {QStringLiteral("nth"), nth}});
    });
    // A signal's number drawn by hand is its clip's `value`, keyed: one edit of the project.
    connect(m_panel, &RecipeGraphPanel::signalDrawn, this, [w](const QString &clip, const QJsonArray &keys) {
        w->sendEdit(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")},
                                           {QStringLiteral("clip"), clip},
                                           {QStringLiteral("props"),
                                            QJsonObject{{QStringLiteral("value"), QJsonObject{{QStringLiteral("keys"), keys}}}}}}});
    });
    connect(m_panel, &RecipeGraphPanel::takeChosen, this, [this, w](int number, const QString &path) { w->useTake(m_media, number, path); });
    // Making asks first, as the bin's does: what would run is shown, and runs on a yes.
    connect(m_panel, &RecipeGraphPanel::makeRequested, this, [this, w](int rolls) { w->makeMedia(m_media, rolls); });
    connect(m_panel, &RecipeGraphPanel::stopRequested, this, [this, w] { w->stopMake(m_media); });
    connect(m_panel, &RecipeGraphPanel::castRequested, this, [w] { w->showCast(); });
    connect(m_panel, &RecipeGraphPanel::modelsRequested, this, [w] { w->showSettings(QStringLiteral("models")); });
    // A model's workflow: asked for, shown when the bridge says it, and left by the head's arrow.
    connect(m_panel, &RecipeGraphPanel::workflowRequested, this, [this](int line) {
        m_workflowLine = line;
        requestWorkflow();
    });
    connect(m_panel, &RecipeGraphPanel::workflowClosed, this, [this] {
        m_workflowLine = 0;
        m_panel->closeWorkflow();
    });
    connect(m_panel, &RecipeGraphPanel::workflowEdited, this, [this](const QString &node, const QString &input, const QJsonValue &value) {
        if (m_media.isEmpty() || !m_panel->showsWorkflow()) return;
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("workflow_edit")},
                                {QStringLiteral("media"), m_media},
                                {QStringLiteral("line"), m_panel->workflow().line},
                                {QStringLiteral("was"), m_panel->workflow().text},
                                {QStringLiteral("node"), node},
                                {QStringLiteral("input"), input},
                                {QStringLiteral("value"), value}});
    });

    // Each take read beside its guide, off the bridge's worker: asked once, answered a take at a time.
    connect(m_panel, &RecipeGraphPanel::takesReadRequested, this, [this] {
        if (m_media.isEmpty() || m_readState == QLatin1String("reading") || !m_window->m_connected) return;
        m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("takes_read")}, {QStringLiteral("media"), m_media}});
        m_readings.clear();
        m_best = 0;
        m_readState = QStringLiteral("reading");
        showReadings();
    });

    connect(m_bus, &EditorBus::event, this, &GraphWiring::onEvent);
    connect(m_bus, &EditorBus::snapshotChanged, this, &GraphWiring::onSnapshot);
    connect(w->m_shell, &StageShell::coveredChanged, this, [this](bool covered) {
        // Uncovered by Esc or a press on the dim: nothing more is asked for it.
        if (!covered) m_asked = 0;
    });
}

bool GraphWiring::isOpen() const {
    return m_scrim && m_window->m_shell->covered() && m_window->m_shell->cover() == m_scrim;
}

void GraphWiring::open(const QString &media) {
    MainWindow *w = m_window;
    const QString recipe = w->mediaRecipe(media);
    if (recipe.isEmpty() || !m_scrim) return;
    const bool wasOpen = isOpen();
    m_media = media;
    m_panel->setProjectDirectory(w->m_projectDirectory);
    // Opened anew, it is the shot's graph: a workflow gone into before is gone into again from there.
    if (!wasOpen || m_panel->recipe().media != media) {
        m_workflowLine = 0;
        m_panel->closeWorkflow();
    }
    // Another shot's takes are not these.
    if (m_panel->recipe().media != media) {
        m_readings.clear();
        m_best = 0;
        m_readState.clear();
        showReadings();
    }
    // Each shot keeps its own arrangement, whichever version of its recipe it is made from.
    m_panel->setPlacementKey(QStringLiteral("%1\n%2/%3").arg(w->m_projectDirectory, QFileInfo(recipe).path(), recipeShotName(recipe)));
    // Until the bridge has said the graph, the shot's name over an empty canvas.
    graph::Recipe waiting;
    waiting.media = media;
    waiting.recipe = recipe;
    waiting.name = recipeShotName(recipe);
    waiting.said = false;
    if (m_panel->recipe().media != media) {
        m_panel->setRecipe(waiting);
        m_panel->view()->select(QString());
    }
    followMaking();
    request();
    if (wasOpen) return;
    w->m_shell->setCover(m_scrim);
    w->m_shell->setCovered(true);
    // The room is there at once; what is in it comes up into place.
    moves::rise(m_panel->body(), 0, tokens::motion::pageRise, tokens::motion::page);
    m_panel->view()->setFocus(Qt::OtherFocusReason);
}

void GraphWiring::openWhenReady(const QString &media, const QString &node) {
    m_pendingMedia = media;
    m_pendingNode = node;
    onSnapshot();
}

void GraphWiring::close() {
    if (isOpen()) m_window->m_shell->setCovered(false);
}

void GraphWiring::request() {
    if (m_media.isEmpty()) return;
    m_asked = m_bus->revision();
    // Not sent while the engine is not there to hear it: asked again once it is.
    m_owed = !m_window->m_connected ||
             !m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("recipe_get")}, {QStringLiteral("media"), m_media}});
}

void GraphWiring::requestWorkflow() {
    if (m_media.isEmpty() || m_workflowLine <= 0) return;
    m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("workflow_get")},
                            {QStringLiteral("media"), m_media},
                            {QStringLiteral("line"), m_workflowLine}});
}

void GraphWiring::edit(QJsonObject change) {
    if (m_media.isEmpty()) return;
    change.insert(QStringLiteral("type"), QStringLiteral("recipe_edit"));
    change.insert(QStringLiteral("media"), m_media);
    change.insert(QStringLiteral("was"), m_panel->recipe().text);
    m_bus->send(change);
}

void GraphWiring::showReadings() { m_panel->setTakeReadings(m_readings, m_best, m_readState); }

void GraphWiring::followMaking() {
    MainWindow *w = m_window;
    const auto making = w->m_makingState.constFind(m_media);
    if (making == w->m_makingState.constEnd()) m_panel->setMaking(false);
    else m_panel->setMaking(true, making->take, making->since, making->line);
    const bool can = w->m_commands && w->m_commands->isEnabled(QStringLiteral("media.make"));
    m_panel->setCanMake(can, can || !w->m_commands ? QString() : w->m_commands->reason(QStringLiteral("media.make")));
}

void GraphWiring::onEvent(const QString &type, const QJsonObject &event) {
    if (m_media.isEmpty()) return;
    // What the graph shows is the window's to say first (its makings, its commands).
    if (type == QLatin1String("recipe")) {
        if (event.value(QStringLiteral("media")).toString() != m_media) return;
        m_panel->setRecipe(graph::Recipe::read(event));
        if (const QString node = std::exchange(m_pendingNode, QString()); !node.isEmpty()) {
            // `workflow:<line>` starts on the workflow of the model on that line.
            static const QString workflow = QStringLiteral("workflow:");
            if (node.startsWith(workflow)) {
                m_workflowLine = node.mid(workflow.size()).toInt();
                requestWorkflow();
            } else {
                m_panel->view()->select(node);
            }
        }
        followMaking();
        // The line a workflow is of was changed (its workflow= taken off, a knob
        // set): what fills its blanks, and which file it is, are read again.
        if (m_panel->showsWorkflow() && event.value(QStringLiteral("edited")).toBool()) requestWorkflow();
        return;
    }
    if (type == QLatin1String("workflow")) {
        // The one asked for, of this shot: shown in place of the shot's graph.
        if (event.value(QStringLiteral("media")).toString() == m_media && event.value(QStringLiteral("line")).toInt() == m_workflowLine)
            m_panel->setWorkflow(graph::Workflow::read(event));
        return;
    }
    const bool ofThis = event.value(QStringLiteral("media")).toString() == m_media;
    if (type == QLatin1String("take_read") && ofThis) {
        m_readings.insert(event.value(QStringLiteral("take")).toInt(), event);
        showReadings();
        return;
    }
    if (type == QLatin1String("takes_read") && ofThis) {
        m_best = event.value(QStringLiteral("best")).toInt();
        m_readState = QStringLiteral("read");
        showReadings();
        return;
    }
    if (type == QLatin1String("error") && ofThis && event.value(QStringLiteral("command")).toString() == QLatin1String("takes_read")) {
        // Why its takes cannot be held to anything, said where the reading was asked for.
        m_readState = event.value(QStringLiteral("message")).toString();
        showReadings();
        return;
    }
    if (!isOpen()) return;
    if (type == QLatin1String("media_making") && ofThis) {
        followMaking();
    } else if ((type == QLatin1String("media_made") || type == QLatin1String("media_make_cancelled")) && ofThis) {
        // Its takes are other than they were.
        followMaking();
        request();
    } else if (type == QLatin1String("error") && ofThis) {
        followMaking();
        // A making that failed leaves the graph as it was; a change that was
        // refused is answered with the graph as the file has it.
        if (event.value(QStringLiteral("command")).toString() == QLatin1String("media_make")) request();
    } else if (type == QLatin1String("cast") || type == QLatin1String("models") || type == QLatin1String("connection")) {
        // The cast's words, the models set up, and whether anything can be asked at all.
        if (type != QLatin1String("connection") || m_owed) request();
        followMaking();
    }
}

void GraphWiring::onSnapshot() {
    if (!m_pendingMedia.isEmpty() && !m_window->mediaRecipe(m_pendingMedia).isEmpty()) {
        open(std::exchange(m_pendingMedia, QString()));
        return;
    }
    // The project changed: a signal's keys, a clip's way, a take chosen.
    if (!isOpen() || (m_bus->revision() == m_asked && !m_owed)) return;
    request();
    if (m_panel->showsWorkflow()) requestWorkflow();
    followMaking();
}

} // namespace kerf
