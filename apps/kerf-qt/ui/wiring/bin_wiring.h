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
//                    on the selected picture clip, else the program's clip under
//                    the playhead, else the sequence's first picture clip
//   hover scrub      effect_previews {effects: [{fx, params, key: scrub}]}, 1 per 120 ms
//   Titles thumbs    effect_previews {clip: "", effects: [generator ids], key: title}
//   add a title      edit {insert {generator: {fx, params}, at, frames, tracks,
//                    mode: overwrite}} at the playhead, 5 s at the sequence's
//                    rate, on the target video track above V1 (else the first
//                    above it, else a new one: add-track first); a card dropped
//                    on a video track goes there. kerfd's answer (state or
//                    error) shows on the Titles page.
//   ⌘F ⌘⌥2 ⌘⇧F       the registry's bin.search, bin.toggleView, browser.search
//   add an effect    edit {fx-add} on the selected clips (MainWindow::buildBin)
//   Presets          preset_library, preset_save, preset_inspect (unchanged)

#include "test_friends.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QSet>
#include <QString>
#include <QStringList>

class QTimer;

namespace kerf {

class EditorBus;
class EffectsPanel;
class MainWindow;

class BinWiring final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
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
    // The video clip the Effects page previews on when no picture clip is
    // selected: the program's at `seconds`, else the sequence's first media
    // clip on a video track; empty when there is no picture clip at all.
    static QString previewClipFor(const QJsonObject &timeline, double seconds);

    // ---- Titles: adding a generator
    // A generator clip lasts this long when added.
    static constexpr double kTitleSeconds = 5.0;
    // What a title says when it is added (and what its card shows).
    static QString defaultTitleText();
    // The track a generator goes on: `dropped` when it is an unlocked video
    // track; else the targeted video track above V1; else the first unlocked
    // video track above V1; empty when there is none (one is added).
    static QString titleTrack(const QJsonObject &timeline, const QStringList &targets, const QString &dropped = {});
    // A free id for a new video track: V<n> after the video tracks there are.
    static QString newVideoTrackId(const QJsonObject &timeline);
    // The ops that add `fx` at frame `at` for `frames` on `track` (a new
    // video track first when `track` is empty).
    static QJsonArray generatorOps(const QString &fx, const QJsonObject &params, qint64 at, qint64 frames, const QString &track,
                                   const QString &newTrack = {});
    // The ids of the generator clips in a timeline.
    static QSet<QString> generatorClips(const QJsonObject &timeline);
    // Whether `timeline` holds a generator clip of `fx` on `track` starting
    // at frame `at`, other than those in `before`: how a snapshot shows that
    // kerfd took an add.
    static bool holdsGenerator(const QJsonObject &timeline, const QString &track, const QString &fx, qint64 at, double fps,
                               const QSet<QString> &before = {});
    // Adds `fx` at `seconds` on the program (the playhead when negative), on
    // `dropped` when it is a video track above V1.
    bool addGenerator(const QString &fx, const QString &dropped = {}, double seconds = -1.0);
    // The ops last sent by addGenerator, until kerfd answers them.
    bool titlePending() const { return m_titlePending; }

    // Registers the region's commands with the window (bin.search,
    // bin.toggleView, browser.search) once the registry exists.
    void attachCommands();
    void searchBin();
    void toggleBinView();
    void searchBrowser();

    // Edit roles…: a dialog with a role per track, sent as one edit.
    void editRoles();

private:
    void onEvent(const QString &type, const QJsonObject &event);
    void onSnapshot();
    void updateProgramClip();
    void requestMediaInfo();
    void followImport();
    void settleTitle(const QString &error);

    MainWindow *m_window = nullptr;
    EditorBus *m_bus = nullptr;
    EffectsPanel *m_titles = nullptr;
    // Media whose media_info was asked for.
    QSet<QString> m_infoAsked;
    bool m_importFollowed = false;
    // A generator add on its way: what it was and the revision it was sent at.
    bool m_titlePending = false;
    QString m_titleLabel;
    quint64 m_titleRevision = 0;
    // What the add asked for, to find it in a newer snapshot.
    QString m_titleFx;
    QString m_titleTrack;
    qint64 m_titleAt = 0;
    double m_titleFps = 0;
    QSet<QString> m_titleBefore;
    QTimer *m_titleTimer = nullptr;
    QObject *m_dropFilter = nullptr;
};

} // namespace kerf
