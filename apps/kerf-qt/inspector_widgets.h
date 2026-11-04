#pragma once

#include "design_tokens.h"
#include "editor_common.h"
#include "inspector_presentation.h"
#include "test_friends.h"

#include <QAbstractButton>
#include <QComboBox>
#include <QLabel>
#include <QPointer>
#include <QWidget>

#include <functional>

class QHBoxLayout;
class QLineEdit;
class QMenu;
class QTimer;
class QVBoxLayout;

namespace kerf {

// The Inspector's controls in the flat graphite look of the review-thread
// design (mockup-spec 5): property groups with a 28-point header (an
// enable box, the name, a count, a reset shown on hover and a fold
// chevron) over 24-point rows on one grid, 84 | 1fr | 56 | 20 (label,
// control, value, keyframe), with the kit's slider, ghost pop-ups, mono
// values with their units, X/Y pairs and keyframe buttons. Colours, type
// and radii come from design_tokens.h only.

// A type style as style-sheet declarations (family, size, weight): the app's
// sheet sets `* { font-size: 12px }`, which wins over setFont(), so a label
// in another style says its own in its local sheet.
QString inspectorFontCss(const tokens::TypeStyle &style);

// The ink a value is drawn in once it moved from its default. The design
// shows every value in the primary ink; kept for the callers that ask.
QColor changedValueInk();

// The one control behind every number and timecode: the value right-aligned
// in mono 12 ink with its unit after it (UI 11 ink3 in a 14-point slot, 2
// after the figures) and an axis prefix before it (UI 11 ink3, 4 before the
// figures). Drag across it (or its row's label) to scrub, one step per 2
// points (⇧ ×10, ⌥ ÷10); click to type; ↑↓ step while focused or typing. A
// gesture is one edit: the value follows the pointer and `committed` fires
// once, on release; a run of ↑↓ presses (a held key, or presses under 400 ms
// apart) is one edit too, committed 400 ms after the last or when the field
// loses focus. A click or a drag takes no keyboard focus (only Tab and typing
// do), so the transport keys keep working; Return or Esc after typing hands
// the keys back (`doneEditing`).
class ScrubField final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kHeight = 24;
    static constexpr int kUnitSlot = 14; // the unit's slot after the figures
    static constexpr int kUnitGap = 2;

    explicit ScrubField(QWidget *parent = nullptr);
    // Room kept clear at the right end, for a glyph inside the field (the
    // speed presets' ⌄).
    void setTrailingInset(int inset);

    void setFormat(const NumberFormat &format);
    const NumberFormat &format() const { return m_format; }
    // The value in display units; it does not emit.
    void setValue(double display);
    double value() const { return m_value; }
    // The value a reset restores.
    void setDefaultValue(double display);
    bool isDefault() const;
    // Several clips disagree: "Mixed", and scrubbing moves each by the same amount.
    void setMixed(bool mixed);
    bool isMixed() const { return m_mixed; }
    // Its effect is switched off: the value in the secondary ink (it still edits).
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // The small form under a colour wheel: mono 11 ink3, centred.
    void setSmall(bool small);
    bool isSmall() const { return m_small; }
    // The ink its value is drawn in, and which side it keeps to.
    QColor valueInk() const;
    Qt::Alignment valueAlignment() const;
    // Read-only: shown, never edited (a value no gesture may change).
    void setReadOnly(bool readOnly);
    bool isReadOnly() const { return m_readOnly; }
    QString text() const;
    // The unit as drawn after the figures (empty for none).
    QString unitText() const;
    // Whether its value, unit and prefix fit it whole (no ellipsis).
    bool showsWhole() const;
    // The width its figures, unit and prefix need, snapped to whole points.
    int neededWidth() const;
    QSize sizeHint() const override;

    // A drag that began on the row's label.
    void scrubPress(const QPoint &global);
    void scrubMove(const QPoint &global, Qt::KeyboardModifiers modifiers);
    void scrubRelease();
    // Open the field for typing, with the number selected.
    void beginTyping();
    bool isTyping() const;
    QLineEdit *editor() const { return m_editor; }
    // Commit typed text as if Return were pressed.
    void commitText(const QString &text);
    // Commit a run of ↑↓ steps now rather than 400 ms after the last.
    void flushSteps();
    bool isStepping() const { return m_stepping; }
    static constexpr int kStepRunMs = 400;

signals:
    // Return or Esc ended typing: the keys go back where they came from.
    void doneEditing();
    // One per gesture: the new value, and how far it moved from where the
    // gesture started (for several clips moved alike). `relative` is false
    // when a value was typed outright.
    void committed(double value, double delta, bool relative);
    // While dragging.
    void previewed(double value);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    QFont valueFont() const;
    double clamped(double display, bool *clamped = nullptr) const;
    double stepFor(Qt::KeyboardModifiers modifiers) const;
    void endTyping(bool commit);
    // ↑↓ while typing: step from the typed value, as one run of steps.
    void stepEditor(Qt::KeyboardModifiers modifiers, bool up);
    void flash();

    NumberFormat m_format;
    double m_value = 0.0;
    double m_default = 0.0;
    bool m_hasDefault = false;
    bool m_mixed = false;
    bool m_dimmed = false;
    bool m_small = false;
    bool m_readOnly = false;
    bool m_hovered = false;
    // Dragging: where it began, and the value then.
    bool m_pressed = false;
    bool m_dragging = false;
    int m_pressX = 0;
    int m_travel = 0;
    double m_startValue = 0.0;
    QLineEdit *m_editor = nullptr;
    QTimer *m_flash = nullptr;
    int m_trailingInset = 0;
    // A run of ↑↓ steps: the value it started from, and the pause that ends it.
    bool m_stepping = false;
    double m_stepStart = 0.0;
    QTimer *m_stepRun = nullptr;
};

// The kit's slider (.sl) paired with a ScrubField (the field is the source
// of truth), in a 24-point row: a 2-point track (`track`) at the row's
// middle, filled in `track2` from its origin (the left end, or the middle of
// a bipolar range) to the knob, a 10-point ink knob with the knob shadow,
// and on a bipolar slider a 1 x 6 `track2` tick at the middle. A gain fader
// maps its travel so 0 dB sits at 75%. A press never jumps the value, on the
// knob or anywhere on the track: a drag moves the knob by as much as the
// pointer moves, so a stray click or an off-centre grab changes nothing. The
// default catches the knob on one pointer pixel only.
class InspectorSlider final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr double kGrab = 6.0;
    static constexpr double kKnobRadius = 5.0;
    explicit InspectorSlider(QWidget *parent = nullptr);
    void setRange(double minimum, double maximum);
    void setOrigin(double origin);
    void setDetent(double detent);
    void setGainTravel(bool gain);
    void setValue(double value);
    double value() const { return m_value; }
    // Several clips disagree: the knob hides.
    void setMixed(bool mixed);
    // Its effect is switched off: the fill quieter (it still edits).
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // A range either side of zero: filled from the middle, with its tick.
    bool isBipolar() const;
    // The ink its fill is drawn in.
    QColor fillInk() const;
    QSize sizeHint() const override { return {100, ScrubField::kHeight}; }
    // The knob's centre and the track, for tests.
    QPointF knobCenter() const;
    QRectF trackRect() const { return groove(); }
    // The fill from the origin to the knob's centre; empty at the origin.
    QRectF fillRect() const;
    // The bipolar slider's 1 x 6 tick at its middle; empty otherwise.
    QRect detentTick() const;
    // The default catches the pointer only on the one pixel nearest it
    // (half a pixel of pointer travel either side).
    static constexpr double kSnap = 0.5;
    // With ⌥ held the knob moves a tenth of the pointer's travel.
    static constexpr double kFine = 10.0;
    // The value at `x` along the track; `scale` is how far the knob moves
    // per pixel of pointer travel (1, or 1/kFine), which the catch follows.
    double valueAt(double x, double scale = 1.0) const;

signals:
    void moved(double value);
    void released(double value, double start);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;

private:
    double travelOf(double value) const;
    double dragValue(double x, Qt::KeyboardModifiers modifiers);
    double fromTravel(double travel) const;
    QRectF groove() const;

    double m_min = 0.0;
    double m_max = 1.0;
    double m_origin = 0.0;
    double m_detent = 0.0;
    bool m_gain = false;
    double m_value = 0.0;
    double m_start = 0.0;
    bool m_mixed = false;
    bool m_dimmed = false;
    bool m_dragging = false;
    double m_pressX = 0.0;
    double m_pressKnob = 0.0;
    bool m_fine = false;
};

// The kit's 14-point check box (.cbx): radius 4, an inset 1-point white 16%
// line; on, the `on` fill with the check path in ink2. Space toggles it when
// focused. A switch property (a mask's Invert, a bool parameter) and a
// group's or an effect's enable box.
class InspectorSwitch : public QAbstractButton {
    Q_OBJECT
public:
    static constexpr int kSize = 14;
    explicit InspectorSwitch(QWidget *parent = nullptr);
    QSize sizeHint() const override { return {kSize, kSize}; }

protected:
    void paintEvent(QPaintEvent *event) override;
};

// Two or three choices on the kit's segmented track (raised, radius 6, pad
// 2): the chosen one on the `on` fill, its legend in ink 500, the others in
// ink2. None is chosen when the values match no choice.
class SegmentedField final : public QWidget {
    Q_OBJECT
public:
    enum class Glyph { None, AlignLeft, AlignCenter, AlignRight };
    struct Segment {
        QString label;
        Glyph glyph = Glyph::None;
        QString tip;
    };
    explicit SegmentedField(const QList<Segment> &segments, QWidget *parent = nullptr);
    void setCurrent(int index);
    int current() const { return m_current; }
    void setDefaultIndex(int index);
    int count() const { return int(m_segments.size()); }
    QRect segmentRect(int index) const;
    // The ink the choice at `index` is drawn in.
    QColor inkAt(int index) const;
    QSize sizeHint() const override;

signals:
    void activated(int index);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;

private:
    QList<Segment> m_segments;
    int m_current = -1;
    int m_default = -1;
};

// A pop-up as the design draws it: a ghost button (.ghost-b, 24 tall,
// radius 6, pad 0 8, UI 12/16 ink2 with a 14-point chevron; hover raised and
// ink), right-aligned in its row. It is a QComboBox underneath (items, data,
// `activated`), and opens a kit menu (ui::styleMenu) of its items, the
// current one checked.
class GhostPopup : public QComboBox {
    Q_OBJECT
public:
    explicit GhostPopup(QWidget *parent = nullptr);
    void showPopup() override;
    void hidePopup() override;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;
    // The menu the last showPopup() opened (for tests), or null.
    QMenu *menu() const { return m_menu; }
    QColor textColor() const;

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    QPointer<QMenu> m_menu;
    bool m_hovered = false;
};

// A row's keyframe button (.kf): 20 x 20, radius 4. Empty: the g-kf
// diamond outline in ink3; hover raised and ink. At a keyframe (`has`):
// the filled g-kff diamond in ink, "Keyframe at playhead". Disabled: dis,
// its tooltip saying why.
class KeyframeButton final : public QAbstractButton {
    Q_OBJECT
public:
    static constexpr int kSize = 20;
    explicit KeyframeButton(QWidget *parent = nullptr);
    void setHas(bool has);
    bool has() const { return m_has; }
    // The parameter has keyframes, though none at the playhead.
    void setAnimated(bool animated);
    bool isAnimated() const { return m_animated; }
    QString iconName() const;
    QColor iconColor() const;
    QSize sizeHint() const override { return {kSize, kSize}; }

protected:
    void paintEvent(QPaintEvent *event) override;
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    bool m_has = false;
    bool m_animated = false;
    bool m_hovered = false;
};

// A bare icon button (.ib, 24 x 24, radius 6, a 16-point icon in ink2;
// hover raised and ink; checked on and ink; disabled dis). The glyphs are
// the design's icons where it has one (g-reset, g-more, g-chev, g-kf,
// g-mrect, g-mell, g-mpen), drawn in code for the rest.
class GlyphButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Glyph { Reset, More, Plus, Chevron, Reverse, Menu, Diamond, Rectangle, Ellipse, Pen, Folder, Bubble };
    explicit GlyphButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    // A chevron's fold: open points down, folded turns it −90°.
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    QSize sizeHint() const override;
    // The fill behind the glyph (invalid when none) and the glyph's ink.
    QColor discColor() const;
    QColor inkColor() const;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    Glyph m_glyph;
    bool m_open = false;
};

// Draw one of the glyphs into `rect` in `color`.
void drawInspectorGlyph(QPainter &painter, GlyphButton::Glyph glyph, const QRectF &rect, const QColor &color, bool open = false);

// A row's label: UI 12/16 ink2, one line, an ellipsis where its column cuts
// it (the whole in its tooltip). Drag across it to scrub the row's field,
// double-click it to reset the row.
class RowLabel final : public QLabel {
    Q_OBJECT
public:
    explicit RowLabel(const QString &text, QWidget *parent = nullptr);
    void setField(ScrubField *field) { m_field = field; }
    ScrubField *field() const { return m_field; }
    QString fullText() const { return m_full; }
    // The label of a `.l.w` row: in ink, beside a check box.
    void setStrong(bool strong);

signals:
    void resetRequested();

protected:
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    QPointer<ScrubField> m_field;
    QString m_full;
    bool m_pressed = false;
};

// One 24-point property row on the column's grid: pad 0 8 0 12, then
// 84 | 1fr | 56 | 20 with 8 between (label, control, value, keyframe). A
// slider fills the control column and its value sits in the value column;
// a row without a slider is "wide": its label runs on as far as its
// controls (a pop-up, a value, an X/Y pair) leave, and they sit at the right
// end of the control and value columns. The keyframe column always keeps
// its place, so every row's values end at the same edge.
class InspectorRow final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeight = 24;
    static constexpr int kLabelWidth = 84;
    static constexpr int kValueField = 56;
    static constexpr int kKeyframe = 20;
    static constexpr int kGap = 8;
    static constexpr int kPadLeft = 12;
    static constexpr int kPadRight = 8;

    explicit InspectorRow(const QString &label, QWidget *parent = nullptr);
    RowLabel *label() const { return m_label; }
    // The control column (1fr), and the value column (56).
    QHBoxLayout *controls() const { return m_controls; }
    void addValue(QWidget *value);
    QWidget *valueSlot() const { return m_valueSlot; }
    KeyframeButton *keyframeButton() const { return m_keyframe; }
    // Wide: the label takes what the controls leave (no slider in the row).
    void setWide(bool wide);
    bool isWide() const { return m_wide; }
    void setChanged(bool changed);
    bool isChanged() const { return m_changed; }
    // An animated value, and what its keyframe button says.
    void setAnimated(bool animated, const QString &tip = QString());
    bool isAnimated() const { return m_animated; }
    void setResetHandler(std::function<void()> reset);
    void reset();

private:
    RowLabel *m_label = nullptr;
    QWidget *m_controlSlot = nullptr;
    QHBoxLayout *m_controls = nullptr;
    QWidget *m_valueSlot = nullptr;
    QHBoxLayout *m_values = nullptr;
    KeyframeButton *m_keyframe = nullptr;
    std::function<void()> m_resetHandler;
    bool m_wide = false;
    bool m_changed = false;
    bool m_animated = false;
};

// A property group (#insp .grp): a 28-point header, pad 0 8 0 12, gap 8,
// UI 500 12 ink: the enable box (the effect's enabled flag, or the group's
// own switch; disabled with the reason where nothing can switch it), the
// name, a count in ink3 400, then at the right end the reset icon button,
// shown while the pointer is over the header, and the 14-point fold chevron
// in ink3, turned −90° when folded. Its rows under it, 4 of padding under
// them; a 1-point line at its foot. Folded, the rows hide. A click on the
// header folds or opens it; ⌥-click opens or closes every group; ⌘-click
// opens this one and closes the rest.
class InspectorSection final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeaderHeight = 28;
    static constexpr int kFoot = 4;

    // What the header's box says.
    enum class Enable {
        Hidden,  // no box (a list: tracks, markers)
        On,      // checked, toggles
        Off,     // unchecked, toggles (a group added on first enable)
        Always,  // checked and disabled: always applied (its tooltip says so)
    };

    explicit InspectorSection(const QString &id, const QString &title, QWidget *parent = nullptr);
    QString id() const { return m_id; }
    QString title() const { return m_title; }
    QWidget *header() const { return m_header; }
    QVBoxLayout *body() const { return m_body; }
    QWidget *bodyWidget() const { return m_bodyWidget; }
    QLabel *titleLabel() const { return m_titleLabel; }
    InspectorSwitch *enableBox() const { return m_enable; }
    void setEnable(Enable enable, const QString &tip = QString());
    Enable enable() const { return m_enableState; }
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    void setCount(int count);
    // What changed in it, as whole items, for the header's tooltip.
    struct Summary {
        QStringList items;
        QString suffix;
        QString compact;
        QString full;
    };
    void setSummary(const Summary &summary);
    void setSummary(const QString &summary);
    QString summary() const { return m_summary.full; }
    // The longest form of `summary` that fits `room` points in `metrics`.
    static QString fitSummary(const Summary &summary, const QFontMetrics &metrics, int room);
    void addTrailing(QWidget *widget);
    void setChanged(bool changed);
    bool isChanged() const { return m_changed; }
    void setResetHandler(std::function<void()> reset);
    GlyphButton *resetButton() const { return m_reset; }
    GlyphButton *chevron() const { return m_chevron; }
    QSize headerSizeHint() const;

signals:
    // A click on the header; `modifiers` say whether all, or only this.
    void toggled(bool open, Qt::KeyboardModifiers modifiers);
    // Opened or folded, by a click or from code.
    void foldChanged(bool open);
    // The enable box was clicked.
    void enableToggled(bool on);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;

private:
    void updateHeader();
    QString m_id;
    QString m_title;
    QWidget *m_header = nullptr;
    InspectorSwitch *m_enable = nullptr;
    GlyphButton *m_chevron = nullptr;
    QLabel *m_titleLabel = nullptr;
    QLabel *m_countLabel = nullptr;
    QHBoxLayout *m_trailing = nullptr;
    GlyphButton *m_reset = nullptr;
    QWidget *m_bodyWidget = nullptr;
    QVBoxLayout *m_body = nullptr;
    Summary m_summary;
    std::function<void()> m_resetHandler;
    Enable m_enableState = Enable::Hidden;
    bool m_open = false;
    bool m_changed = false;
    bool m_hovered = false;
};

// A colour wheel of the Finish workspace (.wheel .w): a 72-point circle
// with an inset 2-point `track` ring and a 1-point `track` crosshair 48 long,
// its 8-point ink puck (with the knob shadow) at the colour it pushes the
// picture towards. The puck is a unit vector, −1..1 each way within the
// disc, x right, y down. A drag moves the puck by as much as the pointer
// (a press alone changes nothing); its release is one edit. A double-click
// puts it back in the middle.
class ColorWheel final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kSize = 72;
    static constexpr int kRing = 2;
    static constexpr int kCross = 48;
    static constexpr int kPuck = 8;
    // How far from the middle the puck's centre may go, in points.
    static constexpr double kTravel = kSize / 2.0 - kRing - kPuck / 2.0 - 1.0;
    explicit ColorWheel(QWidget *parent = nullptr);
    void setPuck(const QPointF &unit);
    QPointF puck() const { return m_puck; }
    // The puck's centre in the widget.
    QPointF puckCenter() const;
    QSize sizeHint() const override { return {kSize, kSize}; }

signals:
    void puckMoved(const QPointF &unit);
    void puckReleased(const QPointF &unit, const QPointF &start);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;

private:
    QPointF m_puck;
    QPointF m_start;
    QPointF m_pressAt;
    bool m_pressed = false;
    bool m_dragging = false;
};

// The kind line's 8 by 8 square, radius 2, in the clip's timeline colour.
class KindSquare final : public QWidget {
    Q_OBJECT
public:
    explicit KindSquare(QWidget *parent = nullptr);
    void setColor(const QColor &color);
    QColor color() const { return m_color; }
    QSize sizeHint() const override { return {8, 8}; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QColor m_color;
};

// The clip's name in the clip header: UI 500 14/20 ink. Double-click (or
// Return when focused) edits it in place: Return commits, Esc reverts.
class NameLabel final : public QWidget {
    Q_OBJECT
public:
    explicit NameLabel(QWidget *parent = nullptr);
    void setName(const QString &name);
    QString name() const { return m_name; }
    void setEditable(bool editable);
    bool isEditable() const { return m_editable; }
    void beginEditing();
    QLineEdit *editor() const { return m_editor; }
    QSize sizeHint() const override;

signals:
    void renamed(const QString &name);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void endEditing(bool commit);
    QString m_name;
    bool m_editable = false;
    QLineEdit *m_editor = nullptr;
};

} // namespace kerf
