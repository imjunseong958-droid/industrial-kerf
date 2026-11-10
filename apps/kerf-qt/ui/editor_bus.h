#pragma once

// The editor's bus: how the regions of the window (stage, comments,
// timeline, bin, toolbar) hear the engine and each
// other without including each other's headers.
//
// MainWindow owns it. Every bridge event reaches `event(type, json)` after
// the window has handled it; the state the window keeps (snapshot, playhead,
// selection, program frame) is mirrored here with a signal for
// each change. Requests go back through send() (MainWindow::sendSimple, so a
// test still reads m_lastCommand) and run() (MainWindow::execute: a command
// of the registry, resolved in the window's context).
//
// Cross-region requests are signals only: a region emits one, the region
// that owns the thing named acts on it.

#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSize>
#include <QString>
#include <QStringList>

#include <functional>

namespace kerf {

class ReviewModel;

class EditorBus final : public QObject {
    Q_OBJECT
public:
    using Sender = std::function<bool(const QJsonObject &command)>;
    using Runner = std::function<bool(const QString &commandId, const QJsonObject &params)>;

    explicit EditorBus(QObject *parent = nullptr);

    // ---- outgoing ----------------------------------------------------------
    // A bridge command, as MainWindow::sendSimple sends it; false when it was
    // not sent (offline, no bridge).
    bool send(const QJsonObject &command);
    // A registry command by id (`qc.run`, `comment.accept`), with parameters
    // that take the place of the context; false when it did not run.
    bool run(const QString &commandId, const QJsonObject &params = {});
    // Set by MainWindow; a bus without them sends nothing.
    void setSender(Sender sender) { m_sender = std::move(sender); }
    void setRunner(Runner runner) { m_runner = std::move(runner); }

    // ---- state -------------------------------------------------------------
    // The last state snapshot as the bridge sent it.
    QJsonObject snapshot() const { return m_snapshot; }
    quint64 revision() const { return m_revision; }
    // The program sequence as the window shows it (clips named after their
    // media).
    QJsonObject timeline() const { return m_timeline; }
    double fps() const { return m_fps; }
    QSize frameSize() const { return m_frameSize; }
    double playhead() const { return m_playhead; }
    double duration() const { return m_duration; }
    bool playing() const { return m_playing; }
    // The selected clips, the primary one last.
    QStringList selection() const { return m_selection; }
    // The program's latest frame, as the viewer shows it.
    QImage programFrame() const { return m_programFrame; }
    // The latest event of `type` the bridge sent, empty before the first.
    QJsonObject lastEvent(const QString &type) const { return m_lastEvents.value(type); }
    // The review model (comments and proposals), once one is set.
    ReviewModel *review() const { return m_review; }
    void setReview(ReviewModel *review);
    // The person at this Mac as collab.status names them: {name, email}.
    QJsonObject identity() const { return m_identity; }
    // Guest mode: the person (an email or client id) the panel is viewed as,
    // empty for oneself.
    QString viewAs() const { return m_viewAs; }
    void setViewAs(const QString &person);

    // ---- fed by MainWindow -------------------------------------------------
    // Every bridge event, after the window handled it: kept as lastEvent and
    // sent on as event(type, json). A `collab` status updates identity().
    void dispatch(const QJsonObject &event);
    void setSnapshot(const QJsonObject &snapshot, const QJsonObject &timeline, double fps, const QSize &frameSize);
    void setPlayhead(double seconds);
    void setDuration(double seconds);
    void setPlaying(bool playing);
    void setSelection(const QStringList &clips);
    void setProgramFrame(const QImage &frame);

signals:
    // Every bridge event. (The name hides QObject::event(QEvent *) for
    // callers of the bus by name only; Qt's own event delivery is virtual
    // and unaffected.)
    void event(const QString &type, const QJsonObject &json);
    void snapshotChanged();
    void playheadChanged(double seconds);
    void playingChanged(bool playing);
    void selectionChanged(const QStringList &clips);
    void programFrameChanged(const QImage &frame);
    void reviewChanged();
    void identityChanged(const QJsonObject &identity);
    void viewAsChanged(const QString &person);

    // Cross-region requests.
    // Show the thread `id` in the comments panel (opening it).
    void openThread(const QString &id);
    // The composer's Draw: the stage takes the pointer for a drawing on the
    // frame, then finishes (the strokes, normalised to the picture) or is
    // cancelled.
    void drawingRequested();
    void drawingFinished(const QJsonArray &strokes);
    void drawingCancelled();
    // Open the composer with `text`, as a reply to `threadId` when it is set.
    void askPrefill(const QString &text, const QString &threadId);
    void currentThreadChanged(const QString &id);
    // The comments panel's thread under the pointer (empty when none), and
    // the comment last chosen there: the stage shows their overlap box and
    // drawing.
    void threadHovered(const QString &id);
    void commentSelected(const QString &id);

private:
    Sender m_sender;
    Runner m_runner;
    QJsonObject m_snapshot;
    QJsonObject m_timeline;
    quint64 m_revision = 0;
    double m_fps = 24.0;
    QSize m_frameSize;
    double m_playhead = 0.0;
    double m_duration = 0.0;
    bool m_playing = false;
    QStringList m_selection;
    QImage m_programFrame;
    QHash<QString, QJsonObject> m_lastEvents;
    // Not owned; the window keeps it as long as the bus (a forward
    // declaration only, so no QPointer).
    ReviewModel *m_review = nullptr;
    QJsonObject m_identity;
    QString m_viewAs;
};

} // namespace kerf
