#pragma once

// The conversation's footer: the composer. What to ask, and under it what the
// request goes with: how far Kerf may go without asking, which model answers,
// the playhead's stamp, how full Kerf's context is, and Send (Stop while Kerf
// works); the suggestions popover over it while it is focused and empty.
//
//   ┌──────────────────────────────────────────────────────────────────────┐
//   │ Ask Kerf to make or change something                        ◔ [send] │  input, UI 14/22
//   └──────────────────────────────────────────────────────────────────────┘
//    [folder name v]                             [model v] [lock Ask before edits v]
//
// The box holds only what is asked and Send. Under it, outside, are the
// quieter things: the folder the request is for (its owner puts that chip
// first in toolRow()) at the start, under where the text starts; the model
// and the mode at the end, under Send. The playhead's stamp joins the folder
// only while the time is not sent. The row is centred on the window's foot
// line, with the sidebar's account.
//
// The composer only collects what is written and chosen; ChatWiring sends it
// as an agent turn (main_window_chat.cpp).

#include "components.h"
#include "test_friends.h"

#include <QAbstractButton>
#include <QColor>
#include <QFrame>
#include <QJsonArray>
#include <QPointer>
#include <QPushButton>
#include <QString>
#include <QStringList>
#include <QWidget>

class QAction;
class QHBoxLayout;
class QLabel;
class QMenu;
class QTextEdit;
class QVariantAnimation;

namespace kerf {

namespace ui {
class IconButton;
} // namespace ui

// How far Kerf goes without asking, as kerf-mcp's permissions name it: every
// edit a proposal to accept; edits accepted as they are made; or edits
// accepted and the shots it asks for made too, up to the person's limit.
enum class PermissionMode { Ask, AcceptEdits, Auto };

namespace permission {
// "ask", "accept-edits", "auto": what a request and the settings say.
QString name(PermissionMode mode);
// The mode a name stands for; anything else asks.
PermissionMode named(const QString &name);
// "Ask before edits", "Accept edits", "Auto": what the chip says.
QString label(PermissionMode mode);
// One line on what it lets through, for the menu and the tip.
QString summary(PermissionMode mode);
// The next mode round, as Shift-Tab goes.
PermissionMode next(PermissionMode mode);
} // namespace permission

// A model's name as it is shown: the aliases Kerf itself offers (default,
// opus, sonnet, haiku) with a capital; a model named by the person as they
// named it, since that is an identifier.
QString modelLabel(const QString &model);

// A chip in the composer's row (28 tall, radius 6, pad 0 6): a 16 pt icon, a
// word in UI 14/20, and the chevron of something that opens: the sizes of the
// window's foot (tokens::layout::footLine). Ink2 at rest, or its tone; hover2
// under the pointer.
class ComposerChip final : public ui::KitButton {
    Q_OBJECT
public:
    explicit ComposerChip(QWidget *parent = nullptr);
    void setIconName(const QString &icon);
    QString iconName() const { return m_icon; }
    void setLabel(const QString &text);
    // The colour of its icon and word at rest; invalid goes back to ink2.
    void setTone(const QColor &tone);
    QColor tone() const { return m_tone; }
    // Whether it opens something: without, it only says.
    void setChevron(bool chevron);
    bool hasChevron() const { return m_chevron; }
    // The widest its word is drawn before it is cut with an ellipsis.
    void setWidestLabel(int width);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_icon;
    QColor m_tone;
    int m_widest = 180;
    bool m_chevron = true;
};

// How full Kerf's context is: a ring, 16 across, filled round from the top as
// the conversation takes more of what a request can hold. Ink2, then the
// warning's amber past three quarters and red past nine tenths. Hidden until
// a turn has said how full it is. Pressed, it opens its card, a popover set as
// a menu is:
//
//   Context                         34%     UI 14 ink; the share 13 ink3
//   ████████████────────────────────────    a 4 pt bar in the ring's colour
//   68k of 200k tokens                      what it is of, 13 ink3
class ContextMeter final : public QWidget {
    Q_OBJECT
public:
    explicit ContextMeter(QWidget *parent = nullptr);
    // `used` of `limit`; `tokens` when they are the model's tokens, not Kerf's
    // own measure. A limit of none hides it.
    void setUse(qint64 used, qint64 limit, bool tokens);
    void clear() { setUse(0, 0, false); }
    // 0 to 1.
    qreal fraction() const;
    qint64 used() const { return m_used; }
    qint64 limit() const { return m_limit; }
    // The ring's colour now.
    QColor ringColor() const;
    // "Context 34% full", and what that is of.
    QString describe() const;
    // What that is of: "68k of 200k tokens", or what Kerf does as it fills.
    QString note() const;
    // Its card, while it is open.
    ui::Popover *card() const { return m_card; }
    void openCard();
    QSize sizeHint() const override { return QSize(24, 24); }

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    qint64 m_used = 0;
    qint64 m_limit = 0;
    bool m_tokens = false;
    QPointer<ui::Popover> m_card;
};

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

// The stamp (.stamp): a request goes with the playhead's time, and that needs
// no saying, so this shows only once the time is taken off (the conversation's
// menu, "Send playhead time"): 28 tall, radius 6, pad 0 6, a 16 pt clock and
// the live playhead TC in mono 12, ink3 and struck through; hover hover2.
// Clicked, the time goes with the request again and the stamp is gone.
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

// A floating card list over the panel (the suggestions popover): card, radius 8, the pop shadow and an inset line2, pad 4;
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
    explicit Composer(QWidget *parent = nullptr);

    QTextEdit *input() const { return m_input; }
    QString text() const;
    SendButton *sendButton() const { return m_send; }
    SendButton *stopButton() const { return m_stop; }
    StampButton *stamp() const { return m_stamp; }
    // What a request goes with, in the row under the text. They show once
    // setAgentControls(true) says the composer sends to an agent that takes
    // them (the start window's does not).
    ComposerChip *modeChip() const { return m_mode; }
    ComposerChip *modelChip() const { return m_model; }
    ContextMeter *contextMeter() const { return m_context; }
    void setAgentControls(bool shown);
    // How far Kerf goes without asking. Shift-Tab in the text goes round the three.
    PermissionMode permissionMode() const { return m_permission; }
    void setPermissionMode(PermissionMode mode);
    // The menu the mode chip opens, made as it opens (for tests).
    QMenu *buildModeMenu();
    // The models the model chip offers (only while `choosable`), and the one
    // each request names.
    void setModels(const QStringList &models, const QString &current, bool choosable);
    QMenu *buildModelMenu();
    QWidget *box() const { return m_box; }
    QLabel *errorLabel() const { return m_error; }
    FloatingList *suggestionsPopover() const { return m_suggestions; }
    // The row under the box, for a control that belongs beside its tools.
    QHBoxLayout *toolRow() const { return m_toolRow; }
    // A larger box for when the composer is the whole of the window's work.
    void setRoomy();
    // The face and size chosen for reading changed: the text is set in them.
    void refreshFaces();

    // Where the popovers float: the panel. They are placed over the composer.
    void setOverlayParent(QWidget *parent);
    // The live playhead TC on the stamp.
    void setTimecode(const QString &timecode);
    // Attached and offered: with no picture open there is no time to stamp.
    bool stamped() const { return m_stampOffered && m_stamp->isAttached(); }
    void setStamped(bool stamped);
    // Whether there is a time to send: a picture is open.
    void setStampOffered(bool offered);
    bool stampOffered() const { return m_stampOffered; }
    // Kerf drafting: Send gives way to Stop.
    void setBusy(bool busy);
    bool isBusy() const { return m_busy; }
    // Whether the focus is in the composer (its text or its buttons).
    bool hasFocusWithin() const;
    // A refused post, shown under the box in neg; empty clears it.
    void setError(const QString &message);
    // The request that goes once Kerf is done (Return while it works), over
    // the box with Cancel; empty: none waits.
    void setQueued(const QString &text);
    QString queued() const { return m_queuedText; }
    QWidget *queuedRow() const { return m_queuedRow; }
    QAbstractButton *queuedCancel() const { return m_queuedCancel; }
    // The popover's requests, shown while the input is focused and empty.
    void setSuggestions(const QStringList &labels);
    QStringList suggestions() const;
    // Writes `text` in the input, the cursor at its end, and focuses it.
    void fill(const QString &text);
    void clear();
    // Whether the popovers would show now (focused and empty, and so on).
    void updatePopovers();

signals:
    void submitRequested();
    void stopRequested();
    // Return while Kerf works: what is written waits for its turn to end.
    void queueRequested();
    void queuedCancelled();
    // Chosen here, from the chip's menu or with Shift-Tab.
    void permissionModeChosen(kerf::PermissionMode mode);
    void modelChosen(const QString &model);
    // The model menu's way to where the connection is set up.
    void agentSettingsRequested();
    void suggestionChosen(int index);
    // Esc with nothing else to close.
    void escapePressed();
    void focusChanged(bool focused);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void resizeEvent(QResizeEvent *) override;
    void moveEvent(QMoveEvent *) override;

private:
    void fitInput();
    void syncStamp();
    void placePopovers();

    QWidget *m_box = nullptr;
    QHBoxLayout *m_toolRow = nullptr;
    // The least the text's field is tall: a line's own height unless roomy.
    int m_leastInput = 0;
    bool m_roomy = false;
    QTextEdit *m_input = nullptr;
    StampButton *m_stamp = nullptr;
    bool m_stampOffered = true;
    ComposerChip *m_mode = nullptr;
    ComposerChip *m_model = nullptr;
    ContextMeter *m_context = nullptr;
    PermissionMode m_permission = PermissionMode::Ask;
    QStringList m_models;
    QString m_currentModel;
    bool m_modelsChoosable = false;
    bool m_agentControls = false;
    SendButton *m_send = nullptr;
    SendButton *m_stop = nullptr;
    QLabel *m_error = nullptr;
    QWidget *m_queuedRow = nullptr;
    QLabel *m_queuedLabel = nullptr;
    QAbstractButton *m_queuedCancel = nullptr;
    QString m_queuedText;
    FloatingList *m_suggestions = nullptr;
    bool m_busy = false;
    bool m_focused = false;
};

} // namespace kerf
