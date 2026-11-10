#pragma once

// The timeline region's wiring (Q-TIMELINE, W2): a friend of MainWindow that
// holds the region's own state, so the region changes the window's
// behaviour only here and in main_window_timeline.cpp.
//
// What it wires, all of it engine data or engine requests:
//   - the bar's tools strip, placed after "N tracks · duration" in the
//     timeline's title strip; its tools, Snap, Add marker, Add track and
//     Ask run the registry's commands (tool.*, sequence.snap, marker.add,
//     timeline.addTrack, ai.ask) and follow their QActions: enabled with
//     them, the reason when not, and the keymap's shortcut in the tip;
//   - the review: comment marks on the ruler and every open proposal's band
//     and veil, from `comments` and the snapshot's proposals (the bus's
//     ReviewModel when one is set, which also gives the current thread;
//     else one of its own fed from the same events); a mark clicked opens
//     its thread; "Ask · N notes" counts the open threads while the panel
//     is closed;
//   - QC dots from `check` findings, ghosts from `preview` of a proposal;
//   - the source patch for the bin's media (sourceTargets()),
//     again whenever the timeline's targets change;
//   - solo → `solo {tracks}`, sent again when the engine is ready after a
//     reconnect; the rubber band → `set {props: {gain}}`.

#include "review_model.h"
#include "timeline_widget.h"

#include <QHash>
#include <QJsonObject>
#include <QList>
#include <QMetaObject>
#include <QObject>
#include <QPointer>
#include <QSet>
#include <QString>

namespace kerf {

class EditorBus;
class MainWindow;

class TimelineWiring final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    TimelineWiring(MainWindow *window, EditorBus *bus);
    ~TimelineWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // Once the timeline and the bus exist (MainWindow::wireTimeline). Safe to
    // call once only.
    void attach();
    TimelineToolStrip *tools() const { return m_tools; }
    PictureTransport *pictureTransport() const { return m_pictureTransport; }
    // The transport is in the strip over a wide sequence, and under the
    // picture of a vertical one.
    void setWide(bool wide);
    // The review the timeline reads: the bus's when set, else its own.
    ReviewModel *review() const;

    // The review's marks and open proposals as the timeline draws them.
    static QList<TimelineWidget::CommentMark> commentMarks(const ReviewModel &review, const QString &current);
    static QList<TimelineWidget::ProposalBand> proposalBands(const ReviewModel &review);
    // The clips a finished check found problems in (its findings' entities).
    static QSet<QString> qcClips(const QJsonObject &check);
    // The proposal a `preview` event plays, or empty when none plays.
    static QString previewedProposal(const QJsonObject &preview, const QString &playing);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void refreshReview();
    void bindCommands();
    void followCommands();
    void sendSolo(const QStringList &tracks);
    void showTime(double seconds);
    void followSharedReview();
    void handleEvent(const QString &type, const QJsonObject &json);
    QString currentThread() const;
    void triggerCommand(const QString &id);

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    QPointer<TimelineToolStrip> m_tools;
    PictureTransport *m_pictureTransport = nullptr; // the shell's child
    bool m_wide = true;
    ReviewModel *m_own = nullptr;
    QPointer<ReviewModel> m_shared;
    QList<QMetaObject::Connection> m_sharedConnections;
    bool m_attached = false;
    bool m_commandsBound = false;
};

} // namespace kerf
