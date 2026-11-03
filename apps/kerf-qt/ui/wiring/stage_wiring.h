#pragma once

// The stage region's wiring: the Program stage: device, picture overlays, transport, rail and the program strip. A friend of MainWindow that
// holds the region's own state, so a region changes the window's behaviour
// only here and in main_window_stage.cpp.
//
// Created by Q-SKELETON (W1) near empty; Q-STAGE fills it in (W2).

#include <QObject>

namespace kerf {

class EditorBus;
class MainWindow;

class StageWiring final : public QObject {
    Q_OBJECT
public:
    StageWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

private:
    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
};

} // namespace kerf
