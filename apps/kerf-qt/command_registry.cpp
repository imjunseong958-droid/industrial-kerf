#include "command_registry.h"
#include "bridge.h"

#include <QAction>
#include <QJsonDocument>
#include <QKeySequence>
#include <QMenu>
#include <QMenuBar>
#include <QWidget>

namespace kerf {

namespace {

QJsonObject callRegistry(char *(*function)(const char *), const QByteArray &argument) {
    char *raw = function(argument.constData());
    if (!raw) return {};
    const QByteArray bytes(raw);
    kerf_bridge_string_free(raw);
    return QJsonDocument::fromJson(bytes).object();
}

QByteArray compact(const QJsonObject &value) {
    return QJsonDocument(value).toJson(QJsonDocument::Compact);
}

} // namespace

QString CommandRegistry::resolve(const QString &id) {
    // None is renamed now.
    return id;
}

CommandRegistry::CommandRegistry(const QString &keymap, const QHash<QString, QWidget *> &scopes, QObject *parent)
    : QObject(parent), m_scopes(scopes) {
    load(keymap);
}

void CommandRegistry::load(const QString &keymap) {
    const QJsonObject catalog = callRegistry(kerf_commands_catalog, keymap.toUtf8());
    m_keymap = catalog.value(QStringLiteral("keymap")).toString();
    m_keymaps = catalog.value(QStringLiteral("keymaps")).toArray();
    m_commands = catalog.value(QStringLiteral("commands")).toArray();
    m_menus.clear();
    for (const QJsonValue &menu : catalog.value(QStringLiteral("menus")).toArray()) m_menus.append(menu.toString());
    for (const QJsonValue &value : m_commands) {
        const QJsonObject command = value.toObject();
        const QString id = command.value(QStringLiteral("id")).toString();
        QAction *action = m_actions.value(id);
        if (!action) {
            action = new QAction(command.value(QStringLiteral("label")).toString(), this);
            action->setObjectName(id);
            action->setData(id);
            action->setCheckable(command.value(QStringLiteral("checkable")).toBool());
            const QString scope = command.value(QStringLiteral("scope")).toString();
            if (QWidget *panel = m_scopes.value(scope)) {
                // The timeline takes focus itself; the source monitor's
                // shortcuts also apply while one of its controls has it.
                action->setShortcutContext(scope == QStringLiteral("timeline") ? Qt::WidgetShortcut
                                                                                : Qt::WidgetWithChildrenShortcut);
                panel->addAction(action);
            }
            if (action->isCheckable())
                connect(action, &QAction::toggled, this, [this, id](bool checked) { emit toggled(id, checked); });
            else
                connect(action, &QAction::triggered, this, [this, id] { emit triggered(id); });
            m_actions.insert(id, action);
        }
        QList<QKeySequence> shortcuts;
        for (const QJsonValue &key : command.value(QStringLiteral("keys")).toArray())
            shortcuts.append(keymapKey(key.toString()));
        action->setShortcuts(shortcuts);
        refreshTip(id);
    }
    emit shortcutsChanged();
}

QKeySequence CommandRegistry::keymapKey(const QString &key, bool mac) {
    QString text = key;
    if (!mac) text.replace(QStringLiteral("Meta+"), QStringLiteral("Ctrl+"));
    return QKeySequence::fromString(text, QKeySequence::PortableText);
}

void CommandRegistry::setKeymap(const QString &id) {
    if (id != m_keymap) load(id);
}

QStringList CommandRegistry::menuNames() const {
    return m_menus;
}

QHash<QString, QMenu *> CommandRegistry::buildMenus(QMenuBar *bar) {
    QHash<QString, QMenu *> menus;
    for (const QString &name : m_menus) {
        QMenu *menu = bar->addMenu(name);
        menu->setToolTipsVisible(true);
        menus.insert(name, menu);
        int group = -1;
        for (const QJsonValue &value : m_commands) {
            const QJsonObject command = value.toObject();
            if (command.value(QStringLiteral("menu")).toString() != name) continue;
            const int commandGroup = command.value(QStringLiteral("group")).toInt();
            if (group >= 0 && commandGroup != group) menu->addSeparator();
            group = commandGroup;
            menu->addAction(m_actions.value(command.value(QStringLiteral("id")).toString()));
        }
    }
    return menus;
}

QString CommandRegistry::labelWithShortcut(const QString &id, const QString &label) const {
    const QAction *action = m_actions.value(id);
    const QString text = label.isEmpty() && action ? action->text() : label;
    if (!action || action->shortcut().isEmpty()) return text;
    return QStringLiteral("%1 (%2)").arg(text, action->shortcut().toString(QKeySequence::NativeText));
}

void CommandRegistry::refreshTip(const QString &id) {
    QAction *action = m_actions.value(id);
    if (!action) return;
    const QString reason = m_reasons.value(id);
    const QString tip = labelWithShortcut(id);
    action->setToolTip(reason.isEmpty() ? tip : QStringLiteral("%1\n%2").arg(tip, reason));
    action->setStatusTip(reason);
}

void CommandRegistry::update(const QJsonObject &context) {
    if (m_hasContext && context == m_lastContext) return;
    m_lastContext = context;
    m_hasContext = true;
    const QJsonObject states = callRegistry(kerf_commands_states, compact(context));
    for (auto it = m_actions.cbegin(); it != m_actions.cend(); ++it) {
        const QJsonValue state = states.value(it.key());
        // A command the registry did not answer for stays unavailable.
        const QString reason = state.isNull() ? QString()
            : state.isString() ? state.toString() : QStringLiteral("Unavailable");
        const bool changed = reason != m_reasons.value(it.key()) || !m_reasons.contains(it.key());
        m_reasons.insert(it.key(), reason);
        it.value()->setEnabled(reason.isEmpty());
        if (changed) refreshTip(it.key());
    }
}

QJsonObject CommandRegistry::run(const QString &id, const QJsonObject &params, const QJsonObject &context) {
    const QJsonObject invocation{{QStringLiteral("command"), id},
                                 {QStringLiteral("params"), params},
                                 {QStringLiteral("context"), context}};
    QJsonObject result = callRegistry(kerf_commands_run, compact(invocation));
    if (result.isEmpty()) result.insert(QStringLiteral("error"), QStringLiteral("The command registry did not answer."));
    return result;
}

} // namespace kerf
