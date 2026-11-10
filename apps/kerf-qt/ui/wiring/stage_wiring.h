#pragma once

// The picture region's wiring: its own state and its traffic with the engine
// and the other regions, for MainWindow (a friend of it).
//
//   - The agent HUD and the preview chip (built here), children of the picture.
//   - The comments' drawings (view.annotations, remembered on this Mac), and
//     the viewer's zoom (viewer.zoom: the ≡ menu's View ▸ Viewer zoom).
//   - The program's in and out marks and the loop (mark.in, mark.out,
//     mark.clear, playback.loop): with the loop on, playback that reaches the out mark goes
//     back to the in mark (or the start), and playback that ends at the
//     program's end starts again from there. The timeline's ruler shows them.
//   - The picture's overlays from the engine: the QC overlap box (the `check`
//     finding text.overlap whose comment is the current thread, while the
//     playhead is in it) and the drawings of the current thread's comments
//     (while the playhead is in the comment's time).
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
#include <QWidget>

#include <cmath>
#include <limits>

class QAbstractButton;
class QLabel;

namespace kerf {

class AgentHud;
class EditorBus;
class MainWindow;

// "Previewing proposal vN" and Exit preview, 12 inside the picture's top left
// (mockup #ptitle). It exists only while a proposal is being previewed: the
// bridge's `preview` events say so, and Exit sends preview_exit.
class PreviewChip final : public QWidget {
    Q_OBJECT
public:
    explicit PreviewChip(QWidget *parent, EditorBus *bus);
    QString text() const;
    QAbstractButton *exitButton() const;
    void follow(const QJsonObject &preview);
    // Stands aside while something else says the same, with its own Exit.
    void setSuppressed(bool suppressed);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    EditorBus *m_bus = nullptr;
    QLabel *m_text = nullptr;
    QAbstractButton *m_exit = nullptr;
    bool m_suppressed = false;
};

class StageWiring final : public QObject {
    Q_OBJECT
public:
    StageWiring(MainWindow *window, EditorBus *bus);
    ~StageWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // From buildStage, once the viewer exists: the HUD and the preview chip.
    void build();
    // From setupPictureCommands, once every region and the registry exist:
    // the commands, the bus.
    void attach();

    AgentHud *hud() const { return m_hud; }
    PreviewChip *previewChip() const { return m_chip; }

    void setAnnotations(bool shown);
    bool annotations() const;
    // 0 is Fit; 0.5, 1, 2 the share of the frame's pixels.
    void setZoom(double zoom);
    double zoom() const;
    static QString zoomText(double zoom);
    // The Fit menu (Fit, 50%, 100%, 200%), as the ≡ menu's
    // Viewer zoom; viewer.zoom's key shows it under the ≡ button.
    QMenu *zoomMenu();
    void showZoomMenu(QWidget *anchor = nullptr);

    // ---- marks and loop
    void markIn();
    void markOut();
    void clearMarks();
    bool hasIn() const { return !std::isnan(m_in); }
    bool hasOut() const { return !std::isnan(m_out); }
    double inPoint() const { return m_in; }
    double outPoint() const { return m_out; }
    void setLoop(bool loop);
    bool loop() const { return m_loop; }
    // Where a loop starts and ends: the marks, else the program's ends.
    double loopStart() const;
    double loopEnd() const;

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
    void syncSwitches();
    void showMarks();
    void followPlayhead();
    void followPlaying(bool playing);
    void refreshOverlap();
    void refreshAnnotations();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    AgentHud *m_hud = nullptr;
    PreviewChip *m_chip = nullptr;
    double m_in = std::numeric_limits<double>::quiet_NaN();
    double m_out = std::numeric_limits<double>::quiet_NaN();
    bool m_loop = false;
    // Sent back from the out mark, until the playhead leaves it again.
    bool m_wrapped = false;
    QPointer<QMenu> m_zoomMenu;
    QList<QPair<double, QAction *>> m_zoomActions;
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
