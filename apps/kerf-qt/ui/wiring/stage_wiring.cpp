#include "stage_wiring.h"

namespace kerf {

StageWiring::StageWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("stageWiring"));
}

} // namespace kerf
