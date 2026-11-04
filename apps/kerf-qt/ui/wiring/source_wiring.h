#pragma once

// The source region's wiring (Q-SOURCE): the Source viewers, the stage
// column's (m_stageSource) and the Inspector's compact Source pane
// (m_sourceMonitor), which share the one SourceState this class owns. A
// friend of MainWindow, so the region changes the window's behaviour only
// here and in main_window_source.cpp.
//
//   media index    asks kerfd `media_index {media}` for the open media (once
//                  per media and file, again after a reconnection), and
//                  feeds `media_index` and `error {command: media_index}`
//                  events to the state: the viewer's shot rows.
//   snapshots      the open media's entry and file follow the snapshot; the
//                  clip "Open in timeline" goes to is found in the program.
//   Open in        ⇧F or the header's button: selects the program clip of
//   timeline       the open media (the one showing the source position, else
//                  the first) and moves the playhead to that frame of it.
//   commands       the registry's source.play and source.reveal run Play and
//                  Open in timeline (the viewers' buttons trigger them, and a
//                  key one gives them in the keymap editor works while a
//                  viewer has the focus).
//   sound          none: no engine path plays a source's sound (the bridge
//                  plays the program only), so Play is picture-only and says
//                  nothing about sound.

#include <QJsonObject>
#include <QObject>
#include <QString>

namespace kerf {

class EditorBus;
class MainWindow;
class SourceState;

class SourceWiring final : public QObject {
    Q_OBJECT
public:
    SourceWiring(MainWindow *window, EditorBus *bus);
    ~SourceWiring() override;
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }
    // The model both viewers show.
    SourceState *state() const { return m_state; }

    // Called by MainWindow::buildSource once both viewers exist.
    void attach();
    // Registers source.play and source.reveal once the registry exists.
    void attachCommands();
    // Play or pause the open media's picture.
    bool togglePlay();

    // The program clip that shows the open media at the source position
    // (else its first clip), and the program time of that frame in it.
    struct Target {
        QString clip;
        double at = 0.0;
    };
    Target revealTarget() const;
    // Open in timeline: selects that clip and moves the playhead there.
    bool reveal();
    // Asks for the open media's index unless it is in; `again` asks even
    // while an earlier request is outstanding (a new connection).
    void requestIndex(bool again = false);

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void updateReveal();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    SourceState *m_state = nullptr;
    bool m_askAgain = false;
};

} // namespace kerf
