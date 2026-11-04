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
// overwrite edits. Its model is a SourceState, which the window owns, and it
// is the Inspector sheet's Source page: a 280 x 158 canvas (narrower in a
// narrower pane, at its shape: 216 x 122 at 240), the bar, the timecodes, the
// transport, the three edit buttons and the Source clip key-values, in a
// scrolling column. It plays the picture only: no engine path plays a source's
// sound.
//
// The buttons run the registry's commands (setActions) when it has them:
// source.markIn, source.markOut, source.previousFrame, source.nextFrame,
// source.insert, source.overwrite and source.clearMarks, and source.play. Space
// and ⇧F play and reveal the media in the timeline (source.reveal) while the
// viewer has the focus, whatever the keymap.
class SourceMonitorPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // A viewer of `state` (shared, not owned), or of a state of its own.
    explicit SourceMonitorPanel(SourceState *state = nullptr, QWidget *parent = nullptr);
    ~SourceMonitorPanel() override;

    // Shows `state` (not owned) instead; the viewer's own state goes.
    void setState(SourceState *state);
    SourceState *state() const;

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
    // The canvas.
    QRect pictureRect() const;
    QRect barRect() const;
    QRect transportRect() const;
    // The "Source clip" header and its key-value rows.
    QRect sectionRect() const;
    QRect keyValueRect(int index) const;
    // A button by command id: source.markIn, source.markOut,
    // source.previousFrame, source.play, source.nextFrame, source.insert,
    // source.overwrite, source.clearMarks.
    QAbstractButton *button(const QString &id) const;
    // What each key-value row shows now: Name, In, Out, Duration.
    QStringList keyValues() const;
    // The TCs row under the bar, left and right (in and out marks).
    QPair<QString, QString> compactTimecodes() const;
    // The painted body that takes the focus, the pointer and the keys.
    QWidget *body() const;

signals:
    void changed();
    // Open in timeline (⇧F): the window reveals the clip of this media.
    void revealRequested();

protected:
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return {0, 0}; }

private:
    class Body;
    void attach();
    void addSourceActions();

    QPointer<SourceState> m_state;
    SourceState *m_ownState = nullptr;
    Body *m_body = nullptr;
    ui::FadeScrollArea *m_scroll = nullptr;
};

} // namespace kerf
