#include "timeline_wiring.h"

namespace kerf {

TimelineWiring::TimelineWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("timelineWiring"));
}

} // namespace kerf
