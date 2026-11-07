#pragma once

#include "inspector_presentation.h"
#include "test_friends.h"

#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QPointer>
#include <QSet>
#include <QWidget>

#include <functional>
#include <optional>

class QAction;
class QFrame;
class QHBoxLayout;
class QLabel;
class QMenu;
class QPushButton;
class QScrollArea;
class QTimer;
class QVBoxLayout;

namespace kerf {

class ColorWheel;
class GlyphButton;
class InspectorRow;
class InspectorSection;
class MediaArtwork;
class NameLabel;
class ScrubField;

namespace ui {
class Button;
class GhostButton;
class IconButton;
} // namespace ui

// The Inspector (mockup-spec 5): whatever is selected, as flat property
// groups. Its clip header (44 points: the name, UI 500 14/20, over a mono
// 11 ink3 line, "V1 · in–out · duration", and the More menu) scrolls with
// the groups under it; each group has its enable box, count, reset on hover
// and fold, over 24-point rows on the 84 | 1fr | 56 | 20 grid (label,
// control, value, keyframe). Three panes share it, as the column's kinds:
//   Video  the effect stack (with the proposal hint row), Compositing,
//          Transform, Crop, Distort, Stabilization, Masks and Time; in the
//          Finish tray the Lift / Gamma / Gain wheels above them; a
//          footer with Save effects preset.
//   Audio  Volume (level, pan) and Channel EQ (audio.eq, added on first
//          edit) of the clip's sound (its own, or its linked clip's).
//   Info   the clip's facts as key-value rows, from the snapshot and the
//          bridge's media_info; a field the engine does not give is left out.
// With several clips it shows what they share ("Mixed" where they differ);
// with nothing selected, the sequence. Edits leave as typed operations, one
// per gesture (several clips: one array, so one undo step). A number row's
// keyframe button writes {keys:[{at: frame, value, interp}]} at the
// playhead; editing an animated value sets the key at the playhead.
class InspectorPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Pane { Video, Audio, Info };
    // The registry's ai.ask: the ⋯ menu's Ask row names its key.
    void setAskAction(QAction *action);

    // What one selected clip is, from the snapshot.
    struct Clip {
        QString id;
        QJsonObject clip;
        QJsonObject track;
        QJsonObject source;      // its media, if it shows media
        QJsonObject linked;      // the clip linked to it on a track of the other kind
        QJsonObject linkedTrack;
    };
    // Which value a row edits.
    struct Target {
        // Fx: the parameter `name` of the first effect `fx` in the stack,
        // whether the clip carries one yet or not (an edit adds it).
        enum class Kind { Prop, Linked, Effect, Mask, Fx };
        Kind kind = Kind::Prop;
        QString name;
        int index = -1;
        QString mask;
        QString fx;
    };

    explicit InspectorPanel(QWidget *parent = nullptr);
    ~InspectorPanel() override;

    // The snapshot's timeline (as the editor holds it) and the selected
    // clips; an empty list shows the sequence.
    void setState(const QJsonObject &timeline, const QStringList &clips);
    void setSelection(const QStringList &clips);
    QStringList selection() const { return m_selection; }
    // The pane the column shows (its kind: Video, Audio or Info).
    void setPane(Pane pane);
    Pane pane() const { return m_pane; }
    // While the Finish tray is open: the colour wheels over the Video pane's groups.
    void setFinish(bool finish);
    bool finish() const { return m_finish; }
    // The playhead, for animated values and keyframes.
    void setPlayhead(double seconds);
    void setArtwork(MediaArtwork *artwork);
    // The picture's frame (kept for the sequence; nothing draws it).
    void setProgramFrame(const QImage &frame);
    // The catalog from kerf_fx_catalog (or _all), for the effects'
    // parameters. Entries are added to those already known, so the sound
    // effects of kerf_fx_catalog_all stay when the picture catalog arrives.
    void setEffectCatalog(const QJsonArray &catalog);
    // The picture tracks an effect on the clip may read (track parameters)
    // and the sound tracks it may draw (sound parameters).
    void setPictureTracks(const QStringList &tracks);
    void setSoundTracks(const QStringList &tracks);
    // The mask tools, as buttons in the Masks group's header.
    void setMaskTools(const QList<QAction *> &tools);
    // A mask's property definitions and modes, from the snapshot.
    void setMaskCatalog(const QJsonArray &defs, const QStringList &modes);
    // Whether the clip shown carries masks: a picture clip does.
    void setMaskable(bool maskable);
    void selectMask(const QString &name);
    // Open the effect at `index` and scroll it into view, now or once a
    // snapshot shows it (an effect just added).
    void revealEffect(int index);
    // The snapshot's proposals and their timelines: the hint row says what an
    // open one would change on the clip shown. `threadOf` names the comment
    // thread a proposal belongs to.
    void setProposals(const QJsonArray &proposals, const QJsonObject &timelines);
    void setThreadResolver(std::function<QString(const QString &proposalId)> threadOf);
    // media_info's {media id: {codec, profile, pixFmt, bitDepth, colorSpace,
    // transfer, primaries, camera, lens, reel, audioBitDepth, …}}, merged
    // into what is known.
    void setMediaInfo(const QJsonObject &media);
    QJsonObject mediaInfo() const { return m_mediaInfo; }
    // Whether the bridge saves effects presets (look_save); when it does not,
    // the footer's button is disabled with `reason` as its tooltip.
    void setPresetSaving(bool available, const QString &reason = QString());
    bool presetSaving() const { return m_presetSaving; }

    // A colour well's swatch for `color`.
    static void paintWell(QPushButton *well, const QString &color);

    // What the header shows, for the window and its tests.
    QString kindText() const;
    QString nameText() const;
    QString metaText() const;
    QScrollArea *scrollArea() const { return m_scroll; }
    InspectorSection *section(const QString &id) const;
    QWidget *header() const { return m_header; }
    QWidget *footer() const { return m_footer; }
    ui::Button *savePresetButton() const { return m_savePreset; }
    // The hint row of an open proposal that changes the clip, or null.
    QWidget *proposalHint() const { return m_hint; }
    QString proposalHintText() const;
    // The wheels row (Finish), or null.
    QWidget *wheels() const { return m_wheels; }
    // The Info pane's value for `key` ("Codec", "Lens"), empty when absent.
    QString infoValue(const QString &key) const;
    QStringList infoKeys() const;

signals:
    void editRequested(const QJsonArray &ops);
    // A mask's name was clicked.
    void maskSelected(const QString &name);
    // More ▸ Ask Kerf: open the conversation about `subject` ("@name").
    void askRequested(const QString &subject);
    // Add effect: the Effects browser, for a picture clip or a sound one.
    void addEffectRequested(bool sound);
    // More ▸ Reveal in project.
    void revealRequested(const QString &sourceId);
    // The hint row's Show: the proposal's thread and card.
    void proposalShowRequested(const QString &threadId, const QString &proposalId);
    // Save effects preset, for the clip shown.
    void savePresetRequested(const QString &clipId);
    // Typing ended (Return or Esc), or Esc pressed with nothing being typed:
    // the keys go back to the timeline.
    void focusReturnRequested();

protected:
    void dragEnterEvent(QDragEnterEvent *event) override;
    void dragMoveEvent(QDragMoveEvent *event) override;
    void dragLeaveEvent(QDragLeaveEvent *event) override;
    void dropEvent(QDropEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;

private:
    // One value a row shows: how to show it again, whether it differs from
    // its default, what the group's tooltip says about it, and the edit that
    // puts it back.
    struct Binding {
        std::function<void()> refresh;
        std::function<bool()> changed;
        std::function<QString()> summary;
        std::function<QJsonArray()> resetOps;
    };
    // A number a row's keyframe button keys.
    struct Keyed {
        Target target;
        QJsonObject def;
    };
    struct RowEntry {
        InspectorRow *row = nullptr;
        QList<Binding> values;
        QList<Keyed> keyed;
    };
    struct SectionEntry {
        InspectorSection *section = nullptr;
        QList<int> rows;
        std::function<QString()> summary;
        // Sets a summary of whole items (the effects); wins over `summary`.
        std::function<void(InspectorSection *)> items;
        // The group's enable box: what it says now, and what a click does.
        std::function<void()> enable;
    };
    // A keyframe as the snapshot gives it: its frame from the clip's start.
    struct Key {
        qint64 frame = 0;
        QJsonValue value;
        QString interp;
    };
    enum class Mode { Sequence, Clip, Several, MixedKinds };

    void refresh();
    void gatherClips();
    QString shapeKey() const;
    void rebuild();
    void refreshValues();
    void clearBody();
    // The body in the column, scrolled back to where it was for its kind.
    void showBody();
    // A body built for one shape is parked while another shows, and shown
    // again when a clip of its shape is chosen, instead of being built anew.
    void parkBody();
    bool unparkBody(const QString &shape);
    // What the bodies show changed (a catalog, the tracks): the next
    // refresh builds the body again, and nothing parked is shown.
    void forgetBodies();

    // The clip header.
    void buildHeader();
    void refreshHeader();
    QString askKey() const;
    void refreshFooter();
    void updateFade();

    // The body, by pane and kind.
    void buildVideo(QVBoxLayout *into, bool generator);
    void buildAudio(QVBoxLayout *into);
    void buildAudioPane(QVBoxLayout *into);
    void buildInfo(QVBoxLayout *into);
    void buildTitle(QVBoxLayout *into);
    void buildSolid(QVBoxLayout *into);
    void buildAdjustment(QVBoxLayout *into);
    void buildCaption(QVBoxLayout *into);
    void buildGenerator(QVBoxLayout *into);
    void buildSequence(QVBoxLayout *into);
    void buildWheels(QVBoxLayout *into);
    void buildTransformRows(QVBoxLayout *into, int section);
    void buildDepthRows(QVBoxLayout *into, int section);
    void buildFitFill(int section);
    void buildCompositingRows(QVBoxLayout *into, int section);
    void buildCropSection(QVBoxLayout *into);
    void buildEffectGroup(QVBoxLayout *into, const QString &id, const QString &title, const QString &fx);
    void buildChannelEq(QVBoxLayout *into);
    void buildMasksSection(QVBoxLayout *into);
    void buildEffectsSection(QVBoxLayout *into, bool openByDefault);
    // Time: the clip's timing, its speed (and reverse) and its source In and Out.
    void buildTimeSection(QVBoxLayout *into);
    void buildTiming(QVBoxLayout *into, int section);
    void buildSpeedRow(QVBoxLayout *into, int section);
    void buildSourceRows(QVBoxLayout *into, int section);
    void buildEffect(QVBoxLayout *into, int index);
    void buildMask(QVBoxLayout *into, const QString &name);
    void buildProposalHint();
    void placeProposalHint();
    // A key-value row (.kv): UI 12 ink2, the value right-aligned in mono 12
    // ink (UI when `ui`), with an ellipsis.
    void addInfoRow(QVBoxLayout *into, const QString &key, const QString &value, bool ui = false);
    void addInfoHeader(QVBoxLayout *into, const QString &title);

    // Rows and groups.
    int addSection(QVBoxLayout *into, const QString &id, const QString &title, bool openByDefault = false);
    QVBoxLayout *sectionBody(int section) const;
    int addRow(QVBoxLayout *into, const QString &label, int section = -1);
    InspectorRow *row(int index) const;
    // A number: a ScrubField (and a slider, when given) bound to `target`.
    ScrubField *bindNumber(int row, QHBoxLayout *into, const Target &target, const QJsonObject &def,
                           bool withSlider = false, bool primaryField = true);
    // An enum, a colour, text, a file, a track or a switch, by type.
    QWidget *bindValue(int row, QHBoxLayout *into, const Target &target, const QJsonObject &def);
    QJsonObject defNamed(const Clip &clip, const QString &name) const;
    QJsonArray defsOf(const Clip &clip) const;
    QJsonObject effectParamDef(int effect, const QString &name) const;
    NumberFormat formatFor(const Clip &clip, const QJsonObject &def) const;
    PresentationContext contextOf(const Clip &clip) const;
    QJsonValue rawValue(const Clip &clip, const Target &target) const;
    QJsonValue valueOf(const Clip &clip, const Target &target, const QJsonObject &def) const;
    bool isAnimated(const Clip &clip, const Target &target) const;
    double displayOf(const Clip &clip, const Target &target, const QJsonObject &def) const;
    QJsonObject opFor(const Clip &clip, const Target &target, const QJsonValue &value) const;
    bool targets(const Clip &clip, const Target &target) const;
    void commitNumber(const Target &target, const QJsonObject &def, double value, double delta, bool relative);
    void commitValue(const Target &target, const QJsonObject &def, const QJsonValue &value);
    QJsonArray resetOpsFor(const Target &target, const QJsonObject &def) const;
    void emitOps(const QJsonArray &ops);
    void setOpenSections(const QString &id, bool open, Qt::KeyboardModifiers modifiers);
    bool savedOpen(const QString &id, bool openByDefault) const;
    void saveSections();
    void timingCommitted(const QString &field, double value, double delta, bool relative);
    void resetAll();
    void askAbout(const QString &subject);
    int effectIndexAt(int y) const;
    void showDropLine(int index);
    ScrubField *newScrubField();

    // Keyframes.
    double fpsOf() const;
    // The playhead's frame from the clip's first frame; -1 when the playhead
    // is not over the clip.
    qint64 playheadFrame(const Clip &clip) const;
    QList<Key> keysOf(const Clip &clip, const Target &target) const;
    static QJsonValue keysJson(const QList<Key> &keys);
    // Why a parameter cannot take keyframes here (empty when it can).
    QString keyframeRefusal(const Clip &clip, const Target &target, const QJsonObject &def) const;
    void bindKeyframe(int row, const Target &target, const QJsonObject &def);
    void refreshKeyframe(int row);
    void toggleKeyframe(int row);

    // Effects by id in the clip's stack (the first), -1 for none.
    int effectIndexOf(const Clip &clip, const QString &fx) const;
    // The edits that set `params` on the clip's effect `fx`: fx-set on it, or
    // fx-add with them when the stack has none.
    QJsonArray effectParamOps(const Clip &clip, const QString &fx, const QJsonObject &params) const;

    QString kindKey(const Clip &clip) const;
    QString kindName(const Clip &clip) const;
    QString displayName(const Clip &clip) const;
    double clipTime(const Clip &clip) const;
    // The sound a clip carries for the Audio pane: itself, its linked clip,
    // or nothing.
    std::optional<Clip> soundOf(const Clip &clip) const;

    // State.
    QJsonObject m_project;
    QStringList m_selection;
    QList<Clip> m_clips;
    Mode m_mode = Mode::Sequence;
    Pane m_pane = Pane::Video;
    bool m_finish = false;
    // The Audio pane with clips selected that carry no sound.
    bool m_silent = false;
    double m_playhead = 0.0;
    QPointer<MediaArtwork> m_artwork;
    QPointer<QAction> m_askAction;
    QImage m_programFrame;
    QHash<QString, QJsonObject> m_effectDefs;
    QStringList m_pictureTracks;
    QStringList m_soundTracks;
    QList<QAction *> m_maskTools;
    QJsonArray m_maskDefs;
    QStringList m_maskModes;
    bool m_maskable = false;
    QString m_selectedMask;
    QString m_shape;
    QString m_kindForSections;
    int m_revealEffect = -1;
    int m_dropIndex = -1;
    QHash<QString, int> m_scrollByKind;
    QSet<QString> m_openMasks;
    QJsonArray m_proposals;
    QJsonObject m_proposalTimelines;
    std::function<QString(const QString &)> m_threadOf;
    QJsonObject m_mediaInfo;
    bool m_presetSaving = true;
    QString m_presetReason;
    // The proposal the hint row names, and the group it sits in.
    QString m_hintProposal;
    QString m_hintThread;
    // Where each target's row sits: "prop:x", "fx:2:amount" → group index.
    QHash<QString, int> m_targetSections;
    QHash<int, int> m_rowSections;
    QHash<QString, QString> m_info; // the Info pane's rows, key → value

    // The clip header.
    QWidget *m_header = nullptr;
    NameLabel *m_name = nullptr;
    QLabel *m_kind = nullptr; // the mono meta line under the name
    ui::IconButton *m_more = nullptr;
    QMenu *m_moreMenu = nullptr;
    QList<ScrubField *> m_timingFields;
    QLabel *m_note = nullptr;

    // The body.
    QScrollArea *m_scroll = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_contentLayout = nullptr;
    QWidget *m_bodyWidget = nullptr;
    QWidget *m_fade = nullptr;
    QFrame *m_dropLine = nullptr;
    QWidget *m_hint = nullptr;
    QLabel *m_hintText = nullptr;
    QWidget *m_wheels = nullptr;
    QWidget *m_footer = nullptr;
    ui::Button *m_savePreset = nullptr;
    QList<RowEntry> m_rows;
    QList<SectionEntry> m_sections;
    QList<std::function<void()>> m_refreshers;
    QList<QWidget *> m_effectBoxes;
    struct ParkedBody {
        QWidget *widget = nullptr;
        QString kind; // m_kindForSections
        QList<RowEntry> rows;
        QList<SectionEntry> sections;
        QList<std::function<void()>> refreshers;
        QList<QWidget *> effectBoxes;
        QList<ScrubField *> timingFields;
        QHash<QString, int> targetSections;
        QHash<int, int> rowSections;
        QWidget *hint = nullptr;
        QLabel *hintText = nullptr;
        QWidget *wheels = nullptr;
    };
    static constexpr int kParkedBodies = 6;
    // Moving a body back costs about as much as building a plain one, so
    // only a body that took longer than this to build is parked.
    static constexpr qint64 kParkedBodyCost = 12;
    qint64 m_bodyCost = 0;
    QWidget *m_parking = nullptr;
    QHash<QString, ParkedBody> m_parked;
    QStringList m_parkedOrder;
};

} // namespace kerf
