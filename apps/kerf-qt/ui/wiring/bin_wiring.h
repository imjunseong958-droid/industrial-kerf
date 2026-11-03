#pragma once

// The bin region's wiring: the media bin and the Effects/Titles/Presets
// browser. A friend of MainWindow that holds the region's own state, so a
// region changes the window's behaviour only here and in main_window_bin.cpp.
//
// Engine paths (plan.md section 3):
//   bin tiles        snapshot timeline.sources, media_status, media_info {media}
//   Index tab        media_index {media} -> media_index {media, cuts, ranges, silences, loudest}
//   Index rows       MainWindow::openSource + the Source viewer's seek
//   Roles            timeline.tracks[].role/enabled; edit {set-track enabled|role}
//   Import           the registry's file.import
//   Effects thumbs   kerf_fx_catalog_all + effect_previews {clip, at, effects, width, key: card}
//   hover scrub      effect_previews {effects: [{fx, params, key: scrub}]}, 1 per 120 ms
//   Titles thumbs    effect_previews {clip: "", effects: [generator ids], key: title}
//   add an effect    edit {fx-add} on the selected clips (MainWindow::buildBin)
//   Presets          preset_library, preset_save, preset_inspect (unchanged)

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>

namespace kerf {

class EditorBus;
class EffectsPanel;
class MainWindow;

class BinWiring final : public QObject {
    Q_OBJECT
public:
    BinWiring(MainWindow *window, EditorBus *bus);
    MainWindow *window() const { return m_window; }
    EditorBus *bus() const { return m_bus; }

    // Called by MainWindow::buildBin once the bin, the Effects page and
    // the presets exist: makes the Titles page and connects the region.
    void build();
    EffectsPanel *titles() const { return m_titles; }

    // The picture clip the program shows at `seconds`: the media clip on
    // the uppermost enabled video track that holds it, or empty.
    static QString programClipAt(const QJsonObject &timeline, double seconds);
    // set-track ops switching `tracks` on or off.
    static QJsonArray enableOps(const QStringList &tracks, bool enabled);
    // set-track ops giving each track in `roles` ({track: role}) its role
    // ("" gives back the kind's own).
    static QJsonArray roleOps(const QJsonObject &roles);
    // The roles the Edit roles dialog offers: the engine's named ones, then
    // the timeline's own.
    static QStringList roleChoices(const QJsonArray &tracks);
    // Why the Titles page cannot add a generator yet.
    static QString titleAddReason();

    // Edit roles…: a dialog with a role per track, sent as one edit.
    void editRoles();

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void updateProgramClip();
    void requestMediaInfo();
    void followImport();

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    EffectsPanel *m_titles = nullptr;
    // Media whose media_info was asked for.
    QSet<QString> m_infoAsked;
    bool m_importFollowed = false;
};

} // namespace kerf
