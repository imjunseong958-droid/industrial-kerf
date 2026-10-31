#pragma once

#include "inspector_presentation.h"

#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QPointer>
#include <QSet>
#include <QWidget>

#include <functional>

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

class GlyphButton;
class InspectorRow;
class InspectorSection;
class InspectorThumbnail;
class KindSquare;
class MediaArtwork;
class NameLabel;
class ScrubField;

// The Inspector: whatever is selected, at a glance. Its identity header
// says what the clip is (thumbnail, editable name, kind, track) with its
// Start, End and Duration in timecode; under it the handful of things one
// changes, in the order one changes them, with no header; everything else
// folds into sections whose headers say what has changed in them. With
// several clips it shows what they share ("Mixed" where they differ); with
// nothing selected, the sequence. Edits leave as typed operations, one per
// gesture (several clips: one array, so one undo step).
class InspectorPanel final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
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
        enum class Kind { Prop, Linked, Effect, Mask };
        Kind kind = Kind::Prop;
        QString name;
        int index = -1;
        QString mask;
    };

    explicit InspectorPanel(QWidget *parent = nullptr);

    // The snapshot's timeline (as the editor holds it) and the selected
    // clips; an empty list shows the sequence.
    void setState(const QJsonObject &timeline, const QStringList &clips);
    void setSelection(const QStringList &clips);
    QStringList selection() const { return m_selection; }
    // The playhead, for animated values and the sequence's thumbnail.
    void setPlayhead(double seconds);
    void setArtwork(MediaArtwork *artwork);
    // The Program monitor's frame, for the sequence's thumbnail.
    void setProgramFrame(const QImage &frame);
    // The catalog from kerf_fx_catalog, for the effects' parameters.
    void setEffectCatalog(const QJsonArray &catalog);
    // The picture tracks an effect on the clip may read (track parameters)
    // and the sound tracks it may draw (sound parameters).
    void setPictureTracks(const QStringList &tracks);
    void setSoundTracks(const QStringList &tracks);
    // The mask tools, as buttons in the Masks section's header.
    void setMaskTools(const QList<QAction *> &tools);
    // A mask's property definitions and modes, from the snapshot.
    void setMaskCatalog(const QJsonArray &defs, const QStringList &modes);
    // Whether the clip shown carries masks: a picture clip does.
    void setMaskable(bool maskable);
    void selectMask(const QString &name);
    // Open the effect at `index` and scroll it into view, now or once a
    // snapshot shows it (an effect just added).
    void revealEffect(int index);

    // A colour well's swatch for `color`, ringed so white and black both show.
    static void paintWell(QPushButton *well, const QString &color);

    // What the header shows, for the window and its tests.
    QString kindText() const;
    QString nameText() const;
    QScrollArea *scrollArea() const { return m_scroll; }
    InspectorSection *section(const QString &id) const;

signals:
    void editRequested(const QJsonArray &ops);
    // A mask's name was clicked.
    void maskSelected(const QString &name);
    // ⋯ ▸ Ask Kerf: open the conversation about `subject` ("@name").
    void askRequested(const QString &subject);
    // Add effect: the Effects browser, for a picture clip or a sound one.
    void addEffectRequested(bool sound);
    // ⋯ ▸ Reveal in Project.
    void revealRequested(const QString &sourceId);
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
    void showEvent(QShowEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    // One value a row shows: how to show it again, whether it differs from
    // its default, what a folded header says about it, and the edit that
    // puts it back.
    struct Binding {
        std::function<void()> refresh;
        std::function<bool()> changed;
        std::function<QString()> summary;
        std::function<QJsonArray()> resetOps;
    };
    struct RowEntry {
        InspectorRow *row = nullptr;
        QList<Binding> values;
    };
    struct SectionEntry {
        InspectorSection *section = nullptr;
        QList<int> rows;
        std::function<QString()> summary;
    };
    enum class Mode { Sequence, Clip, Several, MixedKinds };

    void refresh();
    void gatherClips();
    QString shapeKey() const;
    void rebuild();
    void refreshValues();
    void clearBody();

    // The header.
    void buildHeader();
    void refreshHeader();
    QList<QImage> thumbnails() const;
    QImage thumbnailOf(const Clip &clip) const;
    void buildTiming(QHBoxLayout *row);

    // The body, by kind.
    void buildVideo(QVBoxLayout *into, bool generator);
    void buildAudio(QVBoxLayout *into);
    void buildTitle(QVBoxLayout *into);
    void buildSolid(QVBoxLayout *into);
    void buildAdjustment(QVBoxLayout *into);
    void buildCaption(QVBoxLayout *into);
    void buildGenerator(QVBoxLayout *into);
    void buildSequence(QVBoxLayout *into);
    void buildTransformRows(QVBoxLayout *into, int section, bool fitFill);
    void buildCompositingRows(QVBoxLayout *into, int section);
    void buildCropSection(QVBoxLayout *into);
    void buildAnchorSection(QVBoxLayout *into);
    void buildLinkedAudioSection(QVBoxLayout *into);
    void buildMasksSection(QVBoxLayout *into);
    void buildEffectsSection(QVBoxLayout *into, bool openByDefault);
    void buildSourceSection(QVBoxLayout *into);
    void buildSpeedRow(QVBoxLayout *into);
    void buildEffect(QVBoxLayout *into, int index);
    void buildMask(QVBoxLayout *into, const QString &name);

    // Rows and sections.
    int addSection(QVBoxLayout *into, const QString &id, const QString &title, bool openByDefault = false);
    QVBoxLayout *sectionBody(int section) const;
    int addRow(QVBoxLayout *into, const QString &label, int section = -1);
    InspectorRow *row(int index) const { return m_rows.at(index).row; }
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
    void saveSections();
    void timingCommitted(const QString &field, double value, double delta, bool relative);
    void resetAll();
    void askAbout(const QString &subject);
    int effectIndexAt(int y) const;
    void showDropLine(int index);
    // The body's right padding, less the scroll bar's width while it shows.
    void fitScrollBar();
    // The body's scroll bar shows only while the pointer is over the
    // Inspector or the body is scrolling, as an overlay scroller does.
    void showScrollBar(bool shown);
    QTimer *m_scrollBarTimer = nullptr;
    // A field whose Return or Esc hands the keys back.
    ScrubField *newScrubField();

    QString kindKey(const Clip &clip) const;
    QString kindName(const Clip &clip) const;
    // The name the header shows (see the definition for the fallbacks).
    QString displayName(const Clip &clip) const;
    double clipTime(const Clip &clip) const;

    // State.
    QJsonObject m_project;
    QStringList m_selection;
    QList<Clip> m_clips;
    Mode m_mode = Mode::Sequence;
    double m_playhead = 0.0;
    QPointer<MediaArtwork> m_artwork;
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
    // Sections open per kind, and where the body was scrolled.
    QHash<QString, int> m_scrollByKind;
    QSet<QString> m_openMasks;

    // The header.
    QWidget *m_header = nullptr;
    InspectorThumbnail *m_thumbnail = nullptr;
    NameLabel *m_name = nullptr;
    KindSquare *m_kindSquare = nullptr;
    QLabel *m_kind = nullptr;
    GlyphButton *m_more = nullptr;
    QMenu *m_moreMenu = nullptr;
    QWidget *m_timing = nullptr;
    QList<ScrubField *> m_timingFields;
    QLabel *m_note = nullptr;
    QFrame *m_scrollEdge = nullptr;

    // The body.
    QScrollArea *m_scroll = nullptr;
    QWidget *m_content = nullptr;
    QVBoxLayout *m_contentLayout = nullptr;
    QWidget *m_bodyWidget = nullptr;
    QFrame *m_dropLine = nullptr;
    QList<RowEntry> m_rows;
    QList<SectionEntry> m_sections;
    QList<std::function<void()>> m_refreshers;
    QList<QWidget *> m_effectBoxes;
};

} // namespace kerf
