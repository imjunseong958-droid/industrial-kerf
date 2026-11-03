#include "toolbar_wiring.h"

namespace kerf {

ToolbarWiring::ToolbarWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("toolbarWiring"));
}

} // namespace kerf
