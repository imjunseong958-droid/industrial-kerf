#pragma once

// The conversation panel: one conversation with Kerf, as a coding agent's
// chat reads, top to bottom in the order it was said.
//
//   header      the conversation's title (double-click renames it) and ⋯:
//               Rename, Archive, Delete, Agent settings, Model, Activity
//   list        what was said (ui/chat_messages.h): the person's requests as
//               bubbles at the right, Kerf's answers flush left with their
//               work logs and the cards of what they proposed; a new
//               conversation shows the welcome and what to try
//   composer    what to ask next, with the playhead's stamp, Send and Stop
//
// What it lists comes from two models: the conversation as kept beside the
// project (ConversationModel, from the bridge), and the proposals of the
// snapshot (ProposalModel), whose cards sit under the answer that staged
// them. The turn in progress shows as live messages (the request, then
// Kerf at work) until the bridge keeps the turn and sends the conversation
// again.
//
// The panel turns clicks into requests (signals); ChatWiring sends them.

#include "test_friends.h"
#include "chat_parts.h"
#include "composer.h"
#include "proposal_card.h"
#include "proposal_model.h"
#include "stage_motion.h"
#include "ui/chat_messages.h"
#include "ui/conversation_model.h"

#include <QAbstractButton>
#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QPointer>
#include <QWidget>

#include <functional>
#include <limits>

class QAction;
class QLabel;
class QLineEdit;
class QMenu;
class QPushButton;
class QScrollArea;
class QTextEdit;
class QVBoxLayout;

namespace kerf {

class KerfMessage;
class ProposalCard;
class SuggestionButton;
class YouMessage;

namespace ui {
class FadeScrollArea;
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
    // Close and the overflow's dots.
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

class ChatPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeaderHeight = 40;
    static constexpr int kSidePadding = 22;
    // The list's padding: 22 over the first message, 16 under the last.
    static constexpr int kListTop = 22;
    static constexpr int kListBottom = 16;
    // Between one message and the next; between a request and its answer.
    static constexpr int kGap = 18;

    explicit ChatPanel(QWidget *parent = nullptr);
    ~ChatPanel() override;

    // ---- what it shows ----------------------------------------------------
    void setConversations(ConversationModel *model);
    ConversationModel *conversations() const { return m_conversations.data(); }
    void setProposals(ProposalModel *model);
    ProposalModel *proposals() const { return m_model.data(); }
    // The snapshot's timeline and proposal timelines (change rows, versions).
    void setSnapshot(const QJsonObject &snapshot);
    // The program as the window shows it (clips named after their media),
    // for the names the cards give clips and tracks.
    void setNamedTimeline(const QJsonObject &timeline);
    void setFps(double fps);
    double fps() const { return m_fps; }
    void setConnected(bool connected);
    // Words for a script line when a version has no timeline to compare.
    void setScriptDescriber(std::function<ProposalChange(const QString &)> describer) { m_describer = std::move(describer); }
    // Kerf's next steps (agent_next {steps: [{label, request}]}): the
    // welcome's Try asking and the composer's suggestions.
    void setNextSteps(const QJsonArray &steps);
    QJsonArray nextSteps() const { return m_steps; }
    // The models the More menu offers under Model (only with more than one,
    // and while `choosable`: Kerf Cloud picks its own), and the one each
    // request names, shown at Agent settings….
    void setModels(const QStringList &models, const QString &current, bool choosable);
    QStringList models() const { return m_models; }
    QString model() const { return m_currentModel; }
    // How far Kerf goes without asking: the composer's chip, and what the
    // welcome says an edit comes back as.
    void setPermissionMode(PermissionMode mode);
    PermissionMode permissionMode() const;
    // How full Kerf's context is in `conversation` (empty: the one a turn just
    // started), as the turn in progress last said. The composer's meter shows
    // it while that conversation is the open one.
    void setContextUse(const QString &conversation, qint64 used, qint64 limit, bool tokens);
    // The latest check's summary rows: failing ones become suggestions.
    void setCheckRows(const QJsonArray &rows);
    // The proposal a running preview shows (empty when none).
    void setPreviewProposal(const QString &proposalId);
    QString previewProposal() const { return m_previewProposal; }
    // A proposal accepted or rejected from this panel: its card settles to one line.
    void markSettledHere(const QString &proposalId, bool accepted);
    // The picture of the cut a proposal (or its version) makes, for its card;
    // an empty one when it has none to show.
    void setPictureSource(std::function<CutPicture(const QString &)> source) { m_pictureSource = std::move(source); }
    // The posters may have come since the cards were made: their pictures again.
    void refreshPictures();
    // The proposal whose cut the stage shows, opened from its card; empty when none.
    void setStageProposal(const QString &proposalId);
    QString stageProposal() const { return m_stageProposal; }
    // Lays the list out for the panel's size now, not when its posted requests
    // come: where things will be once a move has settled.
    void layOutNow() { syncContent(); }

    // ---- the turn in progress ---------------------------------------------
    // The person's request, shown at once; Kerf at work under it. `at` is the
    // playhead it was asked at, NaN when it was not stamped.
    void beginTurn(const QString &text, double at = std::numeric_limits<double>::quiet_NaN());
    bool turnLive() const { return m_live.active; }
    // Kerf's answer as it streams, and whole.
    void appendAnswer(const QString &delta);
    void setAnswer(const QString &text);
    // A finished tool call or status: a step of the work log.
    void addStep(const QString &step);
    // What Kerf is doing now, beside "Working".
    void setTurnStatus(const QString &status);
    void setTurnBusy(bool busy);
    // The turn ended: answered, failed (with what went wrong) or stopped.
    // Its messages stay until the bridge sends the conversation with it kept.
    void finishTurn();
    void failTurn(const QString &message);
    void stopTurn();
    // The live Kerf message; null when there is no turn.
    KerfMessage *liveMessage() const { return m_live.kerf.data(); }

    // ---- the list ----------------------------------------------------------
    // Every message, in order (not the welcome).
    QList<QWidget *> messages() const;
    QList<YouMessage *> requests() const;
    QList<KerfMessage *> answers() const;
    bool isEmpty() const { return messages().isEmpty(); }
    // Every message's speaker and text, in order, as plain text.
    QString transcript() const;
    ProposalCard *card(const QString &proposalId) const;
    QList<ProposalCard *> cards() const;
    // The view keeps to the latest message unless the reader scrolled away.
    bool followsLatest() const { return m_follow; }
    // The footage Kerf chooses from, under the latest message; none takes it away.
    void setFootage(const QList<FootageTile> &tiles);
    FootageSheet *footage() const { return m_footage; }
    // The first cut Kerf is making now: its card is in Kerf's live message, at
    // work, until it is done; then it is the proposal's card, which shows it is
    // done. Empty when none.
    void setBuildingProposal(const QString &proposalId);
    QString buildingProposal() const { return m_building; }
    ProposalCard *buildingCard() const { return m_buildingCard.data(); }
    // The version `proposalId`'s card shows: the one chosen in its Versions
    // menu, else the proposal itself.
    QString shownVersion(const QString &proposalId) const;
    // The registry's proposal.accept: every card's Accept tip names its key.
    void setAcceptAction(QAction *action);

    // ---- the composer ------------------------------------------------------
    void updateComposer(bool busy);
    struct Suggestion {
        QString label;
        QString request;
        bool run = false; // runs at once (QC) instead of filling the composer
    };
    QList<Suggestion> suggestions() const { return m_suggestionList; }
    void updateSuggestions();
    // ⇧⌘J: the keys in the composer.
    void focusComposer();

    // ---- parts -------------------------------------------------------------
    QLabel *title() const { return m_title; }
    QLineEdit *titleEditor() const { return m_titleEdit; }
    // Renames the conversation in place: the title becomes a field.
    void startRename();
    ChatIconButton *moreButton() const { return m_more; }
    QScrollArea *scroll() const;
    ui::FadeScrollArea *list() const { return m_list; }
    QWidget *listContent() const { return m_content; }
    Composer *composerWidget() const { return m_composer; }
    QFrame *composer() const;
    QTextEdit *input() const;
    QPushButton *sendButton() const;
    QPushButton *stopButton() const;
    QWidget *welcome() const { return m_welcome; }
    QList<QAbstractButton *> examples() const;
    // The More menu, built as it opens (for tests).
    QMenu *buildMoreMenu();

signals:
    // Esc reached the composer with nothing of its own to stop or cancel.
    void escapeUnused();
    // Ask again on a failed or stopped answer: the request to send again.
    void askAgain(const QString &text);
    void seekRequested(double seconds);
    void acceptRequested(const QString &proposalId);
    void rejectRequested(const QString &proposalId);
    // The Preview checkbox, or an older version chosen while previewing.
    void previewRequested(const QString &proposalId, bool on);
    // A card's Watch what changed.
    void changedPartsRequested(const QString &proposalId);
    // A card's picture was pressed: the stage, for that cut.
    void pictureRequested(const QString &proposalId);
    // A card's Versions menu chose the version it shows.
    void shownVersionChanged(const QString &proposalId, const QString &versionId);
    void postRequested();
    void stopRequested();
    // A suggestion that runs (QC).
    void runRequested(const QString &what);
    // The open conversation: "rename" (with its new title), "archive", "delete".
    void conversationAction(const QString &action, const QString &argument);
    // The More menu: "settings", "activity", "model" (a model id).
    void moreAction(const QString &action, const QString &argument);
    // The composer's chip, or Shift-Tab, chose how far Kerf may go.
    void permissionModeChosen(kerf::PermissionMode mode);

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    struct LiveTurn {
        bool active = false;   // Kerf is at work on it
        bool finished = false; // done, failed or stopped; not yet kept
        QString conversation;  // where it was asked (empty: a new one)
        qsizetype base = 0;    // how many messages it held then: the turn's request is kept after them
        QString text;
        double at = std::numeric_limits<double>::quiet_NaN();
        QPointer<YouMessage> you;
        QPointer<KerfMessage> kerf;
    };
    struct ColourChange {
        QString clip;
        QString fx;
        QString title;
        QString params;
    };
    struct CardState {
        bool open = false;
        bool openSet = false;
        bool script = false;
        QString shown; // the version shown, empty for the current one
    };

    QWidget *buildHeader();
    QWidget *buildWelcome();
    void refillWelcome();
    void updateHeader();
    void updateWelcome();
    void updateContextMeter();
    // The faces chosen in Settings changed: what is said is set in them again.
    void refreshFaces();
    // Lays the list out again from the conversation. Messages it already shows
    // (by item id) are kept; only new ones are made.
    void rebuild();
    void scheduleRebuild();
    // The cards again, from the proposals.
    void refreshCards();
    void syncContent();
    void scheduleFollow();
    void dropLive();
    KerfMessage *ensureLiveKerf();
    YouMessage *makeYou(const QString &text, double at);
    KerfMessage *makeKerf();
    void insertMessage(QWidget *message);
    ProposalCard *buildCard(const ProposalModel::Proposal &proposal);
    // Proposals no answer here staged (an agent over MCP): shown at the end.
    QStringList strayProposals() const;
    QList<ProposalModel::ClipChange> versionChanges(const ProposalModel::Proposal &proposal, const QString &versionId) const;
    ColourChange colourChange(const ProposalModel::Proposal &proposal, const QString &versionId) const;
    QString trackName(const QString &trackId) const;
    QString clipName(const QString &clipId) const;
    bool onStage(const ProposalModel::Proposal &proposal) const;
    void commitRename();

    QPointer<ConversationModel> m_conversations;
    QPointer<ProposalModel> m_model;
    QPointer<QAction> m_acceptAction;
    QJsonObject m_timeline;
    QJsonObject m_named;
    QJsonObject m_proposalTimelines;
    double m_fps = 25.0;
    bool m_connected = false;
    std::function<ProposalChange(const QString &)> m_describer;
    std::function<CutPicture(const QString &)> m_pictureSource;
    QString m_stageProposal;
    QJsonArray m_steps;
    QStringList m_models;
    QString m_currentModel;
    bool m_modelsChoosable = false;
    QJsonArray m_checkRows;
    QString m_previewProposal;
    QHash<QString, bool> m_settledHere;
    QHash<QString, QDateTime> m_settledAt;
    QHash<QString, CardState> m_cardState;
    QList<Suggestion> m_suggestionList;
    LiveTurn m_live;
    bool m_follow = true;
    bool m_rebuildQueued = false;

    QLabel *m_title = nullptr;
    QLineEdit *m_titleEdit = nullptr;
    ChatIconButton *m_more = nullptr;
    ui::FadeScrollArea *m_list = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_listLayout = nullptr;
    QWidget *m_welcome = nullptr;
    QLabel *m_welcomeTitle = nullptr;
    QLabel *m_welcomeText = nullptr;
    // The latest reading of Kerf's context, and the conversation it is of.
    struct ContextUse {
        QString conversation;
        qint64 used = 0;
        qint64 limit = 0;
        bool tokens = false;
    } m_context;
    QWidget *m_examples = nullptr;
    QVBoxLayout *m_examplesLayout = nullptr;
    QList<SuggestionButton *> m_exampleButtons;
    // The messages of the kept conversation, by item id, in order.
    QString m_shownConversation;
    QList<QPair<QString, QPointer<QWidget>>> m_shown;
    // Cards of proposals no answer here staged, under the last message.
    QPointer<KerfMessage> m_strays;
    FootageSheet *m_footage = nullptr;
    QString m_building;
    QPointer<ProposalCard> m_buildingCard;
    // The cut Kerf last finished, and when.
    QString m_justBuilt;
    qint64 m_builtAt = -1;
    Composer *m_composer = nullptr;
};

} // namespace kerf
