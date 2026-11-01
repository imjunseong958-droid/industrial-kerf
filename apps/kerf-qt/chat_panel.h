#pragma once

#include <QAbstractButton>
#include <QDateTime>
#include <QLayout>
#include <QList>
#include <QWidget>

class QComboBox;
class QFrame;
class QHBoxLayout;
class QLabel;
class QPushButton;
class QScrollArea;
class QTextEdit;
class QTimer;
class QToolButton;
class QVBoxLayout;

namespace kerf {

class EdgeCover;

// The lines of a label's text as it lays them out, in its own coordinates:
// each line's box and the y of its capitals' tops.
struct TextLine {
    QRect rect;
    int capTop = 0;
};
QList<TextLine> labelTextLines(const QLabel *label);
// The line of a text button's words, centred in it as it draws them (a
// push button's label, an expander's "2 steps"), in its own coordinates.
TextLine buttonTextLine(const QAbstractButton *button);

// Chips that wrap onto as many lines as their column needs, left to right,
// as tags do in a narrow sidebar: the welcome's examples. On a single line
// they never wrap and scroll sideways instead, as a chip bar does: Kerf's
// next steps over the composer.
class FlowLayout final : public QLayout {
public:
    FlowLayout(QWidget *parent, int spacing);
    void setSingleLine(bool single) { m_singleLine = single; invalidate(); }
    bool singleLine() const { return m_singleLine; }
    // How far a single line is scrolled, and how wide its chips are.
    void setScroll(int scroll);
    int scroll() const { return m_scroll; }
    int contentWidth() const;
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
    // As narrow as anything: chips narrow to the column, never widen it.
    QSize minimumSize() const override;

private:
    // Lays the chips out in `rect` (or only measures) and returns the height they take.
    int arrange(const QRect &rect, bool apply) const;

    QList<QLayoutItem *> m_items;
    int m_spacing;
    bool m_singleLine = false;
    int m_scroll = 0;
};

// One change a pending proposal makes, as its pinned comment lists it: a
// glyph for what it does, a plain description, and a short detail (how
// much, or where).
struct ProposalChange {
    enum class Glyph { Trim, Remove, Add, Title, Move, Split, Level, Effect, Other };
    Glyph glyph = Glyph::Other;
    QString description;      // "Trim fisher-close.mov"
    QString detail;           // "end −1.5 s"
    QString shortDescription; // "Trim fisher-close": its clip names without their file extension
};

// A proposal's change as a row of the pinned comment: a 16px glyph in
// secondary ink at the text column, the description (13px) and, right-
// aligned, the detail (12px, secondary). 28 points tall. The detail is a
// number or a time and is never cut: in a narrow row the description first
// drops its clips' file extensions ("Trim fisher-close"), and where it
// still does not fit beside the detail, the detail moves to a second line
// under the description (kTwoLineHeight tall). Only the description is
// ever elided, and only when it alone is wider than the row. Whatever is
// shortened is said whole in the tooltip.
class ProposalChangeRow final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeight = 28;
    // Two lines: the description's 18-point line, then the detail's 16.
    static constexpr int kTwoLineHeight = 40;
    static constexpr int kDescriptionLine = 18;
    static constexpr int kDetailLine = 16;
    // Where the description starts (the 16-point glyph, then 5), and the
    // room between it and the detail.
    static constexpr int kTextLeft = 16 + 5;
    static constexpr int kDetailGap = 4;
    explicit ProposalChangeRow(const ProposalChange &change, QWidget *parent = nullptr);
    ProposalChange change() const { return m_change; }
    QSize sizeHint() const override { return QSize(160, height()); }
    // The glyph's square, the description's and the detail's rectangles.
    QRect glyphRect() const;
    QRect detailRect() const;
    QRect descriptionRect() const;
    // What the row shows at its width: on one line or two, and the words.
    bool twoLines() const { return fit(width()).twoLines; }
    QString shownDescription() const { return fit(width()).description; }
    QString shownDetail() const { return fit(width()).detail; }

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    struct Fit {
        bool twoLines = false;
        QString description;
        QString detail;
        int detailWidth = 0;
    };
    Fit fit(int width) const;
    QFont descriptionFont() const;
    QFont detailFont() const;
    void updateLines();
    ProposalChange m_change;
};

// A request to try under the welcome: a 16px arrow in secondary ink, then
// the words (13px) wrapping to two lines, no border, a wash under the
// pointer. A click writes it in the composer.
class SuggestionButton final : public QAbstractButton {
    Q_OBJECT
public:
    explicit SuggestionButton(const QString &text, QWidget *parent = nullptr);
    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override;
    QSize sizeHint() const override { return QSize(200, heightForWidth(200)); }
    QSize minimumSizeHint() const override { return QSize(80, heightForWidth(80)); }
    // Where its words go.
    QRect textRect() const;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
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
    // Close, the overflow's dots, and the chevrons that page a row of chips.
    enum class Glyph { Close, More, Back, Forward };
    ChatIconButton(Glyph glyph, int diameter, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    // A disc of the panel's surface under the symbol at rest, as a chip
    // bar's paging buttons have, so it reads over the chips it pages.
    void setFilled(bool filled) { m_filled = filled; update(); }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    Glyph m_glyph;
    int m_diameter;
    bool m_filled = false;
};

// "▸ 2 steps": the reply expander under a comment, in Kerf's ink, with a
// chevron that turns as it opens.
class ChatExpander final : public QAbstractButton {
    Q_OBJECT
public:
    explicit ChatExpander(QWidget *parent = nullptr);
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;
    // A text button (12px/500, no leading pad, so its chevron starts the
    // text column), as on the pinned proposal's pills' row in a narrow
    // panel; with too little room it keeps only its chevron (the tooltip
    // still says what it opens).
    void setCompact(bool compact);
    bool isCompact() const { return m_compact; }
    // Whether its label shows (in a compact one squeezed, only the chevron).
    bool labelShown() const;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    bool m_compact = false;
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
    // The chat, the phone a portrait picture shows in, the laptop a wide
    // one shows in, and safe zones.
    enum class Glyph { Chat, Phone, Laptop, SafeZones };
    static constexpr int kDisc = 44;
    explicit RailButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    void setGlyph(Glyph glyph);
    void setLabel(const QString &label);
    QString label() const { return m_label; }
    // The disc, in the button's coordinates.
    QRect discRect() const;
    QSize sizeHint() const override;
    // A button whose on or off comes from its owner, not from a click: the
    // device button opens a menu, and showing the opposite state while the
    // menu is open said the preview had already changed.
    void setCheckedByOwner(bool owned) { m_checkedByOwner = owned; }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
    void nextCheckState() override {
        if (!m_checkedByOwner) QAbstractButton::nextCheckState();
    }

private:
    Glyph m_glyph;
    QString m_label;
    bool m_checkedByOwner = false;
};

// The rail's buttons, stacked at its foot, level with the picture's bottom
// edge, as Shorts stacks its actions: the device (a phone for a portrait
// picture, a laptop for a wide one) and safe zones (portrait only), then
// the chat.
class StageRail final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kWidth = 48;
    explicit StageRail(QWidget *parent = nullptr);
    RailButton *chatButton() const { return m_chat; }
    RailButton *deviceButton() const { return m_device; }
    RailButton *safeZonesButton() const { return m_safeZones; }

private:
    RailButton *m_device = nullptr;
    RailButton *m_safeZones = nullptr;
    RailButton *m_chat = nullptr;
};

// Kerf's conversation as a video's comments panel: a rounded card with its
// own surface, the same height as the picture beside it. Its header is the
// title and the number of messages, an overflow button and a close button;
// the conversation scrolls as comment rows, cut cleanly at the edge under
// the header, with a hairline there while it is scrolled away from its top;
// a pending proposal is pinned at the top of the list and scrolls away with
// it, a compact bar under the header keeping its Accept and Reject in reach
// while its own are out of view; next steps are chips over the composer,
// which is the panel's footer.
class ChatPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    static constexpr int kRadius = 12;
    static constexpr int kHeaderHeight = 52;
    static constexpr int kSidePadding = 16;
    static constexpr int kAvatar = 28;
    static constexpr int kComposerAvatar = 28;
    // Accept and Reject on the pinned proposal; the composer's pills.
    static constexpr int kPillHeight = 36;
    static constexpr int kComposerPillHeight = 32;
    static constexpr int kChipHeight = 28;
    // The next steps' row: one line of chips that scrolls sideways.
    static constexpr int kChipRowHeight = kChipHeight + 12;
    // The most of the proposal's changes listed under its summary.
    static constexpr int kProposalChangeLines = 3;
    // The most lines the proposal's script and diff show before they scroll.
    static constexpr int kProposalDiffLines = 10;
    // The fade at either end of the chips' row while it has more that way,
    // and the round buttons over it that page the row.
    static constexpr int kChipFadeWidth = 48;
    static constexpr int kChipPager = 28;
    static constexpr int kChipPage = 160;
    // The blank room between the list's view and the chips or composer
    // under it: rows are cut there at every scroll position.
    static constexpr int kListBottom = 16;
    // A row cut at the view's edge that would show less than this of a
    // symbol (an avatar, a glyph, the typing dots), or less than
    // kSliverSymbol of its height, shows none of it; a line of text, less
    // than kSliverLine of its line's height over the bottom edge, or its
    // capitals' tops under the top edge (a name cut through its capitals
    // reads as another word: "Kerr now").
    static constexpr int kSliver = 4;
    static constexpr qreal kSliverSymbol = 0.3;
    static constexpr qreal kSliverAvatar = 0.5;
    static constexpr qreal kSliverLine = 0.45;
    // Under this width the pinned proposal's Script expander leaves the
    // summary's line for the pills' row, so the summary keeps one line.
    static constexpr int kNarrowProposal = 360;

    explicit ChatPanel(QWidget *parent = nullptr);

    // The conversation.
    CommentRow *addComment(CommentRow::Kind kind, const QString &text);
    QList<CommentRow *> rows() const { return m_rows; }
    bool isEmpty() const { return m_rows.isEmpty(); }
    // Every row's name and text, in order, as plain text; replies are not
    // included (they stay folded until opened).
    QString transcript() const;
    // The strips (in the list's view) covering slivers of rows its edges cut.
    QList<QRect> edgeCoverStrips() const;
    // The view keeps to the latest output unless the reader scrolled away.
    bool followsLatest() const { return m_follow; }
    void refreshTimes();

    // The pinned proposal: shown while one is pending, heading the list. A
    // new one (another `id`) scrolls the list to its top, so the proposal
    // is read from its head, and the list stops following the latest
    // comment until the reader scrolls or sends.
    void setProposalShown(bool shown, const QString &id = QString());
    // Its summary in the pinned comment ("Trim, remove, add a title ·
    // 3 changes"); the compact bar's, without the count ("Trim, remove, add
    // a title"); and the count the bar falls back to when it is narrow.
    void setProposalSummary(const QString &summary, const QString &compact = QString(), const QString &count = QString());
    // What it changes, one row each: the first few listed under its summary.
    void setProposalChanges(const QList<ProposalChange> &changes);
    QList<ProposalChangeRow *> proposalChangeRows() const;
    // The hairline closing the pinned comment.
    QFrame *proposalRule() const { return m_proposalRule; }
    QWidget *proposalChanges() const { return m_proposalChanges; }
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
    QList<QAbstractButton *> examples() const { return m_examples; }
    QWidget *statusRow() const { return m_statusRow; }
    QLabel *statusLabel() const { return m_status; }
    QLabel *activityLabel() const { return m_activity; }
    QWidget *chips() const { return m_chips; }
    // The round buttons at the chips' row's ends, shown while it has more that way.
    ChatIconButton *chipsBack() const { return m_chipBack; }
    ChatIconButton *chipsForward() const { return m_chipForward; }
    QFrame *composer() const { return m_composer; }
    QTextEdit *input() const { return m_input; }
    QComboBox *modelSelect() const { return m_model; }
    QPushButton *cancelButton() const { return m_cancel; }
    QPushButton *sendButton() const { return m_send; }
    QPushButton *stopButton() const { return m_stop; }
    QFrame *proposal() const { return m_proposal; }
    // The pinned comment's name line: "Kerf", then the time.
    QLabel *proposalName() const { return m_proposalName; }
    QLabel *proposalTime() const { return m_proposalTime; }
    QLabel *proposalSummary() const { return m_proposalSummary; }
    ChatExpander *proposalExpander() const { return m_proposalExpander; }
    QTextEdit *proposalDiff() const { return m_proposalDiff; }
    QPushButton *acceptButton() const { return m_accept; }
    QPushButton *rejectButton() const { return m_reject; }
    // The compact bar under the header, shown while the pinned proposal's
    // head ("Pinned by Kerf") or its pills are scrolled out of view, and
    // its own summary and pills.
    QFrame *proposalBar() const { return m_proposalBar; }
    QLabel *proposalBarSummary() const { return m_proposalBarSummary; }
    QPushButton *barAcceptButton() const { return m_barAccept; }
    QPushButton *barRejectButton() const { return m_barReject; }
    // The hairline under the header (or the bar), drawn while the list is
    // scrolled away from its top.
    QFrame *listEdge() const { return m_scrollEdge; }
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
    void enterEvent(QEnterEvent *) override;
    void leaveEvent(QEvent *) override;

private:
    QWidget *buildHeader();
    QFrame *buildProposal();
    QFrame *buildProposalBar();
    QWidget *buildWelcome();
    QFrame *buildComposer();
    void updateCount();
    void fitInput();
    void fitProposalDiff();
    // Keep the chips' row's scroll in range and its end fades and pagers in place.
    void updateChipRow();
    void pageChips(int direction);
    // The hairline under the header and the compact proposal bar, as the
    // list's scroll and the proposal's pills stand.
    void updateListEdge();
    void refitListSoon();
    // Where a row is cut at the view's top or bottom edge leaving only a
    // sliver of it (a line's ascenders, an avatar's arc), that sliver is
    // covered in the panel's surface, so the edge is clean, never dirty.
    void updateEdgeCover();
    // The Script expander on the summary's line, or on the pills' row in a
    // narrow panel.
    void placeProposalExpander();
    // The scroll bar shows only while the pointer is over the panel.
    void setHovered(bool hovered);

    QLabel *m_title = nullptr;
    QLabel *m_count = nullptr;
    ChatIconButton *m_more = nullptr;
    ChatIconButton *m_close = nullptr;
    QFrame *m_scrollEdge = nullptr;
    EdgeCover *m_edgeCover = nullptr;
    QFrame *m_proposalBar = nullptr;
    QLabel *m_proposalBarSummary = nullptr;
    QPushButton *m_barAccept = nullptr;
    QPushButton *m_barReject = nullptr;
    QScrollArea *m_scroll = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_list = nullptr;
    QWidget *m_welcome = nullptr;
    QList<QAbstractButton *> m_examples;
    QWidget *m_statusRow = nullptr;
    QWidget *m_typing = nullptr;
    QLabel *m_status = nullptr;
    QLabel *m_activity = nullptr;
    QWidget *m_chips = nullptr;
    QWidget *m_chipFadeLeft = nullptr;
    QWidget *m_chipFadeRight = nullptr;
    ChatIconButton *m_chipBack = nullptr;
    ChatIconButton *m_chipForward = nullptr;
    QWidget *m_proposalChanges = nullptr;
    QFrame *m_composer = nullptr;
    QTextEdit *m_input = nullptr;
    QWidget *m_composerActions = nullptr;
    QComboBox *m_model = nullptr;
    QPushButton *m_cancel = nullptr;
    QPushButton *m_send = nullptr;
    QPushButton *m_stop = nullptr;
    QFrame *m_proposal = nullptr;
    QFrame *m_proposalRule = nullptr;
    QWidget *m_proposalPinLine = nullptr;
    QString m_proposalId;
    QLabel *m_proposalName = nullptr;
    QLabel *m_proposalTime = nullptr;
    QDateTime m_proposalSince;
    QLabel *m_welcomeTime = nullptr;
    QDateTime m_welcomeSince;
    QLabel *m_proposalSummary = nullptr;
    ChatExpander *m_proposalExpander = nullptr;
    QHBoxLayout *m_proposalLine = nullptr;
    QHBoxLayout *m_proposalButtons = nullptr;
    // The summary whole ("Trim, remove, add a title · 3 changes") and
    // without its count, as a narrow panel shows it (its rows count it).
    QString m_summaryFull;
    QString m_summaryShort;
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
