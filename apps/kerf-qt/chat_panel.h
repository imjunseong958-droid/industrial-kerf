#pragma once

#include <QAbstractButton>
#include <QDateTime>
#include <QLayout>
#include <QList>
#include <QWidget>

class QComboBox;
class QFrame;
class QLabel;
class QPushButton;
class QScrollArea;
class QTextEdit;
class QTimer;
class QToolButton;
class QVBoxLayout;

namespace kerf {

// Chips that wrap onto as many lines as their column needs, left to right,
// as tags do in a narrow sidebar: Kerf's next steps over the composer.
class FlowLayout final : public QLayout {
public:
    FlowLayout(QWidget *parent, int spacing);
    ~FlowLayout() override;
    void addItem(QLayoutItem *item) override { m_items.append(item); }
    int count() const override { return int(m_items.size()); }
    QLayoutItem *itemAt(int index) const override { return m_items.value(index); }
    QLayoutItem *takeAt(int index) override;
    Qt::Orientations expandingDirections() const override { return {}; }
    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override { return arrange(QRect(0, 0, width, 0), false); }
    void setGeometry(const QRect &rect) override;
    QSize sizeHint() const override { return minimumSize(); }
    // As narrow as its widest chip.
    QSize minimumSize() const override;

private:
    // Lays the chips out in `rect` (or only measures) and returns the height they take.
    int arrange(const QRect &rect, bool apply) const;

    QList<QLayoutItem *> m_items;
    int m_spacing;
};

// A round avatar: Kerf's monogram on its purple, or the reader's initial
// on a neutral disc, as a comment row starts with its author's picture.
class ChatAvatar final : public QWidget {
    Q_OBJECT
public:
    enum class Who { Kerf, User };
    ChatAvatar(Who who, int diameter, QWidget *parent = nullptr);
    Who who() const { return m_who; }
    // The letter on the disc.
    QString monogram() const;

protected:
    void paintEvent(QPaintEvent *) override;

private:
    Who m_who;
};

// A round icon button drawn in code, as the comments panel's header and
// sort buttons: no bezel, a round wash under the pointer.
class ChatIconButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Glyph { Close, More };
    ChatIconButton(Glyph glyph, int diameter, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    Glyph m_glyph;
    int m_diameter;
};

// "▸ 2 steps": the reply expander under a comment, in Kerf's ink, with a
// chevron that turns as it opens.
class ChatExpander final : public QAbstractButton {
    Q_OBJECT
public:
    explicit ChatExpander(QWidget *parent = nullptr);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
};

// One turn of the conversation as a comment row: the author's avatar, then
// their name and the time, the text (14/20), the replies Kerf's tools left
// behind it (folded under an expander), and a quiet row of actions. No
// bubble and no box: every turn, Kerf's and the reader's, is a row.
class CommentRow final : public QWidget {
    Q_OBJECT
public:
    enum class Kind { User, Kerf, Error };
    CommentRow(Kind kind, const QString &text, const QDateTime &time, QWidget *parent = nullptr);

    Kind kind() const { return m_kind; }
    QString name() const;
    QString text() const { return m_text; }
    void setText(const QString &text);
    void appendText(const QString &text);
    QDateTime time() const { return m_time; }
    // The time as the row shows it, from `now`.
    void refreshTime(const QDateTime &now);

    // A step Kerf took for this turn, listed under the expander.
    void addReply(const QString &text);
    QStringList replies() const { return m_replies; }
    bool repliesExpanded() const;
    void setRepliesExpanded(bool expanded);

    ChatAvatar *avatar() const { return m_avatar; }
    QLabel *nameLabel() const { return m_nameLabel; }
    QLabel *timeLabel() const { return m_timeLabel; }
    QLabel *body() const { return m_body; }
    ChatExpander *expander() const { return m_expander; }
    QWidget *replyList() const { return m_replyList; }
    QList<QPushButton *> actions() const { return m_actions; }

signals:
    // "Ask again" on one of the reader's rows.
    void askAgain(const QString &text);

private:
    void updateExpander();

    Kind m_kind;
    QString m_text;
    QDateTime m_time;
    QStringList m_replies;
    ChatAvatar *m_avatar = nullptr;
    QLabel *m_nameLabel = nullptr;
    QLabel *m_timeLabel = nullptr;
    QLabel *m_body = nullptr;
    ChatExpander *m_expander = nullptr;
    QWidget *m_replyList = nullptr;
    QVBoxLayout *m_replyLayout = nullptr;
    QList<QPushButton *> m_actions;
};

// A round button on the action rail between the picture and the chat, as
// beside a Short: a disc with a symbol, and a count or label under it.
class RailButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Glyph { Chat };
    static constexpr int kDisc = 44;
    explicit RailButton(Glyph glyph, QWidget *parent = nullptr);
    void setLabel(const QString &label);
    QString label() const { return m_label; }
    // The disc, in the button's coordinates.
    QRect discRect() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    Glyph m_glyph;
    QString m_label;
};

// The rail's buttons, stacked at its foot, level with the picture's bottom
// edge, as Shorts stacks its actions.
class StageRail final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kWidth = 48;
    explicit StageRail(QWidget *parent = nullptr);
    RailButton *chatButton() const { return m_chat; }

private:
    RailButton *m_chat = nullptr;
};

// Kerf's conversation as a video's comments panel: a rounded card with its
// own surface, the same height as the picture beside it. Its header is the
// title and the number of messages, an overflow button and a close button;
// a pending proposal is pinned under it; the conversation scrolls as
// comment rows; next steps are chips over the composer, which is the
// panel's footer.
class ChatPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    static constexpr int kRadius = 12;
    static constexpr int kHeaderHeight = 52;
    static constexpr int kSidePadding = 16;
    static constexpr int kAvatar = 28;
    static constexpr int kComposerAvatar = 28;
    static constexpr int kPillHeight = 32;
    static constexpr int kChipHeight = 28;
    // The most lines the proposal's script and diff show before they scroll.
    static constexpr int kProposalDiffLines = 10;
    // The fade over the top of the conversation's view while it is scrolled.
    static constexpr int kScrollFadeHeight = 14;
    // The least the conversation's view keeps under an open proposal.
    static constexpr int kMinListHeight = 72;

    explicit ChatPanel(QWidget *parent = nullptr);

    // The conversation.
    CommentRow *addComment(CommentRow::Kind kind, const QString &text);
    QList<CommentRow *> rows() const { return m_rows; }
    bool isEmpty() const { return m_rows.isEmpty(); }
    // Every row's name and text, in order, as plain text; replies are not
    // included (they stay folded until opened).
    QString transcript() const;
    // The view keeps to the latest output unless the reader scrolled away.
    bool followsLatest() const { return m_follow; }
    void refreshTimes();

    // The pinned proposal: shown while one is pending.
    void setProposalShown(bool shown);
    bool proposalExpanded() const;
    void setProposalExpanded(bool expanded);
    // The composer: expanded (its second row of model, Cancel and Send
    // shown) while it has focus, holds a draft, or Kerf works.
    bool composerExpanded() const;
    void updateComposer(bool busy);

    QLabel *title() const { return m_title; }
    QLabel *countLabel() const { return m_count; }
    ChatIconButton *moreButton() const { return m_more; }
    ChatIconButton *closeButton() const { return m_close; }
    QScrollArea *scroll() const { return m_scroll; }
    QWidget *welcome() const { return m_welcome; }
    QList<QPushButton *> examples() const { return m_examples; }
    QWidget *statusRow() const { return m_statusRow; }
    QLabel *statusLabel() const { return m_status; }
    QLabel *activityLabel() const { return m_activity; }
    QWidget *chips() const { return m_chips; }
    QFrame *composer() const { return m_composer; }
    QTextEdit *input() const { return m_input; }
    QComboBox *modelSelect() const { return m_model; }
    QPushButton *cancelButton() const { return m_cancel; }
    QPushButton *sendButton() const { return m_send; }
    QPushButton *stopButton() const { return m_stop; }
    QFrame *proposal() const { return m_proposal; }
    QLabel *proposalSummary() const { return m_proposalSummary; }
    ChatExpander *proposalExpander() const { return m_proposalExpander; }
    QTextEdit *proposalDiff() const { return m_proposalDiff; }
    QPushButton *acceptButton() const { return m_accept; }
    QPushButton *rejectButton() const { return m_reject; }
    QWidget *resizeHandle() const { return m_resizeHandle; }

signals:
    // A drag on the panel's leading edge asks for this width, and its end
    // for keeping it.
    void widthDragged(int width);
    void widthDragFinished();
    // "Ask again" on one of the reader's rows.
    void askAgain(const QString &text);

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    QWidget *buildHeader();
    QFrame *buildProposal();
    QWidget *buildWelcome();
    QFrame *buildComposer();
    void updateCount();
    void fitInput();
    void fitProposalDiff();

    QLabel *m_title = nullptr;
    QLabel *m_count = nullptr;
    ChatIconButton *m_more = nullptr;
    ChatIconButton *m_close = nullptr;
    QFrame *m_scrollEdge = nullptr;
    QScrollArea *m_scroll = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_list = nullptr;
    QWidget *m_welcome = nullptr;
    QList<QPushButton *> m_examples;
    QWidget *m_statusRow = nullptr;
    QWidget *m_typing = nullptr;
    QLabel *m_status = nullptr;
    QLabel *m_activity = nullptr;
    QWidget *m_chips = nullptr;
    QFrame *m_composer = nullptr;
    QTextEdit *m_input = nullptr;
    QWidget *m_composerActions = nullptr;
    QComboBox *m_model = nullptr;
    QPushButton *m_cancel = nullptr;
    QPushButton *m_send = nullptr;
    QPushButton *m_stop = nullptr;
    QFrame *m_proposal = nullptr;
    QLabel *m_proposalSummary = nullptr;
    ChatExpander *m_proposalExpander = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
    QPushButton *m_accept = nullptr;
    QPushButton *m_reject = nullptr;
    QWidget *m_resizeHandle = nullptr;
    QList<CommentRow *> m_rows;
    QTimer *m_clock = nullptr;
    bool m_follow = true;
    bool m_busy = false;
    int m_dragFromX = 0;
    int m_dragFromWidth = 0;
};

// The time a comment shows: "now", "4 min ago", "2 h ago", then the date.
QString relativeTime(const QDateTime &then, const QDateTime &now);

} // namespace kerf
