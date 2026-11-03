#pragma once

#include <QByteArray>
#include <QHash>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>

#include <optional>

class QWidget;

namespace ads {
class CDockAreaWidget;
class CDockManager;
class CDockWidget;
class CDockSplitter;
} // namespace ads

namespace kerf {

class BrowserPanel;
class InspectorColumn;

// The docked panels of the editor and its workspaces: named layouts of
// those panels, as in Final Cut Pro. Four are built in: Review (the
// default), Edit, Finish and Deliver; people save their own. Each workspace
// keeps the changes made to it until it is reset to its saved layout.
//
// The tree at 1600 x 1000 (mockup-spec 0.1): a top row of the Program
// monitor (1296) and the Inspector column (304) over a band 344 tall of the
// media bin (272), the timeline (1024) and the browser (304). Finish and
// Deliver put their tray in place of the whole band. Every seam is a 1 pt
// splitter line, counted in the column after it, and the stretch goes to
// the Program monitor and the timeline, so the columns and the band keep
// their sizes as the window resizes.
//
// Panels: program, inspector, project, timeline, browser, finish, deliver.
class Workspace final : public QObject {
    Q_OBJECT
public:
    static constexpr const char *kReview = "Review";
    static constexpr const char *kEdit = "Edit";
    static constexpr const char *kFinish = "Finish";
    static constexpr const char *kDeliver = "Deliver";
    // The built-in workspace's name before layout version 9; its layout and
    // its place as the current workspace go to Review. Deprecated, removed in W3.
    static constexpr const char *kEditing = "Editing";

    // A workspace's stage: whether the comments panel is open, whether the
    // surround is dimmed, and the Inspector's kind (video, audio, info).
    // Kept per workspace on this Mac (workspace/stage/<name>); the regions
    // apply it when the bus says the workspace changed.
    struct StageState {
        bool panelOpen = true;
        bool dim = false;
        QString inspector = QStringLiteral("video");
        bool operator==(const StageState &other) const {
            return panelOpen == other.panelOpen && dim == other.dim && inspector == other.inspector;
        }
    };

    // The dock manager is a child of `host`; add `manager()` to its layout.
    explicit Workspace(QWidget *host);

    ads::CDockManager *manager() const { return m_manager; }
    // A panel whose `id` names it in saved layouts.
    ads::CDockWidget *addPanel(const QString &id, const QString &title, QWidget *content);
    // The panel `id`. The ids of panels that became pages are kept until W3
    // (deprecated): effects and presets are the browser's pages, source the
    // Inspector column's.
    ads::CDockWidget *panel(const QString &id) const;
    // The column whose pages the Inspector commands switch, and the browser
    // whose pages Add effect brings up.
    void setInspectorColumn(InspectorColumn *column);
    void setBrowserPanel(BrowserPanel *browser);
    // A panel's own widgets in its area's text-tab row, shown while it is
    // the area's current panel: `leading` right after the tabs (the
    // timeline's "9 tracks · 00:00:18:00"), `trailing` on the right before
    // its title bar actions (its zoom control). They follow the panel to
    // whatever area holds it, and outlive restored layouts.
    void setTitleWidgets(const QString &id, QWidget *leading, QWidget *trailing);

    // Arrange the panels as the built-in workspaces, then open the
    // workspace in use when the editor last closed, with its changes.
    void start();
    // The built-ins in menu order (Edit, Review, Finish, Deliver), then one's own.
    QStringList names() const;
    static QStringList builtIns();
    static bool isBuiltIn(const QString &name);
    QString current() const { return m_current; }
    void activate(const QString &name);
    // Make the current arrangement (and stage) the saved layout of this workspace.
    void saveCurrent();
    // A new workspace from the current arrangement; false when the name is taken or empty.
    bool saveAs(const QString &name);
    void resetCurrent();
    // Keep the current arrangement for the next launch.
    void persist();

    // The stage of `name` (the current workspace when empty), as last left;
    // a built-in's own when none is kept.
    StageState stageState(const QString &name = QString()) const;
    // The current workspace's stage changed (the panel closed, say).
    void setStageState(const StageState &state);
    static StageState defaultStageState(const QString &name);

    // The Inspector follows the selection: a clip clicked brings it to the
    // front of its column (from the Source page, which a deselect goes back
    // to); a clip selected by an edit does not. It never opens a closed
    // column, and raising leaves keyboard focus where it was.
    enum class InspectorOrigin { Click, Edit };
    static bool inspectorFollowsSelection();
    static void setInspectorFollowsSelection(bool follows);
    // `clip`: the clip clicked, so clicking it again after choosing the
    // Source page leaves that page. `force` raises even when following is off.
    void raiseInspector(InspectorOrigin origin, bool force = false, const QString &clip = QString());
    // The selection was cleared: back to the Source page if the Inspector
    // came up over it by itself.
    void inspectorDeselected();
    // ⌘4: show the Inspector, or, when it is already in front, the page before it.
    void toggleInspector();
    // The Inspector's Add effect: the browser's page `id` (effects,
    // presets) in front, the browser opened for it.
    void showBrowserForInspector(const QString &id);
    bool inspectorRaisedBySelection() const { return m_inspectorAuto; }

signals:
    void changed();

private slots:
    void placeTitleWidgets();

private:
    // Take the title widgets out of the areas before a layout is restored
    // (which deletes the areas and their title bars).
    void parkTitleWidgets();
    // The tree all four built-ins share, sized at the 1600 x 1000 design.
    void arrangeTree();
    // The open panels and the rows' sizes of built-in `name` on that tree.
    void arrangeBuiltIn(const QString &name);
    QByteArray savedLayout(const QString &name) const;
    QByteArray builtInLayout(const QString &name) const;
    // Restore a layout saved for workspace `name`: a built-in's from this
    // version only (one from before today's panels gives way), one's own
    // from version 4 on, its panels renamed to today's.
    bool restore(const QByteArray &layout, const QString &name);
    // After every arrangement or restore (which makes new areas and
    // splitters): 1 pt seams, the stretch, the strips that stand in for
    // the areas' tab rows, and the title widgets.
    void afterRestore();
    // Today's panels a restored layout did not name, placed closed where
    // the tree has them.
    void placeUnassigned();
    void fitSeams();
    void letStageStretch();
    // The row over the band closes when the band's split is dragged to the top.
    void letTopRowCollapse();
    void hideOwnStripTitleBars();
    // Older settings: the Editing workspace became Review (version 9), and
    // one's own workspace named like a built-in is renamed.
    void migrateSettings();

    ads::CDockManager *m_manager = nullptr;
    QString m_baseStyleSheet;
    QHash<QString, ads::CDockWidget *> m_panels;
    QHash<QString, QByteArray> m_builtIn;
    QString m_current;
    // The current workspace's stage as it is now (Save Changes keeps it),
    // until another workspace is chosen.
    std::optional<StageState> m_liveStage;
    QPointer<InspectorColumn> m_column;
    QPointer<BrowserPanel> m_browser;
    // Whether the Inspector came up over the Source page by itself, the
    // page it came from, and the clip that raised it.
    bool m_inspectorAuto = false;
    bool m_sourceBefore = false;
    bool m_raisingInspector = false;
    QString m_inspectorClip;
    struct TitleWidgets {
        QPointer<QWidget> leading;
        QPointer<QWidget> trailing;
    };
    QHash<QString, TitleWidgets> m_titleWidgets;
};

} // namespace kerf
