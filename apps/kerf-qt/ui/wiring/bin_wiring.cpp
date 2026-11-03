#include "bin_wiring.h"

namespace kerf {

BinWiring::BinWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("binWiring"));
}

} // namespace kerf
