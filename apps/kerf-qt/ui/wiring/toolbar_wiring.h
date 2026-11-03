#pragma once

// The toolbar region's wiring: the toolbar and the status bar. A friend of MainWindow that
// holds the region's own state, so a region changes the window's behaviour
// only here and in main_window_toolbar.cpp and main_window_status.cpp.
//
// Created by Q-SKELETON (W1) near empty; Q-TOOLBAR fills it in (W2).

#include <QObject>

namespace kerf {

class EditorBus;
class MainWindow;

class ToolbarWiring final : public QObject {
    Q_OBJECT
public:
    ToolbarWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

private:
    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
};

} // namespace kerf
