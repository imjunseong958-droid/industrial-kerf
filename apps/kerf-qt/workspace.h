#pragma once

#include <QByteArray>
#include <QHash>
#include <QObject>
#include <QString>
#include <QStringList>

class QWidget;

namespace ads {
class CDockManager;
class CDockWidget;
class CDockWidgetTab;
} // namespace ads

namespace kerf {

// The docked panels of the editor and its workspaces: named layouts of
// those panels. Editing is built in; people save their own. Each
// workspace keeps the changes made to it until it is reset to its saved
// layout, as in other editors.
class Workspace final : public QObject {
    Q_OBJECT
public:
    static constexpr const char *kEditing = "Editing";

    // The dock manager is a child of `host`; add `manager()` to its layout.
    explicit Workspace(QWidget *host);

    ads::CDockManager *manager() const { return m_manager; }
    // A panel whose `id` names it in saved layouts.
    ads::CDockWidget *addPanel(const QString &id, const QString &title, QWidget *content);
    ads::CDockWidget *panel(const QString &id) const { return m_panels.value(id); }

    // Arrange the panels as the Editing workspace, then open the workspace
    // in use when the editor last closed, with its changes.
    void start();
    QStringList names() const;
    QString current() const { return m_current; }
    void activate(const QString &name);
    // Make the current arrangement the saved layout of this workspace.
    void saveCurrent();
    // A new workspace from the current arrangement; false when the name is taken or empty.
    bool saveAs(const QString &name);
    void resetCurrent();
    // Keep the current arrangement for the next launch.
    void persist() const;

signals:
    void changed();

protected:
    // Shows a tab's close button while the pointer is over the tab.
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void showCloseButton(ads::CDockWidgetTab *tab, bool shown);
    void arrangeEditing();
    QByteArray savedLayout(const QString &name) const;
    bool restore(const QByteArray &layout);

    ads::CDockManager *m_manager = nullptr;
    QString m_baseStyleSheet;
    QHash<QString, ads::CDockWidget *> m_panels;
    QByteArray m_editing;
    QString m_current;
};

} // namespace kerf
