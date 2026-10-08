#pragma once

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QString>

class QAction;
class QMenu;
class QMenuBar;
class QWidget;

namespace cut {

// The editor's side of the command registry in `cut-commands`: one QAction
// per command, with the label, shortcuts, menu, and enabled state the
// registry gives it. The window carries out frontend commands and sends the
// requests engine commands resolve to.
class CommandRegistry final : public QObject {
    Q_OBJECT
public:
    // Timeline-scoped shortcuts are attached to `timeline`.
    CommandRegistry(const QString &keymap, QWidget *timeline, QObject *parent = nullptr);

    QAction *action(const QString &id) const { return m_actions.value(id); }
    // The registry's menus in menu-bar order, with a separator between groups.
    QHash<QString, QMenu *> buildMenus(QMenuBar *bar);
    QStringList menuNames() const;

    QJsonArray keymaps() const { return m_keymaps; }
    QString keymap() const { return m_keymap; }
    void setKeymap(const QString &id);
    // The command's label with its first shortcut, for tooltips.
    QString labelWithShortcut(const QString &id, const QString &label = QString()) const;

    // Enable each command for the editor context; unavailable ones carry
    // the reason in their tooltip and status tip.
    void update(const QJsonObject &context);
    QString reason(const QString &id) const { return m_reasons.value(id); }
    bool isEnabled(const QString &id) const { return m_actions.contains(id) && m_reasons.value(id).isEmpty(); }

    // {request} for an engine command, or {error}.
    static QJsonObject run(const QString &id, const QJsonObject &params, const QJsonObject &context);

signals:
    void triggered(const QString &id);
    void toggled(const QString &id, bool checked);
    void shortcutsChanged();

private:
    void load(const QString &keymap);
    void refreshTip(const QString &id);

    QWidget *m_timeline = nullptr;
    QJsonArray m_commands;
    QJsonArray m_keymaps;
    QStringList m_menus;
    QString m_keymap;
    QHash<QString, QAction *> m_actions;
    QHash<QString, QString> m_reasons;
    QJsonObject m_lastContext;
    bool m_hasContext = false;
};

} // namespace cut
