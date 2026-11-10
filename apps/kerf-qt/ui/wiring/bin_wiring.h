#pragma once

// The bin region's wiring: the media bin. A friend of MainWindow that holds
// the region's own state, so a region changes the window's behaviour only
// here and in main_window_bin.cpp.
//
// Engine paths (plan.md section 3):
//   bin tiles        snapshot timeline.sources, media_status, media_info {media}
//   Index tab        media_index {media} -> media_index {media, cuts, ranges, silences, loudest}
//   Roles            timeline.tracks[].role/enabled; edit {set-track enabled|role}
//   Import           the registry's file.import
//   ⌘F ⌘⌥2           the registry's bin.search, bin.toggleView

#include "test_friends.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>

namespace kerf {

class EditorBus;
class MainWindow;

class BinWiring final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    BinWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // Called by MainWindow::buildBin once the bin exists: connects the region.
    void build();
    // set-track ops switching `tracks` on or off.
    static QJsonArray enableOps(const QStringList &tracks, bool enabled);
    // set-track ops giving each track in `roles` ({track: role}) its role
    // ("" gives back the kind's own).
    static QJsonArray roleOps(const QJsonObject &roles);
    // The roles the Edit roles dialog offers: the engine's named ones, then
    // the timeline's own.
    static QStringList roleChoices(const QJsonArray &tracks);
    // Registers the region's commands with the window (bin.search,
    // bin.toggleView) once the registry exists.
    void attachCommands();
    void searchBin();
    void toggleBinView();

    // Edit roles…: a dialog with a role per track, sent as one edit.
    void editRoles();

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void requestMediaInfo();
    void followImport();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    // Media whose media_info was asked for.
    QSet<QString> m_infoAsked;
    bool m_importFollowed = false;
};

} // namespace kerf
