#include "inspector_wiring.h"

namespace kerf {

InspectorWiring::InspectorWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("inspectorWiring"));
}

} // namespace kerf
