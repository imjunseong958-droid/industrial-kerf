#include "source_wiring.h"

namespace kerf {

SourceWiring::SourceWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("sourceWiring"));
}

} // namespace kerf
