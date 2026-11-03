#pragma once

// The stage region's wiring (Q-STAGE, W2): the Program stage's own state and
// its traffic with the engine and the other regions, for MainWindow (a friend
// of it), so the stage changes the window's behaviour only here and in
// main_window_stage.cpp.
//
//   - The stage's parts: the scrub strip and the agent HUD (built here), the
//     Source column (the window's m_stageSource), placed by the viewer's
//     stage layout; the scrub's narrow mode and the HUD's strip follow it.
//   - Dim surround (view.dim, and each workspace's stage: Finish dims), the
//     comments' drawings (view.annotations, remembered on this Mac), and the
//     viewer's zoom (viewer.zoom: the Fit menu of the Program strip).
//   - The picture's overlays from the engine: the QC overlap box (the
//     `check` finding text.overlap whose comment is the current thread,
//     while the playhead is in it) and the drawings of the current thread's
//     comments (while the playhead is in the comment's time).
//   - A drawing on the frame for the composer: bus drawingRequested starts
//     it, drawingFinished / drawingCancelled end it.

#include "review_model.h"

#include <QAction>
#include <QJsonObject>
#include <QList>
#include <QMenu>
#include <QObject>
#include <QPointer>
#include <QString>

class QAbstractButton;
class QWidget;

namespace kerf {

class AgentHud;
class EditorBus;
class MainWindow;
class ScrubStrip;

class StageWiring final : public QObject {
    Q_OBJECT
public:
    StageWiring(MainWindow *window, EditorBus *bus);
    ~StageWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // From buildStage, once the viewer exists: the scrub strip and the HUD.
    void build();
    // From setupProgramStrip, once every region and the registry exist: the
    // Source column, the commands, the bus.
    void attach();

    ScrubStrip *scrub() const { return m_scrub; }
    AgentHud *hud() const { return m_hud; }

    // Dim surround, as view.dim sets it; `remember` keeps it as the
    // workspace's stage (a workspace's own stage does not).
    void setDim(bool dim, bool remember = true);
    bool dim() const;
    void setAnnotations(bool shown);
    bool annotations() const;
    // 0 is Fit; 0.5, 1, 2 the share of the frame's pixels.
    void setZoom(double zoom);
    double zoom() const;
    static QString zoomText(double zoom);
    // The Fit menu (Fit, 50%, 100%, 200%, Sequence settings…), under the
    // Program strip's zoom button or `anchor`.
    QMenu *zoomMenu();
    void showZoomMenu(QWidget *anchor = nullptr);

    // The thread the overlays follow: the last the bus or the review made
    // current.
    QString currentThread() const;
    // Read the overlays again from the bus (check, comments, playhead).
    void refreshOverlays();
    // The overlap finding shown now, or empty.
    QJsonObject overlapFinding() const { return m_overlapFinding; }

public slots:
    // The thread under the pointer in the comments panel: its overlap box
    // shows too (mockup-spec 3.3: "current or hovered"). Empty when none.
    void hoverThread(const QString &id);
    // The comment chosen in the panel (a reply with a drawing): its drawing
    // shows too, at its time. Empty when none.
    void selectComment(const QString &id);

private:
    void watchReview();
    void applyWorkspaceStage();
    void syncSwitches();
    void placeParts();
    void refreshOverlap();
    void refreshAnnotations();
    void refreshDetail();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    ScrubStrip *m_scrub = nullptr;
    AgentHud *m_hud = nullptr;
    QPointer<QMenu> m_zoomMenu;
    QList<QPair<double, QAction *>> m_zoomActions;
    QPointer<QAction> m_sequenceSettings;
    QPointer<ReviewModel> m_review;
    QList<QMetaObject::Connection> m_reviewConnections;
    QString m_current;
    QString m_hovered;
    QString m_selected;
    QJsonObject m_overlapFinding;
    QJsonObject m_lastCheck;
    bool m_relaying = false;
    bool m_attached = false;
};

} // namespace kerf
