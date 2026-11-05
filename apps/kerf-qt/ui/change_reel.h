#pragma once

// Watching only what a proposal changes: its diff, for video. reel::plan turns
// the proposal's clip changes into segments with a little of what surrounds
// them; ChangeReel plays those in turn, from the proposal's preview or, on the
// Before side, from the program as it is; ChangeReelBar is its strip over the
// picture.
//
//   [2 / 5] Trim sea  ·  18 s left     [<] [>]  [Before|After]  [Exit]

#include "review_model.h"
#include "test_friends.h"

#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QString>
#include <QWidget>

#include <functional>

class QLabel;

namespace kerf {

namespace ui {
class GhostButton;
class IconButton;
class SegmentedControl;
} // namespace ui

namespace reel {

// One stretch to watch, on each side of the proposal.
struct Segment {
    double before = 0.0, beforeEnd = 0.0; // on the program as it is
    double after = 0.0, afterEnd = 0.0;   // on what the proposal makes
    QString title;
    double length() const { return afterEnd - after; }
};

// The segments of `changes`, each with `handle` seconds either side, in order,
// with those that touch merged. The ends keep to each side's duration.
QList<Segment> plan(const QList<ReviewModel::ClipChange> &changes, double beforeDuration, double afterDuration, double handle = 2.0);
double total(const QList<Segment> &segments);
// "Trim sea": what a change does, in words.
QString changeTitle(const ReviewModel::ClipChange &change);
// A position as a tool wrote it (`01:00:04:12`, `96f`, `4.5s`, or plain
// seconds), in seconds from the sequence's start. Negative when unreadable.
double parseTime(const QString &text, double fps, double sequenceStart);

} // namespace reel

class ChangeReel final : public QObject {
    Q_OBJECT
public:
    enum class Side { After, Before };
    // What it needs of the window.
    struct Host {
        std::function<bool(const QString &proposal, double start, double duration)> preview;
        std::function<void()> exitPreview;
        std::function<void(double seconds, double limit)> seek;
        std::function<void(bool playing)> play;
    };
    // The most a preview renders at once.
    static constexpr double kMostRendered = 120.0;

    explicit ChangeReel(Host host, QObject *parent = nullptr);

    bool start(const QString &proposalId, const QList<reel::Segment> &segments);
    void stop();
    void next();
    void previous();
    void setSide(Side side);

    bool isActive() const { return !m_segments.isEmpty(); }
    QString proposalId() const { return m_proposal; }
    int index() const { return m_index; }
    int count() const { return int(m_segments.size()); }
    Side side() const { return m_side; }
    // Waiting for the picture of the side and segment shown.
    bool isWaiting() const { return m_waiting; }
    bool isFinished() const { return m_finished; }
    const QList<reel::Segment> &segments() const { return m_segments; }
    double remaining(double playhead) const;

    // The bridge's `preview` events, and where playback has got to.
    void followPreview(const QJsonObject &event);
    void followPlayhead(double seconds);

signals:
    void changed();

private:
    void show();
    void enterSegment();
    void finish();

    Host m_host;
    QString m_proposal;
    QList<reel::Segment> m_segments;
    int m_index = 0;
    Side m_side = Side::After;
    bool m_waiting = false;
    bool m_finished = false;
    bool m_rendered = false;
    bool m_previewing = false;
    double m_renderedStart = 0.0;
    double m_renderedEnd = 0.0;
    double m_segmentEnd = 0.0;
    double m_limit = 0.0;
    int m_entry = 0;
};

class ChangeReelBar final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = 40;
    explicit ChangeReelBar(ChangeReel *reel, QWidget *parent = nullptr);
    // Refreshes what it says.
    void update(double playhead);
    QString countText() const;
    QString titleText() const;
    ui::IconButton *previousButton() const { return m_previous; }
    ui::IconButton *nextButton() const { return m_next; }
    ui::SegmentedControl *sides() const { return m_sides; }
    ui::GhostButton *exitButton() const { return m_exit; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    ChangeReel *m_reel = nullptr;
    QLabel *m_count = nullptr;
    QLabel *m_title = nullptr;
    ui::IconButton *m_previous = nullptr;
    ui::IconButton *m_next = nullptr;
    ui::SegmentedControl *m_sides = nullptr;
    ui::GhostButton *m_exit = nullptr;
};

} // namespace kerf
