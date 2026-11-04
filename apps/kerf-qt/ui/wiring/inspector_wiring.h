#pragma once

// The inspector region's wiring: the Inspector column. A friend of MainWindow that
// holds the region's own state, so a region changes the window's behaviour
// only here and in main_window_inspector.cpp.
//
// It feeds the Inspector what the bus hears (the Finish workspace for the
// colour wheels, media_info for the Info pane, the review's threads for the
// proposal hint), answers its requests (Show a proposal's thread, Save
// effects preset through look_save) and runs the ⌃⌥5 / ⌃⌥6 commands.

#include <QJsonObject>
#include <QObject>
#include <QString>

#include <functional>

class QWidget;

namespace kerf {

class EditorBus;
class MainWindow;

class InspectorWiring final : public QObject {
    Q_OBJECT
public:
    // Asks for a preset's name (a dialog by default); empty cancels.
    using NameAsker = std::function<QString(QWidget *parent, const QString &suggested)>;

    InspectorWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // Once the Inspector and its column exist (buildInspector).
    void attach();
    // Once the command registry exists: inspector.audio and inspector.info
    // choose the column's kinds. Safe to call more than once.
    void attachCommands();
    void setNameAsker(NameAsker asker) { m_askName = std::move(asker); }

    // The thread a proposal belongs to: the review model's, else the
    // comments that carry it (their root), else its own synthesised thread.
    QString threadOf(const QString &proposalId) const;
    // Save effects preset for `clip`: a name, then look_save {clip, name}.
    void savePreset(const QString &clip);
    // The reason saving is off when the bridge has no look_save.
    static QString unsupportedReason();

private:
    void hear(const QString &type, const QJsonObject &json);

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    NameAsker m_askName;
    bool m_commandsAttached = false;
    // look_save was refused as an unknown command; it stays off.
    bool m_lookSaveMissing = false;
};

} // namespace kerf
