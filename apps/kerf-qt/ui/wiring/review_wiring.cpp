#include "review_wiring.h"

namespace kerf {

ReviewWiring::ReviewWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("reviewWiring"));
}

} // namespace kerf
