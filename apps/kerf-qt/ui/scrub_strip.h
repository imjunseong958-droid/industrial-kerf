#pragma once

// The scrub strip under the picture (mockup-spec 3.4): 56 points tall, as
// wide as the picture on 16:9 and 360 on phones (narrow).
//
//   - The comment lane (y 0-20): one 20-point marker per thread with a time
//     (ReviewModel::markerStacks), people as circles and agents as radius-5
//     squares with their initials, Kerf and QC with their glyphs; current,
//     resolved and hover states; markers closer than 20 points merge into a
//     count stack. Hovering shows a 280-wide popover with each marker's
//     name, role, timecode, Open or Resolved and text. A click opens the
//     thread (bus openThread), makes it current and seeks to it.
//   - The bar (y 24, 4 tall): the played part, the threads' ranges (the
//     current one in the accent, resolved ones faint), and click or drag to
//     seek with a 4-point slop.
//   - The playhead: a 2 x 28 line and a 10-point knob.
//   - The transport (y 28-56): the timecode, mark in and out, go to start,
//     previous frame, play/pause, next frame, go to end, loop, and the
//     duration. Narrow, the duration is hidden.
//
// Engine: the playhead and length come from the bus (clock events); play,
// the frame steps and the ends run the registry's playback commands
// (bus run); a seek is emitted as seekRequested for the stage to queue, or
// sent as a `seek` command when nothing listens. The in and out marks and the
// loop are this strip's: with the loop on, playback that reaches the out
// mark goes back to the in mark (or the start), and playback that ends at
// the program's end starts again from there.

#include "review_model.h"

#include <QHash>
#include <QPointer>
#include <QWidget>

#include <cmath>
#include <limits>

class QAbstractButton;
class QAction;

namespace kerf {

class EditorBus;

namespace ui {
class IconButton;
}

class ScrubStrip final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kLaneHeight = 20;
    static constexpr int kBarTop = 24;
    static constexpr int kBarHeight = 4;
    static constexpr int kBarSlop = 4;
    static constexpr int kTransportTop = 28;
    static constexpr int kTimecodeWidth = 96;
    // The full transport row needs this much; narrower, the strip is narrow.
    static constexpr int kFullWidth = 432;
    static constexpr int kPopoverWidth = 280;

    explicit ScrubStrip(QWidget *parent, EditorBus *bus);
    ~ScrubStrip() override;

    // "00:00:17:08 / 00:00:30:00", from the bus.
    QString timeText() const;
    QString currentText() const;
    QString durationText() const;
    QSize sizeHint() const override;

    // Narrow (phones): the duration hidden and the centre gap 8. A strip
    // narrower than kFullWidth is narrow whatever this says.
    void setNarrow(bool narrow);
    bool isNarrow() const;

    // The registry actions the transport follows (enabled state, shortcut
    // and reason), by command id: playback.toggle, playback.goToStart,
    // playback.previousFrame, playback.nextFrame, playback.goToEnd,
    // playback.loop, mark.in, mark.out. Given none, the strip finds them in
    // its window once it is shown.
    void setActions(const QHash<QString, QAction *> &actions);

    // ---- the lane -----------------------------------------------------------
    QList<ReviewModel::MarkerStack> stacks() const;
    // The 20 x 20 marker of stack `index`.
    QRectF markerRect(int index) const;
    // The stack under `pos`, or -1.
    int stackAt(const QPointF &pos) const;
    int hoveredStack() const { return m_hover; }
    // The marker popover (a child of the window), shown while a marker is
    // hovered; null before the first hover.
    QWidget *popover() const;
    bool popoverShown() const;
    // Opens the thread of stack `index` (its first marker), as a click does.
    void openStack(int index);

    // ---- the bar --------------------------------------------------------------
    QRectF barRect() const;
    double xOf(double seconds) const;
    double timeAt(double x) const;
    struct Range {
        QString id;
        QRectF rect; // on the bar
        QColor color;
        bool current = false;
        bool resolved = false;
    };
    QList<Range> ranges() const;
    double playheadX() const;

    // ---- marks and loop -------------------------------------------------------
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

    // Seeks the program, as clicking the bar does.
    void seek(double seconds);

    QAbstractButton *playButton() const;
    QAbstractButton *loopButton() const;
    QAbstractButton *markInButton() const;
    QAbstractButton *markOutButton() const;
    QAbstractButton *startButton() const;
    QAbstractButton *previousButton() const;
    QAbstractButton *nextButton() const;
    QAbstractButton *endButton() const;
    // The transport's timecodes, where they are drawn.
    QRect currentRect() const;
    QRect durationRect() const;

signals:
    // A seek the stage should queue (MainWindow::queueSeek).
    void seekRequested(double seconds);
    void loopChanged(bool loop);
    void marksChanged();

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void leaveEvent(QEvent *event) override;
    bool event(QEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void watchReview();
    void refresh();
    void relayout();
    void refreshTransport();
    void followPlayhead();
    void followPlaying(bool playing);
    void setHover(int index);
    void showPopover(int index);
    void hidePopover();
    void attachToWindow();
    void run(const QString &command);
    QAction *action(const QString &command) const;

    QPointer<EditorBus> m_bus;
    QList<QMetaObject::Connection> m_reviewConnections;
    QHash<QString, QPointer<QAction>> m_actions;
    bool m_actionsGiven = false;
    bool m_narrow = false;
    int m_hover = -1;
    bool m_dragging = false;
    QPointer<QWidget> m_popover;
    double m_in = std::numeric_limits<double>::quiet_NaN();
    double m_out = std::numeric_limits<double>::quiet_NaN();
    bool m_loop = false;
    // Set when the loop sent the playhead back from the out mark, until the
    // playhead is before it again.
    bool m_wrapped = false;
    // The length the transport was last refreshed for.
    double m_shownDuration = -1.0;
    ui::IconButton *m_markIn = nullptr;
    ui::IconButton *m_markOut = nullptr;
    ui::IconButton *m_start = nullptr;
    ui::IconButton *m_previous = nullptr;
    QAbstractButton *m_play = nullptr;
    ui::IconButton *m_next = nullptr;
    ui::IconButton *m_end = nullptr;
    ui::IconButton *m_loopButton = nullptr;
};

} // namespace kerf
