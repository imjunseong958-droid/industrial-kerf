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
    static constexpr const char *kAssistant = "assistant";
    // The width Kerf's conversation opens at the first time, about that of
    // YouTube's comments and live chat beside a video.
    static constexpr int kAssistantWidth = 360;

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
    void persist();

    // Kerf's conversation is a column of its own on the Program monitor's
    // right, as YouTube opens a Short's comments beside it. Closing it gives
    // its width to the viewer; opening it brings it forward at the width it
    // had when it closed.
    bool assistantOpen() const;
    void setAssistantOpen(bool open);
    int assistantWidth() const { return m_assistantWidth; }

signals:
    void changed();

protected:
    // Shows a tab's close button while the pointer is over the tab.
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void showCloseButton(ads::CDockWidgetTab *tab, bool shown);
    void arrangeEditing();
    QByteArray savedLayout(const QString &name) const;
    // Restore a layout saved for workspace `name`: the Editing workspace's
    // only from this version, a workspace of one's own from any version with
    // today's panels.
    bool restore(const QByteArray &layout, const QString &name);
    // The area showing the Program monitor takes what its row gives up: a
    // closed column beside it, or a wider window. A restored layout has new
    // areas, so this follows every restore.
    void letViewerStretch();
    // When the conversation's column closes, the width it had; when it
    // opens alone in a row, that width again, taken from the viewer.
    void rememberAssistantWidth();
    void restoreAssistantWidth();

    ads::CDockManager *m_manager = nullptr;
    QString m_baseStyleSheet;
    QHash<QString, ads::CDockWidget *> m_panels;
    QByteArray m_editing;
    QString m_current;
    int m_assistantWidth = kAssistantWidth;
    // Arranging or restoring: panels open and close without the reader asking.
    bool m_arranging = false;
};

} // namespace kerf
