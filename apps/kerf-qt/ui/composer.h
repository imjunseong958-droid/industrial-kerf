#pragma once

// The Comments panel's footer (mockup-spec 4.8): the reply-to bar and the
// composer, with the timecode stamp, @mention, Draw on frame and Send; the
// suggestions popover over it while it is focused and empty.
//
//   [reply 14] Replying to Mina Park                           [× 20]   (0 → 24)
//   ┌──────────────────────────────────────────────────────────────┐
//   │ Comment or ask Kerf                                          │   input 20, UI 13/20
//   │ [clock 00:00:17:08] [@] [draw]                        [send] │   row 28, gap 4
//   └──────────────────────────────────────────────────────────────┘
//
// The composer only collects what is written; MainWindow posts it (an agent
// turn or a comment_add, main_window_review.cpp).

#include "test_friends.h"

#include <QAbstractButton>
#include <QFrame>
#include <QJsonArray>
#include <QPushButton>
#include <QString>
#include <QStringList>
#include <QWidget>

class QLabel;
class QTextEdit;
class QVariantAnimation;

namespace kerf {

namespace ui {
class IconButton;
} // namespace ui

// Send (.send): 24 x 24, radius 6. Idle (nothing to send): on and ink3;
// ready: accent and white, accent-hover under the pointer. As Stop (Kerf
// drafting): the stop glyph in ink.
class SendButton final : public QPushButton {
    Q_OBJECT
public:
    enum class Mode { Send, Stop };
    explicit SendButton(Mode mode, QWidget *parent = nullptr);
    Mode mode() const { return m_mode; }
    // The fill as drawn now.
    QColor background() const;
    QColor glyphColor() const;
    QSize sizeHint() const override { return QSize(24, 24); }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    Mode m_mode;
};

// The stamp (.stamp): 24 tall, radius 6, pad 0 6 0 4, a 14 pt clock and the
// live playhead TC in mono 11 ink2; hover hover2. Clicked, it detaches:
// ink3 and struck through, and the note is posted without a time.
class StampButton final : public QAbstractButton {
    Q_OBJECT
public:
    explicit StampButton(QWidget *parent = nullptr);
    void setTimecode(const QString &timecode);
    bool isAttached() const { return !isChecked(); }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
};

// The reply-to bar (.rto): 0 → 24 over 160 ms; "Replying to <name>" (or
// "Editing your note") in 11 ink3 with the name in ink2, a 20 pt close (Esc).
class ReplyBar final : public QWidget {
    Q_OBJECT
public:
    explicit ReplyBar(QWidget *parent = nullptr);
    // "Replying to", "Mina Park". An empty name closes the bar.
    void setTarget(const QString &prefix, const QString &name);
    bool isOpen() const { return m_open; }
    QLabel *label() const { return m_label; }
    ui::IconButton *closeButton() const { return m_close; }

signals:
    void cancelled();

private:
    void animateTo(int height);
    bool m_open = false;
    QLabel *m_label = nullptr;
    ui::IconButton *m_close = nullptr;
    QVariantAnimation *m_grow = nullptr;
};

// A floating card list over the panel (the suggestions popover and the
// @mention list): card, radius 8, the pop shadow and an inset line2, pad 4;
// items 28 tall, radius 6, UI 12/16 ink, an optional 14 pt icon in ink3;
// raised under the pointer and on the keyboard's current item.
class FloatingList final : public QWidget {
    Q_OBJECT
public:
    explicit FloatingList(const QString &icon, QWidget *parent = nullptr);
    void setItems(const QStringList &items);
    QStringList items() const { return m_items; }
    int current() const { return m_current; }
    void setCurrent(int index);
    void moveCurrent(int delta);
    // The card's height for its items (the widget adds the shadow's margins).
    int cardHeight() const;
    // Places the card with its bottom edge at `bottom`, from `left` to `right`
    // (in the parent's coordinates).
    void placeAbove(int left, int right, int bottom);
    QRect cardRect() const;
    QRect itemRect(int index) const;

signals:
    void chosen(int index);

protected:
    void paintEvent(QPaintEvent *) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void leaveEvent(QEvent *) override;

private:
    int itemAt(const QPoint &point) const;
    QString m_icon;
    QStringList m_items;
    int m_current = -1;
    int m_hover = -1;
};

class Composer final : public QFrame {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Draw { None, Drawing, Attached };
    explicit Composer(QWidget *parent = nullptr);

    QTextEdit *input() const { return m_input; }
    QString text() const;
    SendButton *sendButton() const { return m_send; }
    SendButton *stopButton() const { return m_stop; }
    StampButton *stamp() const { return m_stamp; }
    ui::IconButton *mentionButton() const { return m_mention; }
    ui::IconButton *drawButton() const { return m_draw; }
    QWidget *box() const { return m_box; }
    QLabel *errorLabel() const { return m_error; }
    FloatingList *suggestionsPopover() const { return m_suggestions; }
    FloatingList *mentionList() const { return m_mentions; }

    // Where the popovers float: the panel. They are placed over the composer.
    void setOverlayParent(QWidget *parent);
    // The live playhead TC on the stamp.
    void setTimecode(const QString &timecode);
    bool stamped() const { return m_stamp->isAttached(); }
    void setStamped(bool stamped);
    // Kerf drafting: Send gives way to Stop.
    void setBusy(bool busy);
    bool isBusy() const { return m_busy; }
    // Draw on frame: the stage is taking the drawing, or one is attached.
    void setDraw(Draw state, const QJsonArray &strokes = {});
    Draw drawState() const { return m_drawState; }
    QJsonArray drawing() const { return m_drawing; }
    // A refused post, shown under the box in neg; empty clears it.
    void setError(const QString &message);
    // The popover's requests, shown while the input is focused and empty.
    void setSuggestions(const QStringList &labels);
    QStringList suggestions() const;
    // People and agents to @mention.
    void setMentionCandidates(const QStringList &names);
    QStringList mentionCandidates() const { return m_candidates; }
    // Inserts "@" and opens the list, as the @ button does.
    void startMention();
    // Writes `text` in the input, the cursor at its end, and focuses it.
    void fill(const QString &text);
    void clear();
    // Whether the popovers would show now (focused and empty, and so on).
    void updatePopovers();

signals:
    void submitRequested();
    void stopRequested();
    void drawClicked();
    void suggestionChosen(int index);
    // Esc with nothing else to close.
    void escapePressed();
    void focusChanged(bool focused);

protected:
    void paintEvent(QPaintEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
    void resizeEvent(QResizeEvent *) override;
    void moveEvent(QMoveEvent *) override;

private:
    void fitInput();
    // The @token at the cursor (without the @), or a null string when the
    // cursor is not in one.
    QString mentionPrefix(int *start = nullptr) const;
    void insertMention(const QString &name);
    void placePopovers();

    QWidget *m_box = nullptr;
    QTextEdit *m_input = nullptr;
    StampButton *m_stamp = nullptr;
    ui::IconButton *m_mention = nullptr;
    ui::IconButton *m_draw = nullptr;
    SendButton *m_send = nullptr;
    SendButton *m_stop = nullptr;
    QLabel *m_error = nullptr;
    FloatingList *m_suggestions = nullptr;
    FloatingList *m_mentions = nullptr;
    QStringList m_candidates;
    QStringList m_mentionShown;
    QJsonArray m_drawing;
    Draw m_drawState = Draw::None;
    bool m_busy = false;
    bool m_focused = false;
};

} // namespace kerf
