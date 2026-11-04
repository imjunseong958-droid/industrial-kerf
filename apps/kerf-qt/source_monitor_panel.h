#pragma once

#include "test_friends.h"

#include <QImage>
#include <QJsonObject>
#include <QPointer>
#include <QRect>
#include <QWidget>

class QAbstractButton;
class QAction;
class QVariantAnimation;

namespace kerf {

class SourceState;
namespace ui {
class FadeScrollArea;
}

// The Source viewer (mockup-spec 3.6 and 5.2): one piece of media from the
// bin, scrubbed frame by frame, with in and out marks for insert and
// overwrite edits. Its model is a SourceState, which the window shares
// between its two viewers, so both always show the same media, position
// and marks.
//
//   Stage    the stage column's card (radius 12): a 40 header with the
//            media's name and length and Open in timeline (⇧F), the
//            picture (16:9, pillarboxed under the colH - 280 rule), the bar
//            with the marked range and the position, the transport, a 40 pt
//            filmstrip with the marked box, the media index's shots, and a
//            footer of Insert, Overwrite and Clear in and out.
//   Compact  the Inspector's Source pane: a 280 x 158 canvas (narrower in a
//            narrower pane, at its shape: 216 x 122 at 240), the bar, the
//            timecodes, the transport, the three edit buttons and the
//            Source clip key-values, in a scrolling column.
//
// The stage form fades in when it comes into a window on screen (160 ms; at
// once with reduced motion). It plays the picture only: no engine path plays a source's sound.
//
// The buttons run the registry's commands (setActions) when it has them:
// source.markIn, source.markOut, source.previousFrame, source.nextFrame,
// source.insert, source.overwrite and source.clearMarks, and source.play and
// source.reveal (Play, Open in timeline). Space and ⇧F play and reveal while
// the viewer has the focus, whatever the keymap.
class SourceMonitorPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Mode { Stage, Compact };

    // A viewer of its own state, in the stage form.
    explicit SourceMonitorPanel(QWidget *parent = nullptr);
    // A viewer of `state` (shared, not owned) in `mode`.
    SourceMonitorPanel(SourceState *state, Mode mode, QWidget *parent = nullptr);
    ~SourceMonitorPanel() override;

    // Shows `state` (not owned) instead; the viewer's own state goes.
    void setState(SourceState *state);
    SourceState *state() const;
    void setMode(Mode mode);
    Mode mode() const { return m_mode; }

    // `source` is a snapshot media entry; `path` is resolved against the project.
    void open(const QJsonObject &source, const QString &path);
    void close();
    bool isOpen() const;
    QString mediaId() const;
    QJsonObject source() const;
    double fps() const;
    double position() const;
    double inPoint() const;
    double outPoint() const;
    // The decoded picture on screen; null until the first frame arrives.
    QImage frame() const;

    void seek(double seconds);
    void step(int frames);
    void markIn();
    void markOut();
    void clearMarks();
    // The registry's commands for the buttons (found by their ids), with
    // the source-scoped ones this viewer already carries. They go to the
    // shared state, so every viewer of it runs them.
    void setActions(const QList<QAction *> &actions);

    // ---- geometry, in this viewer's coordinates ------------------------------
    // The stage form at `size`: the black picture box (16 in, under the
    // header) and the 16:9 picture in it, pillarboxed when the column is too
    // short for its width: the height is then 9 * floor((colH - 280) / 9).
    struct StagePicture {
        QRect box;
        QRect picture;
    };
    static StagePicture stagePicture(const QSize &size);
    // The picture's box (stage) or the canvas (compact), and the 16:9 area
    // the frame is fitted into.
    QRect pictureRect() const;
    QRect pictureArea() const;
    QRect headerRect() const;
    QRect barRect() const;
    QRect transportRect() const;
    QRect filmstripRect() const;
    // The shots' rows, the area and one row (stage only).
    QRect rangesRect() const;
    QRect rangeRowRect(int index) const;
    int rangeCount() const;
    // A row's span as painted: its first and last frame, inclusive.
    QString rangeSpan(int index) const;
    QRect footerRect() const;
    // Compact: the "Source clip" header and its key-value rows.
    QRect sectionRect() const;
    QRect keyValueRect(int index) const;
    // A button by command id: source.reveal, source.markIn, source.markOut,
    // source.previousFrame, source.play, source.nextFrame, source.insert,
    // source.overwrite, source.clearMarks (the one this mode shows).
    QAbstractButton *button(const QString &id) const;
    // What each key-value row shows now (compact): Name, In, Out, Duration.
    QStringList keyValues() const;
    // Compact: the TCs row under the bar, left and right (in and out marks).
    QPair<QString, QString> compactTimecodes() const;
    // The painted body that takes the focus, the pointer and the keys.
    QWidget *body() const;
    // The stage form's fade-in: running, and the opacity it is at.
    bool isFading() const;
    qreal opacity() const;

signals:
    void changed();
    // Open in timeline (⇧F): the window reveals the clip of this media.
    void revealRequested();

protected:
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return {0, 0}; }
    void showEvent(QShowEvent *event) override;

private:
    class Body;
    void attach();
    void addSourceActions();

    QPointer<SourceState> m_state;
    SourceState *m_ownState = nullptr;
    Mode m_mode = Mode::Stage;
    Body *m_body = nullptr;
    ui::FadeScrollArea *m_scroll = nullptr;
    QVariantAnimation *m_fade = nullptr;
};

} // namespace kerf
