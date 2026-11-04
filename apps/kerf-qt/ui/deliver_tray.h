#pragma once

// The Deliver tray (mockup-spec 13): in the Deliver workspace it takes the
// whole bottom band. A 28-point bar ('Delivery', then 'Project · Sequence ·
// rev N · <destination>'; the blocking notes; Render master) over four
// panes at the spec's x and width: Masters (RenderQueueView), Preflight QC
// (QcView), Loudness (LoudnessView) and Paperwork (PaperworkView).
//
// The blocking count is the review model's open blocking threads (the
// bridge's `comments` before a model is set), plus the blocking findings
// of the last check that no comment mirrors. Render master renders the
// project's Master output (ProRes 422 HQ, its folder and name; plain
// `export.master` when it has nothing to add), follows that registry action
// (enabled, tip and shortcut), is held back until the first `comments`
// event, and is disabled while a note blocks ('Resolve blocking notes
// first'); it is the primary button once nothing does. When the master
// finishes rendering, the chosen paperwork is written beside it.

#include <QHash>
#include <QJsonObject>
#include <QPointer>
#include <QSize>
#include <QString>
#include <QWidget>

class QAction;
class QLabel;

namespace kerf {

class EditorBus;
class LoudnessView;
class PaperworkView;
class QcView;
class RenderQueueView;

namespace ui {
class Button;
} // namespace ui

class DeliverTray final : public QWidget {
    Q_OBJECT
public:
    explicit DeliverTray(QWidget *parent, EditorBus *bus);
    // The project the tray delivers; without one it is the window's
    // (windowFilePath), else none (nothing is kept per project).
    void setProjectDirectory(const QString &directory);
    QString projectDirectory() const;
    // The registry's `export.master`, which Render master follows; until
    // it is set the button is disabled.
    void setRenderAction(QAction *master);
    // A `render_queue` event: {jobs:[{jobId, name, quality, codec, sequence, status, progress, outputPath, error?}]}.
    void setQueue(const QJsonObject &queue);
    QJsonObject queue() const { return m_queue; }
    void setLoudness(const QJsonObject &loudness);
    void setCheck(const QJsonObject &check);
    // "masters" (or "queue"), "qc", "loudness", "paperwork".
    void highlightPane(const QString &id);
    QString highlightedPane() const { return m_highlighted; }
    QWidget *pane(const QString &id) const { return m_panes.value(id); }
    LoudnessView *loudnessView() const { return m_loudness; }
    QcView *qcView() const { return m_qc; }
    RenderQueueView *renderQueueView() const { return m_queueView; }
    PaperworkView *paperworkView() const { return m_paperwork; }
    ui::Button *renderButton() const { return m_render; }
    // Render master, as its button does: false when it is held back or was
    // not sent.
    bool renderMaster();
    // Open blocking notes, from the review model or the last `comments`
    // event, plus the last check's blocking findings no comment mirrors;
    // -1 before the first `comments` event.
    int blockingCount() const { return m_blocking; }
    QString blockingText() const;
    // "Atlas · Main · rev 22 · deliveries/": the document's project name
    // (timeline.name, else the folder's), the first sequence, the revision
    // and the Master output's configured folder.
    QString projectText() const;
    // Rows of the Masters pane that show a job of the queue.
    int queueRowCount() const;

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void setComments(const QJsonObject &comments);
    void updateBlocking();
    void updateBar();
    void layoutPanes();
    void watchReview();
    void masterCompleted(const QString &output, const QJsonObject &job);

    EditorBus *m_bus = nullptr;
    QString m_directory;
    QHash<QString, QWidget *> m_panes;
    QString m_highlighted;
    QJsonObject m_queue;
    // The last `comments` event, and the last check that finished.
    QJsonObject m_comments;
    bool m_commentsSeen = false;
    QJsonObject m_check;
    int m_blocking = -1;
    QWidget *m_bar = nullptr;
    LoudnessView *m_loudness = nullptr;
    QcView *m_qc = nullptr;
    RenderQueueView *m_queueView = nullptr;
    PaperworkView *m_paperwork = nullptr;
    QLabel *m_project = nullptr;
    QLabel *m_blockingLabel = nullptr;
    QLabel *m_mastersMeta = nullptr;
    QLabel *m_qcMeta = nullptr;
    ui::Button *m_render = nullptr;
    QPointer<QAction> m_renderAction;
    QPointer<QObject> m_review;
};

} // namespace kerf
