#pragma once

// The conversation's wiring: the conversation panel, the conversations kept
// beside the project, the proposals, Ask and next steps. A friend of
// MainWindow that holds the region's own state, so the region changes the
// window's behaviour only here and in main_window_chat.cpp.
//
// It owns the ProposalModel every region reads (EditorBus::proposals()) and
// the ConversationModel the panel and the sidebar read, feeds them the
// bridge's state and conversation events, and turns what the panel asks for
// into bridge commands:
//   post           → agent {text, focus, conversation?, at?}
//   accept/reject  → accept/reject {proposalId, revision}
//   Preview        → preview {proposalId, start, duration} / preview_exit
//   conversations  → conversations {action: list | open | archive | rename | delete, id, …}
//
// The member functions that reach into MainWindow are defined in
// main_window_chat.cpp, which the window's executables compile.

#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>

#include <limits>

namespace kerf {

class ChatPanel;
class ConversationModel;
class EditorBus;
class MainWindow;
class ProposalModel;

class ChatWiring final : public QObject {
    Q_OBJECT
public:
    ChatWiring(MainWindow *window, EditorBus *bus);
    ~ChatWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }
    ProposalModel *model() const { return m_model; }
    ConversationModel *conversations() const { return m_conversations; }
    ChatPanel *panel() const;

    // Connects the panel, the models and the bus (MainWindow::buildChat).
    void attach(ChatPanel *panel);

    // What the composer holds, sent to Kerf in the open conversation.
    bool post();
    // What the composer holds while Kerf works: it waits, and goes when the
    // turn ends well; a turn stopped or failed puts it back in the composer
    // to send or change.
    bool queue();
    void sendQueued(bool ended);
    // An agent request in the open conversation (a new one when none is
    // open), asked at `at` (NaN: not stamped).
    bool sendTurn(const QString &text, const QJsonObject &focus, const QString &focusLabel,
                  double at = std::numeric_limits<double>::quiet_NaN());

    // The conversations: open one (empty starts a new one), archive or bring
    // back, rename, delete. Each is sent to the bridge, whose next list has
    // the last word.
    void openConversation(const QString &id);
    bool archiveConversation(const QString &id, bool archived);
    bool renameConversation(const QString &id, const QString &title);
    bool deleteConversation(const QString &id);

    bool accept(const QString &proposalId);
    bool reject(const QString &proposalId);
    // Bring a stale proposal onto the edit as it is, as its next version.
    bool forward(const QString &proposalId);
    bool preview(const QString &proposalId, bool on);
    bool previewRange(const QString &proposalId, double start, double duration);
    // The pinned proposal is the one ⌘↩ and Esc act on (MainWindow::m_proposalId).
    void syncPinnedProposal();
    // A bridge event, after the window handled it.
    void handleEvent(const QString &type, const QJsonObject &event);

private:
    bool sendConversations(const QJsonObject &command);

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    ProposalModel *m_model = nullptr;
    ConversationModel *m_conversations = nullptr;
    QPointer<ChatPanel> m_panel;
    // The request waiting for the turn to end, and the playhead it was asked at.
    QString m_queued;
    double m_queuedAt = std::numeric_limits<double>::quiet_NaN();
    // The turn in progress failed or was stopped.
    bool m_turnTroubled = false;
};

} // namespace kerf
