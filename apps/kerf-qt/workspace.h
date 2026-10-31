#pragma once

#include <QByteArray>
#include <QHash>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>

class QWidget;

namespace ads {
class CDockManager;
class CDockWidget;
class CDockSplitter;
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
    void persist();
    // While the picture is portrait, the row over the timeline is taller (by
    // up to 60 points, never leaving the timeline under 280), and back when
    // it is not. Saved layouts never keep it.
    void fitPictureShape(bool portrait);
    int pictureShapeGain() const { return m_shapeGain; }

    // The Inspector follows the selection: a clip clicked brings it to the
    // front of its column (remembering the tab it came from); a clip
    // selected by an edit does not. It never opens a closed column, and
    // raising leaves keyboard focus where it was.
    enum class InspectorOrigin { Click, Edit };
    static bool inspectorFollowsSelection();
    static void setInspectorFollowsSelection(bool follows);
    // `clip`: the clip clicked, so clicking it again after choosing another
    // tab leaves that tab. `force` raises even when following is off (an
    // effect just added from the Inspector).
    void raiseInspector(InspectorOrigin origin, bool force = false, const QString &clip = QString());
    // The selection was cleared: back to the tab the Inspector came from,
    // if it came up by itself.
    void inspectorDeselected();
    // ⌘4: show the Inspector, or, when it is already in front, the tab before it.
    void toggleInspector();
    bool inspectorRaisedBySelection() const { return m_inspectorAuto; }

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
    // The rows of the Editing arrangement (the timeline under the rest), or
    // null when the panels are arranged otherwise.
    ads::CDockSplitter *rowsSplitter() const;
    void shiftRows(int delta);
    QByteArray saveUnshifted();
    void afterRestore();

    ads::CDockManager *m_manager = nullptr;
    QString m_baseStyleSheet;
    QHash<QString, ads::CDockWidget *> m_panels;
    QByteArray m_editing;
    QString m_current;
    bool m_portrait = false;
    int m_shapeGain = 0;
    // Waiting for the rows to be laid out before taking the portrait gain.
    bool m_shapePending = false;
    QPointer<QWidget> m_watchedRows;
    // The tab the Inspector was raised over, whether it came up by itself,
    // and the clip that raised it.
    QPointer<ads::CDockWidget> m_beforeInspector;
    bool m_inspectorAuto = false;
    bool m_raisingInspector = false;
    QString m_inspectorClip;
};

} // namespace kerf
