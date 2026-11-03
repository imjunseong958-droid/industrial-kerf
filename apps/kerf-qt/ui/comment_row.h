#pragma once

// A comment row of the Comments panel (mockup-spec 4.6) and the small
// controls it is made of: the timecode link, the resolve ring, tags, the
// fold link, the steps list, the drafting dots and skeleton, and the
// suggestion rows of the Kerf welcome.
//
//   [24 avatar] 12 [name 500 13/20 · role 12 ink3 · TC link · Couldn't edit · drawing · time 11 ink3 … ring]
//                  [text 13/20 ink, margin-top 2]
//                  [tags row 16, margin-top 4, gap 8]
//                  [steps | suggestions | skeleton | proposal card]
//                  hover actions at right 20, top −4: [reply][resolve][more]
//
// The row's background layer reaches 8 past its content on either side and
// 6 above and below (the mockup's .cm::before), so a row is 16 wider and 12
// taller than its content; rows 12 apart touch exactly.
//
// Everything a row shows comes from a review log record (ReviewModel::
// Comment) or from the agent turn in progress (a local row), never from
// sample data.

#include "test_friends.h"

#include <QAbstractButton>
#include <QDateTime>
#include <QJsonArray>
#include <QList>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QWidget>

class QHBoxLayout;
class QLabel;
class QTimer;
class QVBoxLayout;
class QVariantAnimation;

namespace kerf {

namespace ui {
class Avatar;
class GhostButton;
class IconButton;
} // namespace ui

namespace review {

// "00:00:17:08" at `fps` (whole frames, non-drop).
QString timecode(double seconds, double fps);
// A length as seconds and frames: "2s12f", "1s02f", "12f".
QString lengthText(double seconds, double fps);
// "00:00:17:08 · 2s12f" for a range, "00:00:17:08" for a point.
QString markText(double at, double duration, double fps);
// "+17f", "−3f" (a true minus), "0f".
QString signedFrames(qint64 frames);
// The age a row shows: "now", "9 min", "2 h", "Mon", then "3 Nov".
QString age(const QDateTime &then, const QDateTime &now);
// Its tooltip: "rev 21 · 14:20, 3 Nov" (without the revision when unknown).
QString ageTip(qint64 revision, const QDateTime &created);
// The seconds of "HH:MM:SS:FF" (or ;FF) at `fps`; NaN when it is not one.
double parseTimecode(const QString &text, double fps);
// The text as rich text: escaped, inline timecodes as tc:<seconds> links
// (mono 12, accent ink), @mentions in ink 500, line breaks kept.
QString richBody(const QString &body, double fps);
// The @mentions in `text`, without the @, in order.
QStringList mentions(const QString &text);
// Whether the text is for Kerf: it mentions @Kerf, or no one.
bool addressesKerf(const QString &text);

} // namespace review

// .tc: a timecode (or range) link, mono 12/16 accent ink at 85%; hover and
// keyboard focus take it to full ink with a dotted underline. A click seeks.
class TimecodeLink final : public QAbstractButton {
    Q_OBJECT
public:
    explicit TimecodeLink(QWidget *parent = nullptr);
    void setMark(double at, double duration, double fps);
    double at() const { return m_at; }
    double duration() const { return m_duration; }
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    double m_at = 0.0;
    double m_duration = 0.0;
};

// .ring: the 24 pt resolve ring, ink3; hover ink on raised. Resolved shows
// g-ringdone in ink2 (tooltip Reopen).
class ResolveRing final : public QAbstractButton {
    Q_OBJECT
public:
    explicit ResolveRing(QWidget *parent = nullptr);
    void setResolved(bool resolved);
    bool isResolved() const { return m_resolved; }
    QSize sizeHint() const override { return QSize(24, 24); }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    bool m_resolved = false;
};

// .tg: a tag in the row's tags row, UI 11/16. Plain tags (#color) are ink3
// and toggle the tag filter; Blocking is ink2 with a neg dot; Failed is neg
// with a dot; Accepted in rev N is ink3. The last two are not clickable.
class TagButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Style { Tag, Blocking, Failed, Accepted };
    TagButton(const QString &text, Style style, QWidget *parent = nullptr);
    Style style() const { return m_style; }
    // The tag the filter takes ("color", or ReviewModel::kBlockingTag).
    QString tag() const { return m_tag; }
    void setTag(const QString &tag) { m_tag = tag; }
    QColor textColor() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    Style m_style;
    QString m_tag;
};

// .lk: a fold link ("2 replies", "3 steps", "2 changes"), UI 12/16 (11/16
// in the tags row) ink2, hover ink, with a 14 pt chevron that turns 180°
// in 160 ms when open. Checkable: checked is open.
class ChatExpander final : public QAbstractButton {
    Q_OBJECT
public:
    explicit ChatExpander(QWidget *parent = nullptr);
    // 11 pt in a tags row (16 tall), else 12 pt (20 tall).
    void setSmall(bool small);
    bool isSmall() const { return m_small; }
    // A link with no chevron (Ask again, Copy).
    void setChevron(bool chevron);
    // The chevron's turn, 0 (closed) to 1 (open), as drawn now.
    qreal turn() const { return m_turn; }
    // Open or closed at once, without toggled() (a rebuild restoring it).
    void setOpenQuietly(bool open);
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    bool m_small = false;
    bool m_chevron = true;
    qreal m_turn = 0;
    QVariantAnimation *m_rotate = nullptr;
};

// .sug: a request to try, 28 tall, radius 6: a 14 pt g-cdr in ink3, the
// words UI 12/16 ink (timecodes in mono ink2); a raised wash under the
// pointer. The wash reaches 8 past the words on the left (margin 0 −8).
class SuggestionButton final : public QAbstractButton {
    Q_OBJECT
public:
    explicit SuggestionButton(const QString &text, QWidget *parent = nullptr);
    // What it asks for when chosen (the label by default).
    QString request() const { return m_request; }
    void setRequest(const QString &request) { m_request = request; }
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return QSize(80, 28); }
    // Where its words go.
    QRect textRect() const;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    QString m_request;
};

// The three 3 pt drafting dots (ink3, gap 3), pulsing in turn unless motion
// is reduced.
class DraftDots final : public QWidget {
    Q_OBJECT
public:
    explicit DraftDots(QWidget *parent = nullptr);
    QSize sizeHint() const override { return QSize(2 + 3 * 3 + 2 * 3, 20); }

protected:
    void paintEvent(QPaintEvent *) override;
    void showEvent(QShowEvent *) override;
    void hideEvent(QHideEvent *) override;

private:
    QTimer *m_timer = nullptr;
    int m_phase = 0;
};

// The drafting skeleton: 8 pt lines, radius 4, raised, 8 apart, at 88% and 56%.
class DraftSkeleton final : public QWidget {
    Q_OBJECT
public:
    explicit DraftSkeleton(QWidget *parent = nullptr);
    QSize sizeHint() const override { return QSize(120, 8 + 8 + 8); }

protected:
    void paintEvent(QPaintEvent *) override;
};

// One turn or note of the review as a row. A row is built empty and filled
// with setters, from a review record or from the agent turn in progress.
class CommentRow final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    // User, Kerf and Error are the local rows of the agent turn in progress
    // (the reader's request, Kerf's answer, a failure); Record is a review
    // log comment; Drafting is Kerf at work.
    enum class Kind { User, Kerf, Error, Record, Drafting };
    // The background layer reaches this far past the content.
    static constexpr int kBleedX = 8;
    static constexpr int kBleedY = 6;
    static constexpr int kAvatar = 24;
    static constexpr int kGap = 12;
    // The body column starts this far into the content.
    static constexpr int kBodyLeft = kAvatar + kGap;

    // A local row: the reader's ("You"), Kerf's answer or a failure.
    CommentRow(Kind kind, const QString &text, const QDateTime &time, QWidget *parent = nullptr);
    ~CommentRow() override;

    Kind kind() const { return m_kind; }
    // The review log id (a record), the client id of an optimistic note, or empty.
    QString commentId() const { return m_commentId; }
    void setCommentId(const QString &id) { m_commentId = id; }
    QString threadId() const { return m_threadId; }
    void setThreadId(const QString &id) { m_threadId = id; }
    bool isReply() const { return m_reply; }
    // A reply sits under its root, its hover actions at right −4.
    void setReply(bool reply);

    // The avatar: an author kind (person, mcp, kerf, qc) and initials.
    void setAuthor(const QString &kind, const QString &initials);
    QString name() const;
    void setName(const QString &name);
    void setRole(const QString &role);
    // The TC link; NaN hides it.
    void setMark(double at, double duration, double fps);
    // The time label and its tooltip.
    void setTime(const QDateTime &time, qint64 revision = -1);
    QDateTime time() const { return m_time; }
    void refreshTime(const QDateTime &now);
    // "Couldn't edit" in neg on the name line.
    void setError(bool error);
    bool isError() const { return m_error; }
    // The 14 pt annotation glyph (Has a drawing).
    void setDrawing(bool drawing);

    // The text, plain; shown as rich text (timecodes as links, @mentions).
    QString text() const { return m_text; }
    void setText(const QString &text);
    void appendText(const QString &text);
    void setFps(double fps);
    // Resolved text reads ink2.
    void setMuted(bool muted);

    // The ring at the end of the name line (roots only).
    void setRing(bool shown, bool resolved);
    // Tags, in order: Failed, Blocking, user tags, Accepted in rev N.
    TagButton *addTag(const QString &text, TagButton::Style style, const QString &tag = {});
    QList<TagButton *> tags() const { return m_tags; }
    // The "N replies" fold link in the tags row; null when there are none.
    ChatExpander *setReplyFold(int replies, bool open);
    ChatExpander *replyFold() const { return m_replyFold; }
    // Links in the tags row with no chevron (an error reply's Ask again, Copy).
    ChatExpander *addLink(const QString &text);
    QList<ChatExpander *> links() const { return m_links; }

    // Steps Kerf took for this turn, listed under "N steps".
    void addReply(const QString &text);
    void setSteps(const QStringList &steps);
    QStringList replies() const { return m_steps; }
    bool repliesExpanded() const;
    void setRepliesExpanded(bool expanded);
    ChatExpander *expander() const { return m_stepsLink; }
    QWidget *replyList() const { return m_stepsList; }

    // The welcome's "Try asking" rows.
    void setSuggestions(const QList<QPair<QString, QString>> &labelAndRequest);
    QList<SuggestionButton *> suggestions() const { return m_suggestions; }

    // Kerf at work: "Kerf is drafting", the dots, the latest status line,
    // Stop; the skeleton until text streams in. Stopped: "Kerf · stopped"
    // and Ask again.
    void setDrafting(bool drafting);
    bool isDrafting() const { return m_kind == Kind::Drafting; }
    void setDraftMeta(const QString &meta);
    QString draftMeta() const;
    void setStopped(bool stopped);
    bool isStopped() const { return m_stopped; }
    // Finishes a drafting row as Kerf's answer (a Kerf row).
    void finish();
    ui::GhostButton *stopButton() const { return m_stop; }

    // Widgets in the body column below the text (steps and the like) and
    // across the row from the avatar's edge (a proposal card in a reply).
    void addBodyWidget(QWidget *widget);
    void addWideWidget(QWidget *widget);

    // The background states: hover, selected (.sel), the current thread's
    // root (.cursel), and the flash after navigation (600 ms).
    void setSelected(bool selected);
    bool isSelected() const { return m_selected; }
    void setCurrent(bool current);
    bool isCurrent() const { return m_current; }
    void flash();
    bool isFlashing() const { return m_flashing; }
    // The colour of the background layer as drawn now (transparent at rest).
    QColor backgroundColor() const;
    // The background layer: the row's rect (content plus the bleed).
    QRect backgroundRect() const { return rect(); }

    // Hover actions: Reply, Resolve (not on one's own comments), More.
    void setActions(bool reply, bool resolve, bool more);
    QWidget *actionsBar() const { return m_actions; }
    ui::IconButton *replyAction() const { return m_replyAction; }
    ui::IconButton *resolveAction() const { return m_resolveAction; }
    ui::IconButton *moreAction() const { return m_moreAction; }
    void setActionsShown(bool shown);

    ui::Avatar *avatar() const { return m_avatar; }
    QLabel *nameLabel() const { return m_nameLabel; }
    QLabel *roleLabel() const { return m_roleLabel; }
    QLabel *timeLabel() const { return m_timeLabel; }
    QLabel *errorLabel() const { return m_errorLabel; }
    QWidget *drawingFlag() const { return m_drawingFlag; }
    TimecodeLink *timecodeLink() const { return m_tc; }
    ResolveRing *ring() const { return m_ring; }
    QLabel *body() const { return m_body; }
    QWidget *tagsRow() const { return m_tagsRow; }
    DraftDots *dots() const { return m_dots; }
    DraftSkeleton *skeleton() const { return m_skeleton; }
    // The body column, in the row's coordinates.
    QRect bodyRect() const;

signals:
    // A timecode was clicked, the TC link or one inline in the text.
    void seekRequested(double seconds);
    void ringClicked();
    void replyClicked();
    void moreClicked();
    void tagClicked(const QString &tag);
    void foldToggled(bool open);
    void stepsToggled(bool open);
    void suggestionChosen(const QString &request);
    void stopClicked();
    void askAgainClicked();
    // A press on the row itself (not on a control).
    void pressed();
    // The pointer came onto the row (true) or left it (false).
    void hoverChanged(bool hovered);
    // Deprecated: "Ask again" on one of the reader's local rows (W3 removes it).
    void askAgain(const QString &text);

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void enterEvent(QEnterEvent *) override;
    void leaveEvent(QEvent *) override;
    void mousePressEvent(QMouseEvent *event) override;

private:
    void build();
    void updateBody();
    void updateSteps();
    void placeOverlays();
    void updateNameLine();
    void fadeTo(qreal target);

    Kind m_kind;
    QString m_commentId;
    QString m_threadId;
    QString m_text;
    QString m_name;
    QDateTime m_time;
    qint64 m_revision = -1;
    QStringList m_steps;
    double m_fps = 25.0;
    bool m_reply = false;
    bool m_error = false;
    bool m_muted = false;
    bool m_stopped = false;
    bool m_selected = false;
    bool m_current = false;
    bool m_flashing = false;
    bool m_hovered = false;
    bool m_hasActions = false;
    qreal m_hover = 0;
    QVariantAnimation *m_fade = nullptr;

    ui::Avatar *m_avatar = nullptr;
    QVBoxLayout *m_bodyLayout = nullptr;
    QVBoxLayout *m_wideLayout = nullptr;
    QWidget *m_nameLine = nullptr;
    QHBoxLayout *m_nameLayout = nullptr;
    QLabel *m_nameLabel = nullptr;
    QLabel *m_roleLabel = nullptr;
    TimecodeLink *m_tc = nullptr;
    QLabel *m_errorLabel = nullptr;
    QWidget *m_drawingFlag = nullptr;
    QLabel *m_timeLabel = nullptr;
    DraftDots *m_dots = nullptr;
    QLabel *m_metaLabel = nullptr;
    ui::GhostButton *m_stop = nullptr;
    QWidget *m_ringRoom = nullptr;
    ResolveRing *m_ring = nullptr;
    QLabel *m_body = nullptr;
    QWidget *m_tagsRow = nullptr;
    QHBoxLayout *m_tagsLayout = nullptr;
    QList<TagButton *> m_tags;
    ChatExpander *m_replyFold = nullptr;
    QList<ChatExpander *> m_links;
    ChatExpander *m_stepsLink = nullptr;
    QWidget *m_stepsList = nullptr;
    QVBoxLayout *m_stepsLayout = nullptr;
    QWidget *m_suggestionBox = nullptr;
    QLabel *m_tryAsking = nullptr;
    QList<SuggestionButton *> m_suggestions;
    DraftSkeleton *m_skeleton = nullptr;
    QWidget *m_actions = nullptr;
    ui::IconButton *m_replyAction = nullptr;
    ui::IconButton *m_resolveAction = nullptr;
    ui::IconButton *m_moreAction = nullptr;
    QTimer *m_flashTimer = nullptr;
};

// A thread of the list (.th): its root row, then its replies 36 in. A 1 pt
// line runs across its top (16 from either side) when another thread is
// above it.
class ThreadWidget final : public QWidget {
    Q_OBJECT
public:
    ThreadWidget(const QString &id, bool first, QWidget *parent = nullptr);
    QString threadId() const { return m_id; }
    bool isFirst() const { return m_first; }
    void setRoot(CommentRow *row);
    CommentRow *root() const { return m_root; }
    void addReply(CommentRow *row);
    QList<CommentRow *> replyRows() const;
    QWidget *replies() const { return m_replies; }
    void setRepliesShown(bool shown);
    bool repliesShown() const;
    // Every row, the root first.
    QList<CommentRow *> rows() const;

protected:
    void paintEvent(QPaintEvent *) override;

private:
    QString m_id;
    bool m_first;
    CommentRow *m_root = nullptr;
    QWidget *m_replies = nullptr;
    QVBoxLayout *m_replyLayout = nullptr;
};

} // namespace kerf
