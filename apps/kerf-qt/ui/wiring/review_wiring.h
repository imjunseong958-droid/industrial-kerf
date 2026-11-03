#pragma once

// The review region's wiring: the comments panel (the agent chat), proposals, Ask and next steps. A friend of MainWindow that
// holds the region's own state, so a region changes the window's behaviour
// only here and in main_window_review.cpp.
//
// Created by Q-SKELETON (W1) near empty; Q-COMMENTS fills it in (W2).

#include <QObject>

namespace kerf {

class EditorBus;
class MainWindow;

class ReviewWiring final : public QObject {
    Q_OBJECT
public:
    ReviewWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

private:
    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
};

} // namespace kerf
