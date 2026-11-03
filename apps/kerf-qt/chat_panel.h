#pragma once

// The Comments panel (the agent chat), mockup-spec 4: a 12-radius card
// beside the picture with the header (Comments, Search, Sort, Filter, More,
// Close), the All/Open/Proposals chips, the scroll-aware pin bar, the list
// of threads with their proposal cards, the reply-to bar and the composer.
//
// Everything it lists is the review model's (ui/review_model.h): comment
// threads from kerfd's review log and the snapshot's proposals. The agent
// turn in progress shows as local rows (the reader's request, then Kerf
// drafting) until the review log records the turn as comments.
//
// The panel turns clicks into requests (signals); ReviewWiring and
// main_window_review.cpp send them to kerfd.

#include "test_friends.h"
#include "comment_row.h"
#include "review_model.h"
// RailButton and StageRail moved to ui/stage_rail.h; included here for the files that
// still find them through this header (deprecated, removed in W3).
#include "stage_rail.h"

#include <QAbstractButton>
#include <QDateTime>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QPointer>
#include <QSet>
#include <QWidget>

#include <functional>
#include <limits>

class QComboBox;
class QFrame;
class QHBoxLayout;
class QLabel;
class QLineEdit;
class QMenu;
class QPushButton;
class QScrollArea;
class QTextEdit;
class QTimer;
class QVBoxLayout;
class QVariantAnimation;

namespace kerf {

class Composer;
class ProposalCard;
class ReplyBar;

namespace ui {
class Chip;
class Dot;
class FadeScrollArea;
class IconButton;
} // namespace ui

// One change a proposal's script line makes, in words (MainWindow::
// describeScriptLine): what it does, and how much or where. A proposal card
// lists these when the proposal has no timeline to compare.
struct ProposalChange {
    enum class Glyph { Trim, Remove, Add, Title, Move, Split, Level, Effect, Other };
    Glyph glyph = Glyph::Other;
    QString description;      // "Trim fisher-close.mov"
    QString detail;           // "end −1.5 s"
    QString shortDescription; // "Trim fisher-close": its clip names without their file extension
};

// An icon button of the panel's header (.ib, 24, radius 6, a 16 pt icon in
// ink2; hover raised and ink).
class ChatIconButton final : public QAbstractButton {
    Q_OBJECT
public:
    // Close, the overflow's dots, and the chevrons.
    enum class Glyph { Close, More, Back, Forward };
    ChatIconButton(Glyph glyph, int diameter, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    // Deprecated: a filled disc at rest (W3 removes it).
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

class ChatPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kRadius = 12;
    static constexpr int kHeaderHeight = 40;
    static constexpr int kChipsHeight = 32;
    static constexpr int kPinHeight = 32;
    static constexpr int kSearchHeight = 32;
    static constexpr int kSidePadding = 16;
    static constexpr int kAvatar = CommentRow::kAvatar;
    // The list's padding: 12 over the first thread, 16 under the last.
    static constexpr int kListTop = 12;
    static constexpr int kListBottom = 16;
    // The pin bar shows while the pinned card's footer is not fully in view
    // (its bottom 24 over the list's).
    static constexpr int kPinMargin = 24;
    // Deprecated sizes of the old conversation, still read by tests of
    // other regions (W3 removes them): buttons and chips are 24 now.
    static constexpr int kComposerAvatar = 24;
    static constexpr int kPillHeight = 24;
    static constexpr int kComposerPillHeight = 24;
    static constexpr int kChipHeight = 24;
    static constexpr int kChipRowHeight = 32;
    static constexpr int kProposalChangeLines = 3;
    static constexpr int kProposalDiffLines = 10;
    static constexpr int kNarrowProposal = 360;

    explicit ChatPanel(QWidget *parent = nullptr);
    ~ChatPanel() override;

    // ---- the review --------------------------------------------------------
    void setReview(ReviewModel *model);
    ReviewModel *review() const { return m_model.data(); }
    // The snapshot's timeline and proposal timelines (change rows, versions,
    // the colour preview), and the program's frame rate.
    void setSnapshot(const QJsonObject &snapshot);
    // The program as the window shows it (clips named after their media),
    // for the names the cards give clips and tracks.
    void setNamedTimeline(const QJsonObject &timeline);
    void setFps(double fps);
    double fps() const { return m_fps; }
    void setPlayhead(double seconds);
    void setConnected(bool connected);
    // Words for a script line when a version has no timeline to compare.
    void setScriptDescriber(std::function<ProposalChange(const QString &)> describer) { m_describer = std::move(describer); }
    // kerf://<project>/rev<N>@TC for a comment at `seconds` (NaN: the revision).
    void setReviewLink(std::function<QString(double)> link) { m_reviewLink = std::move(link); }
    QString reviewLink(double seconds) const;
    // Kerf's next steps (agent_next {steps: [{label, request}]}): the
    // welcome's Try asking and the composer's suggestions.
    void setNextSteps(const QJsonArray &steps);
    QJsonArray nextSteps() const { return m_steps; }
    // The latest check's summary rows: failing ones become suggestions.
    void setCheckRows(const QJsonArray &rows);
    // The proposal a running preview shows (empty when none).
    void setPreviewProposal(const QString &proposalId);
    QString previewProposal() const { return m_previewProposal; }
    // An effect_preview answering a colour card: key "before:<id>" or "after:<id>".
    bool setEffectPreview(const QString &key, const QImage &frame);
    // The effect_previews request for a colour card's version: the clip at
    // the playhead with its current effect ("before:<id>") and the proposed
    // one ("after:<id>"); empty when the version is no colour change.
    QJsonObject colourPreviewRequest(const QString &versionId) const;
    // A proposal accepted or rejected from this panel: its card settles to one line.
    void markSettledHere(const QString &proposalId, bool accepted);
    // Whether the editor is in the Review workspace (first-show placement).
    void setReviewWorkspace(bool review) { m_reviewWorkspace = review; }

    // ---- threads -----------------------------------------------------------
    QList<ThreadWidget *> threadWidgets() const;
    ThreadWidget *threadWidget(const QString &threadId) const;
    CommentRow *rowFor(const QString &commentId) const;
    ProposalCard *card(const QString &proposalId) const;
    QList<ProposalCard *> cards() const;
    // Opens the thread: current, scrolled to the top (12 under the chips),
    // its replies unfolded, and its root flashed.
    void openThread(const QString &threadId, bool flash = true);
    // Scrolls the list so the thread's root sits 12 under the list's top.
    void scrollToThread(const QString &threadId);
    // The first-show placement (spec 4.5); runs once.
    void placeFirst();
    bool placed() const { return m_placed; }

    // ---- the agent turn in progress -----------------------------------------
    // The reader's request, as a local row in `threadId` (a reply) or a new
    // thread. Kerf's rows of the turn follow it. A stamped request (`at`
    // finite, not a reply) shows its timecode.
    CommentRow *beginTurn(const QString &text, const QString &threadId,
                          double at = std::numeric_limits<double>::quiet_NaN());
    // The turn's Kerf row: drafting while busy, Kerf's answer once done.
    CommentRow *turnKerfRow(bool create);
    void setTurnBusy(bool busy);
    void setTurnStatus(const QString &meta);
    void stopTurn();
    // The turn ended (done or failed); the review log takes over its rows
    // when it records them.
    void finishTurn();
    // A note posted with comment_add, shown until the review log has it.
    CommentRow *addPendingNote(const QString &clientId, const QString &text, const QString &threadId, double at);
    void failPendingNote(const QString &clientId, const QString &message);
    // Drops local rows the review log now holds.
    void reconcileLocal();
    // The old API: a local row of `kind` in the turn in progress.
    CommentRow *addComment(CommentRow::Kind kind, const QString &text);
    // Every row of the list (not the welcome), in order.
    QList<CommentRow *> rows() const;
    bool isEmpty() const { return rows().isEmpty(); }
    // Every row's name and text, in order, as plain text; steps are not included.
    QString transcript() const;
    // The view keeps to the latest local row unless the reader scrolled away.
    bool followsLatest() const { return m_follow; }
    void refreshTimes();

    // ---- the composer --------------------------------------------------------
    // Replying to a thread ("Replying to Mina Park"); empty ends it.
    // Only a review-log thread can be replied to; any other ends it.
    void setReplyTarget(const QString &threadId);
    QString replyTarget() const { return m_replyTarget; }
    // The version `proposalId`'s card shows: the one chosen in its Versions
    // menu, else the proposal itself.
    QString shownVersion(const QString &proposalId) const;
    // Editing one's own note: the composer holds its text; Post saves it.
    void setEditing(const QString &commentId);
    QString editing() const { return m_editing; }
    void updateComposer(bool busy);
    // The suggestions the composer offers (labels) and what each asks.
    struct Suggestion {
        QString label;
        QString request;
        bool run = false; // runs at once (QC) instead of filling the composer
    };
    QList<Suggestion> suggestions() const { return m_suggestionList; }
    void updateSuggestions();
    // The names @ completes: Kerf, the MCP agents seen, the people.
    QStringList mentionCandidates() const;

    // ---- parts ---------------------------------------------------------------
    QLabel *title() const { return m_title; }
    ui::IconButton *searchButton() const { return m_searchButton; }
    QLineEdit *searchField() const { return m_search; }
    QWidget *searchRow() const { return m_searchRow; }
    ui::IconButton *sortButton() const { return m_sortButton; }
    QLabel *sortLabel() const { return m_sortLabel; }
    ui::IconButton *filterButton() const { return m_filterButton; }
    bool filterDotShown() const;
    ChatIconButton *moreButton() const { return m_more; }
    ChatIconButton *closeButton() const { return m_close; }
    ui::Chip *chipAll() const { return m_chipAll; }
    ui::Chip *chipOpen() const { return m_chipOpen; }
    ui::Chip *chipProposals() const { return m_chipProposals; }
    QFrame *pinBar() const { return m_pinBar; }
    QLabel *pinAuthor() const { return m_pinAuthor; }
    QLabel *pinSummary() const { return m_pinSummary; }
    QLabel *pinCount() const { return m_pinCount; }
    ui::IconButton *pinPrevious() const { return m_pinPrev; }
    ui::IconButton *pinNext() const { return m_pinNext; }
    // Whether the pin bar is (or is opening to be) shown.
    bool pinBarWanted() const { return m_pinWanted; }
    QString pinBarProposal() const;
    QScrollArea *scroll() const;
    ui::FadeScrollArea *list() const { return m_list; }
    QWidget *listContent() const { return m_content; }
    ReplyBar *replyBar() const { return m_replyBar; }
    Composer *composerWidget() const { return m_composer; }
    QFrame *composer() const;
    QTextEdit *input() const;
    QPushButton *sendButton() const;
    QPushButton *stopButton() const;
    CommentRow *welcomeRow() const { return m_welcomeRow; }
    QWidget *welcome() const;
    QList<QAbstractButton *> examples() const;
    QWidget *resizeHandle() const { return m_resizeHandle; }
    // The menus, built as they open (for tests).
    QMenu *buildSortMenu();
    QMenu *buildFilterMenu();
    QMenu *buildMoreMenu();
    QMenu *buildRowMenu(const QString &commentId);

    // ---- deprecated: the old conversation's parts, kept for other regions'
    // code and tests until W3 removes them. None of them shows. ----------------
    QLabel *countLabel() const { return m_count; }
    QWidget *statusRow() const { return m_statusRow; }
    QLabel *statusLabel() const { return m_status; }
    QLabel *activityLabel() const { return m_activity; }
    QWidget *chips() const { return m_chips; }
    ChatIconButton *chipsBack() const { return m_chipBack; }
    ChatIconButton *chipsForward() const { return m_chipForward; }
    QComboBox *modelSelect() const { return m_modelSelect; }
    QPushButton *cancelButton() const { return m_cancel; }
    QFrame *proposal() const { return m_proposal; }
    QLabel *proposalName() const { return m_proposalName; }
    QLabel *proposalTime() const { return m_proposalTime; }
    QLabel *proposalSummary() const { return m_proposalSummary; }
    QTextEdit *proposalDiff() const { return m_proposalDiff; }
    QPushButton *acceptButton() const { return m_accept; }
    QPushButton *rejectButton() const { return m_reject; }
    QFrame *proposalBar() const { return m_pinBar; }
    QLabel *proposalBarSummary() const { return m_pinSummary; }
    QPushButton *barAcceptButton() const { return m_barAccept; }
    QPushButton *barRejectButton() const { return m_barReject; }
    QFrame *listEdge() const { return m_scrollEdge; }
    void setProposalShown(bool shown, const QString &id = QString());
    void setProposalSummary(const QString &summary, const QString &compact = QString(), const QString &count = QString());
    void setProposalChanges(const QList<ProposalChange> &changes);
    bool proposalExpanded() const { return false; }
    void setProposalExpanded(bool) {}
    bool composerExpanded() const { return true; }

signals:
    // A drag on the panel's leading edge asks for this width, and its end
    // for keeping it.
    void widthDragged(int width);
    void widthDragFinished();
    // "Ask again" on a row: the request to send again, in its thread.
    void askAgain(const QString &text);
    void askAgainInThread(const QString &text, const QString &threadId);
    void seekRequested(double seconds);
    // The ring or Resolve: resolve (true) or reopen a thread.
    void resolveRequested(const QString &threadId, bool resolve);
    void acceptRequested(const QString &proposalId);
    void rejectRequested(const QString &proposalId);
    // The Preview checkbox, or an older version chosen while previewing.
    void previewRequested(const QString &proposalId, bool on);
    // A card's Versions menu chose the version it shows.
    void shownVersionChanged(const QString &proposalId, const QString &versionId);
    // A colour card opened without its before/after frames.
    void colourPreviewWanted(const QString &proposalId);
    void postRequested();
    void stopRequested();
    void drawRequested();
    void drawCancelRequested();
    // A suggestion that runs (QC).
    void runRequested(const QString &what);
    // The row menu: "marker", "delete" on a comment.
    void rowAction(const QString &action, const QString &commentId);
    // The More menu: "settings", "activity", "viewAs" (a person id, or empty
    // for oneself), "export" (markers, csv, edl), "model" (a model id).
    void moreAction(const QString &action, const QString &argument);
    void closeRequested();
    void currentThreadChosen(const QString &threadId);
    // The thread of the row under the pointer, or empty when it leaves.
    void threadHovered(const QString &threadId);
    // A row pressed: its comment (a reply's own id), for its drawing.
    void commentSelected(const QString &commentId);

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    void showEvent(QShowEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    struct LocalTurn {
        QString threadId;
        QDateTime started;
        QString text;
        QList<QPointer<CommentRow>> rows;
        QPointer<CommentRow> kerf;
        bool done = false;
        bool youRecorded = false;
    };
    struct ColourChange {
        QString clip;
        QString fx;
        bool hadBefore = false;
        QJsonObject before;
        QJsonObject after;
        QString title;
        QString params;
    };
    ColourChange colourChange(const ReviewModel::Proposal &proposal, const QString &versionId) const;
    struct CardState {
        bool open = false;
        bool openSet = false;
        bool script = false;
        QString shown; // the version shown, empty for the current one
    };

    QWidget *buildHeader();
    QWidget *buildChips();
    QFrame *buildPinBar();
    QWidget *buildSearch();
    void buildDeprecated();
    void scheduleRebuild();
    void syncContent();
    void rebuild();
    CommentRow *buildRow(const ReviewModel::Comment &comment, const ReviewModel::Thread &thread, bool root);
    ProposalCard *buildCard(const ReviewModel::Proposal &proposal, const ReviewModel::Thread &thread);
    QList<ReviewModel::ClipChange> versionChanges(const ReviewModel::Proposal &proposal, const QString &versionId) const;
    void connectRow(CommentRow *row);
    void stashLocalRows();
    void placeLocalRows(bool &first);
    void updateCounts();
    void updateHeader();
    void updateCurrent();
    void updatePinBar();
    void animatePinBar(bool shown);
    void scheduleFollow();
    void setSearchShown(bool shown);
    QString trackName(const QString &trackId) const;
    QString clipName(const QString &clipId) const;
    bool repliesOpenByDefault(const ReviewModel::Thread &thread) const;
    void showRowMenu(CommentRow *row);
    void refillWelcome();

    QPointer<ReviewModel> m_model;
    QJsonObject m_timeline;
    QJsonObject m_named;
    QJsonObject m_proposalTimelines;
    double m_fps = 25.0;
    double m_playhead = 0.0;
    bool m_connected = false;
    bool m_reviewWorkspace = true;
    bool m_placed = false;
    std::function<ProposalChange(const QString &)> m_describer;
    std::function<QString(double)> m_reviewLink;
    QJsonArray m_steps;
    QJsonArray m_checkRows;
    QString m_previewProposal;
    QHash<QString, QImage> m_effectFrames;
    QSet<QString> m_colourRequested;
    QHash<QString, bool> m_settledHere;
    QHash<QString, QDateTime> m_settledAt;
    QHash<QString, CardState> m_cardState;
    QHash<QString, bool> m_foldState; // thread id → replies open
    QSet<QString> m_stepsOpen;
    QString m_selectedComment;
    QString m_replyTarget;
    QString m_editing;
    QList<Suggestion> m_suggestionList;
    QList<LocalTurn> m_turns;
    QHash<QString, QPointer<CommentRow>> m_pendingNotes; // client id → row
    QHash<QString, QString> m_pendingThreads;            // client id → thread id
    int m_pinIndex = 0;
    bool m_pinWanted = false;
    bool m_follow = true;
    bool m_rebuildQueued = false;

    QLabel *m_title = nullptr;
    ui::IconButton *m_searchButton = nullptr;
    ui::IconButton *m_sortButton = nullptr;
    QLabel *m_sortLabel = nullptr;
    ui::IconButton *m_filterButton = nullptr;
    ui::Dot *m_filterDot = nullptr;
    ChatIconButton *m_more = nullptr;
    ChatIconButton *m_close = nullptr;
    QWidget *m_searchRow = nullptr;
    QLineEdit *m_search = nullptr;
    ui::Chip *m_chipAll = nullptr;
    ui::Chip *m_chipOpen = nullptr;
    ui::Chip *m_chipProposals = nullptr;
    QFrame *m_pinBar = nullptr;
    QLabel *m_pinAuthor = nullptr;
    QLabel *m_pinSummary = nullptr;
    QLabel *m_pinCount = nullptr;
    ui::IconButton *m_pinPrev = nullptr;
    ui::IconButton *m_pinNext = nullptr;
    QVariantAnimation *m_pinGrow = nullptr;
    ui::FadeScrollArea *m_list = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_listLayout = nullptr;
    QList<ThreadWidget *> m_threads;
    QWidget *m_stash = nullptr;
    CommentRow *m_welcomeRow = nullptr;
    ReplyBar *m_replyBar = nullptr;
    Composer *m_composer = nullptr;
    QWidget *m_resizeHandle = nullptr;
    QTimer *m_clock = nullptr;
    int m_dragFromX = 0;
    int m_dragFromWidth = 0;
    QDateTime m_welcomeSince;
    QLabel *m_welcomeTime = nullptr;

    // Deprecated parts (hidden).
    QLabel *m_count = nullptr;
    QWidget *m_statusRow = nullptr;
    QLabel *m_status = nullptr;
    QLabel *m_activity = nullptr;
    QWidget *m_chips = nullptr;
    ChatIconButton *m_chipBack = nullptr;
    ChatIconButton *m_chipForward = nullptr;
    QComboBox *m_modelSelect = nullptr;
    QPushButton *m_cancel = nullptr;
    QFrame *m_proposal = nullptr;
    QLabel *m_proposalName = nullptr;
    QLabel *m_proposalTime = nullptr;
    QDateTime m_proposalSince;
    QString m_proposalId;
    QLabel *m_proposalSummary = nullptr;
    QTextEdit *m_proposalDiff = nullptr;
    QPushButton *m_accept = nullptr;
    QPushButton *m_reject = nullptr;
    QPushButton *m_barAccept = nullptr;
    QPushButton *m_barReject = nullptr;
    QFrame *m_scrollEdge = nullptr;
};

// The time a comment shows: "now", "4 min ago", "2 h ago", then the date
// (the old conversation's form; rows use review::age).
QString relativeTime(const QDateTime &then, const QDateTime &now);

} // namespace kerf
