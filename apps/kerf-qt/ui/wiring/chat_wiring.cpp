#include "chat_wiring.h"

#include "chat_panel.h"
#include "proposal_model.h"
#include "ui/conversation_model.h"

namespace kerf {

// The members that reach into MainWindow (attach, post, the commands) are
// in main_window_chat.cpp.

ChatWiring::ChatWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("chatWiring"));
    m_model = new ProposalModel(this);
    m_model->setObjectName(QStringLiteral("proposalModel"));
    m_conversations = new ConversationModel(this);
    m_conversations->setObjectName(QStringLiteral("conversationModel"));
}

ChatWiring::~ChatWiring() = default;

ChatPanel *ChatWiring::panel() const { return m_panel.data(); }

} // namespace kerf
