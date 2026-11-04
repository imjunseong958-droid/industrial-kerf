#include "inspector_wiring.h"
#include "bridge.h"
#include "command_registry.h"
#include "editor_bus.h"
#include "inspector_column.h"
#include "inspector_panel.h"
#include "main_window.h"
#include "review_model.h"
#include "stage_shell.h"


#include <QAction>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>

namespace kerf {

InspectorWiring::InspectorWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("inspectorWiring"));
    m_askName = [](QWidget *parent, const QString &suggested) {
        bool ok = false;
        const QString name = QInputDialog::getText(parent, QStringLiteral("Save effects preset"), QStringLiteral("Name"), QLineEdit::Normal,
                                                   suggested, &ok);
        return ok ? name.trimmed() : QString();
    };
}

QString InspectorWiring::unsupportedReason() {
    return QStringLiteral("Saving effects presets needs a newer Kerf bridge (look_save)");
}

void InspectorWiring::attach() {
    InspectorPanel *inspector = m_window->m_inspector;
    if (!inspector || !m_bus) return;
    // Finish shows the colour wheels over the Video pane.
    const auto finish = [this, inspector] { inspector->setFinish(m_bus->workspace() == QLatin1String("Finish")); };
    connect(m_bus, &EditorBus::workspaceChanged, inspector, finish);
    finish();
    connect(m_bus, &EditorBus::event, this, &InspectorWiring::hear);
    if (const QJsonObject info = m_bus->lastEvent(QStringLiteral("media_info")); !info.isEmpty())
        inspector->setMediaInfo(info.value(QStringLiteral("media")).toObject());
    inspector->setThreadResolver([this](const QString &proposal) { return threadOf(proposal); });
    // The hint's Show: the comments panel open on the proposal's thread.
    connect(inspector, &InspectorPanel::proposalShowRequested, m_bus, [this](const QString &thread, const QString &) {
        emit m_bus->openThread(thread);
    });
    connect(inspector, &InspectorPanel::savePresetRequested, this, &InspectorWiring::savePreset);
}

void InspectorWiring::attachCommands() {
    if (m_commandsAttached || !m_window->m_commands) return;
    m_commandsAttached = true;
    // ⌃⌥4 is the window's (inspector.video); ⌃⌥5 and ⌃⌥6 choose the
    // Audio and Info panes the same way, once kerf-commands enables them.
    connect(m_window->m_commands, &CommandRegistry::toggled, this, [this](const QString &id, bool) {
        if (id != QStringLiteral("inspector.audio") && id != QStringLiteral("inspector.info")) return;
        InspectorColumn *column = m_window->m_inspectorColumn;
        if (!column) return;
        m_window->m_shell->setSheet(StageShell::Sheet::Inspector);
        column->setPage(InspectorColumn::Page::Inspector);
        column->setKind(id == QStringLiteral("inspector.audio") ? InspectorColumn::Kind::Audio : InspectorColumn::Kind::Info);
        m_window->syncPanelCommands();
    });
}

QString InspectorWiring::threadOf(const QString &proposalId) const {
    if (ReviewModel *review = m_bus->review())
        if (const ReviewModel::Proposal *proposal = review->proposal(proposalId); proposal && !proposal->threadId.isEmpty())
            return proposal->threadId;
    // Without the model: the root of a comment that carries the proposal.
    QHash<QString, QString> parents;
    QString carrier;
    for (const QJsonValue &value : m_bus->lastEvent(QStringLiteral("comments")).value(QStringLiteral("items")).toArray()) {
        const QJsonObject comment = value.toObject();
        const QString id = comment.value(QStringLiteral("id")).toString();
        parents.insert(id, comment.value(QStringLiteral("parent")).toString());
        if (carrier.isEmpty() && comment.value(QStringLiteral("proposal")).toString() == proposalId) carrier = id;
    }
    if (carrier.isEmpty()) return QString::fromLatin1(ReviewModel::kProposalThreadPrefix) + proposalId;
    for (int guard = 0; guard < 64 && !parents.value(carrier).isEmpty(); ++guard) carrier = parents.value(carrier);
    return carrier;
}

void InspectorWiring::savePreset(const QString &clip) {
    InspectorPanel *inspector = m_window->m_inspector;
    if (clip.isEmpty() || !inspector || !inspector->presetSaving()) return;
    const QString name = m_askName ? m_askName(m_window, inspector->nameText()) : QString();
    if (name.isEmpty()) return;
    const QJsonObject command{{QStringLiteral("type"), QStringLiteral("look_save")}, {QStringLiteral("clip"), clip}, {QStringLiteral("name"), name}};
    if (!m_window->m_connected) return;
    // Straight to the bridge, as sendSimple would (m_lastCommand included):
    // a bridge without look_save refuses the command, which turns the
    // footer's button off with the reason rather than reporting a full queue.
    m_window->m_lastCommand = command;
    if (!m_window->m_bridge) return;
    const QByteArray payload = QJsonDocument(command).toJson(QJsonDocument::Compact);
    if (!kerf_bridge_send(m_window->m_bridge, payload.constData())) {
        m_lookSaveMissing = true;
        inspector->setPresetSaving(false, unsupportedReason());
    }
}

void InspectorWiring::hear(const QString &type, const QJsonObject &json) {
    InspectorPanel *inspector = m_window->m_inspector;
    if (!inspector) return;
    if (type == QStringLiteral("media_info")) {
        inspector->setMediaInfo(json.value(QStringLiteral("media")).toObject());
    } else if (type == QStringLiteral("look_saved")) {
        m_lookSaveMissing = false;
        inspector->setPresetSaving(true);
    } else if (type == QStringLiteral("error")) {
        // A bridge that does not know look_save says so: the footer's button
        // goes off with the reason.
        const QString command = json.value(QStringLiteral("command")).toString();
        const QString message = json.value(QStringLiteral("message")).toString();
        const bool unknown = message.contains(QStringLiteral("unknown command"), Qt::CaseInsensitive) ||
                             message.contains(QStringLiteral("unknown variant"), Qt::CaseInsensitive);
        if (unknown && (command == QStringLiteral("look_save") || message.contains(QStringLiteral("look_save")))) {
            m_lookSaveMissing = true;
            inspector->setPresetSaving(false, unsupportedReason());
        }
    }
}

} // namespace kerf
