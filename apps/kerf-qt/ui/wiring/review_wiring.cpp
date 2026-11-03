#include "review_wiring.h"

#include "chat_panel.h"
#include "review_model.h"

#include <QDateTime>
#include <QUuid>

namespace kerf {

// The members that reach into MainWindow (attach, post, the commands) are
// in main_window_review.cpp.

ReviewWiring::ReviewWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("reviewWiring"));
    m_model = new ReviewModel(this);
    m_model->setObjectName(QStringLiteral("reviewModel"));
}

ReviewWiring::~ReviewWiring() = default;

ChatPanel *ReviewWiring::panel() const { return m_panel.data(); }

QString ReviewWiring::nextClientId() {
    // A client id makes a retried comment_add harmless (bridge.h).
    m_lastClientId = QStringLiteral("c-qt-%1-%2")
                         .arg(QUuid::createUuid().toString(QUuid::WithoutBraces).left(8))
                         .arg(++m_serial);
    return m_lastClientId;
}

} // namespace kerf
