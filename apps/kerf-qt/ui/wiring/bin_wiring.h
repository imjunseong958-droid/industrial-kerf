#pragma once

// The bin region's wiring: the media bin and the Effects/Titles/Presets browser. A friend of MainWindow that
// holds the region's own state, so a region changes the window's behaviour
// only here and in main_window_bin.cpp.
//
// Created by Q-SKELETON (W1) near empty; Q-BINBROWSER fills it in (W2).

#include <QObject>

namespace kerf {

class EditorBus;
class MainWindow;

class BinWiring final : public QObject {
    Q_OBJECT
public:
    BinWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

private:
    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
};

} // namespace kerf
