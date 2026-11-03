#pragma once

// The Deliver tray (mockup-spec 13): in the Deliver workspace it takes the
// whole bottom band. A 28-point bar (Delivery, the project and revision, the
// blocking notes, Render master) over four panes: Masters (the render
// queue), Preflight QC, Loudness and Paperwork.
//
// Shell from Q-SKELETON (W1): the blocking count comes from the bridge's
// `comments`, the queue from `render_queue`, QC and loudness from `check`
// and `loudness`; Render master runs `export.master`, follows that registry
// action (enabled, tip and shortcut) and is held back until the first
// `comments` event says no blocking note is open. Paperwork stays empty until Q-DELIVER builds it in
// W2. Q-DELIVER fills it in, keeping the constructor and this API.

#include <QHash>
#include <QJsonObject>
#include <QPointer>
#include <QString>
#include <QWidget>

class QAction;
class QLabel;
class QPushButton;
class QVBoxLayout;

namespace kerf {

class EditorBus;
class LoudnessView;
class QcView;

class DeliverTray final : public QWidget {
    Q_OBJECT
public:
    explicit DeliverTray(QWidget *parent, EditorBus *bus);
    // The registry's `export.master`, which Render master follows; until
    // it is set the button is disabled.
    void setRenderAction(QAction *master);
    // A `render_queue` event: {jobs:[{jobId, name, quality, codec, status, progress, outputPath, error?}]}.
    void setQueue(const QJsonObject &queue);
    QJsonObject queue() const { return m_queue; }
    void setLoudness(const QJsonObject &loudness);
    void setCheck(const QJsonObject &check);
    // "masters", "qc", "loudness", "paperwork".
    void highlightPane(const QString &id);
    QString highlightedPane() const { return m_highlighted; }
    QWidget *pane(const QString &id) const { return m_panes.value(id); }
    LoudnessView *loudnessView() const { return m_loudness; }
    QcView *qcView() const { return m_qc; }
    QPushButton *renderButton() const { return m_render; }
    // Open blocking notes, from the last `comments` event; -1 before the first.
    int blockingCount() const { return m_blocking; }
    QString blockingText() const;
    int queueRowCount() const;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void setComments(const QJsonObject &comments);
    void updateBar();
    EditorBus *m_bus = nullptr;
    QHash<QString, QWidget *> m_panes;
    QString m_highlighted;
    QJsonObject m_queue;
    int m_blocking = -1;
    LoudnessView *m_loudness = nullptr;
    QcView *m_qc = nullptr;
    QLabel *m_project = nullptr;
    QLabel *m_blockingLabel = nullptr;
    QLabel *m_mastersMeta = nullptr;
    QLabel *m_qcMeta = nullptr;
    QPushButton *m_render = nullptr;
    QPointer<QAction> m_renderAction;
    QWidget *m_masters = nullptr;
    QVBoxLayout *m_masterRows = nullptr;
};

} // namespace kerf
