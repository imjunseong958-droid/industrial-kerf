#pragma once

#include "editor_common.h"
#include "inspector_presentation.h"

#include <QAbstractButton>
#include <QLabel>
#include <QPointer>
#include <QWidget>

#include <functional>

class QHBoxLayout;
class QLineEdit;
class QTimer;
class QVBoxLayout;

namespace kerf {

// The Inspector's controls, painted from the editor theme's hardware
// finish: values sit in recessed wells (a value that can be changed is in
// a well; plain text cannot be), choices are latching keys in a bank, the
// clip's timing is a display glass, and a jewel lamp marks what moved from
// its default. Purple is the focus ring's, and a changed value's ink.

// The ink a value moved from its default is drawn in: the accent, 4.5:1 on
// the well it sits in.
QColor changedValueInk();

// The one control behind every number and timecode: the value right-aligned
// in tabular figures with its unit after it. Drag across it (or its row's
// label) to scrub, one step per 2 points (⇧ ×10, ⌥ ÷10); click to type;
// ↑↓ step while focused or typing. A gesture is one edit: the value follows the
// pointer and `committed` fires once, on release; a run of ↑↓ presses (a
// held key, or presses under 400 ms apart) is one edit too, committed
// 400 ms after the last or when the field loses focus. A click or a drag
// takes no keyboard focus (only Tab and typing do), so the transport keys
// keep working; Return or Esc after typing hands the keys back
// (`doneEditing`).
class ScrubField final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    static constexpr int kHeight = 24;

    explicit ScrubField(QWidget *parent = nullptr);
    // Room kept clear at the right end, for a glyph inside the field (the
    // speed presets' ⌄).
    void setTrailingInset(int inset);

    void setFormat(const NumberFormat &format);
    const NumberFormat &format() const { return m_format; }
    // The value in display units; it does not emit.
    void setValue(double display);
    double value() const { return m_value; }
    // The value a reset restores, so a value at it is drawn quieter.
    void setDefaultValue(double display);
    bool isDefault() const;
    // Several clips disagree: "Mixed", and scrubbing moves each by the same amount.
    void setMixed(bool mixed);
    bool isMixed() const { return m_mixed; }
    // Its effect is switched off: the value in the secondary ink (it still edits).
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // Behind the display glass (the clip's timing): no well, its figures in
    // the glass's cream, in the monospaced face.
    void setGlass(bool glass);
    bool isGlass() const { return m_glass; }
    // The ink its value is drawn in (the accent once moved from its
    // default), and which side it keeps to (left for timecode, as the
    // timing row reads, right for everything else).
    QColor valueInk() const;
    Qt::Alignment valueAlignment() const;
    // Read-only: an animated value, shown at the playhead.
    void setReadOnly(bool readOnly);
    bool isReadOnly() const { return m_readOnly; }
    QString text() const;
    // Whether its value, unit and prefix fit it whole (no ellipsis).
    bool showsWhole() const;
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
    double padding() const;
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
    bool m_glass = false;
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

// A slider paired with a ScrubField (the field is the source of truth): a
// 4pt groove filled in the neutral ink from its origin (the default, or the
// centre of a bipolar range) to the knob, with a detent tick at the
// default the knob catches on exactly one pointer pixel. A gain fader maps
// its travel so 0 dB sits at 75%. A press never jumps the value, on the
// knob or anywhere on the groove: a drag moves the knob by as much as the
// pointer moves, so a stray click or an off-centre grab changes nothing.
class InspectorSlider final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    static constexpr double kGrab = 6.0;
    explicit InspectorSlider(QWidget *parent = nullptr);
    void setRange(double minimum, double maximum);
    void setOrigin(double origin);
    void setDetent(double detent);
    void setGainTravel(bool gain);
    void setValue(double value);
    double value() const { return m_value; }
    // Several clips disagree: the knob hides.
    void setMixed(bool mixed);
    // Its effect is switched off: the fill in the secondary ink (it still edits).
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // The ink its fill is drawn in.
    QColor fillInk() const;
    QSize sizeHint() const override { return {92, ScrubField::kHeight}; }
    // The knob's centre, for tests.
    QPointF knobCenter() const;
    // The fill from the origin to the knob's centre; empty at the origin.
    static constexpr double kKnobRadius = 6.5;
    QRectF fillRect() const;
    // The 1pt mark at the default. When the default lies under the knob it
    // reaches kTickBeyondKnob past the knob's top and bottom, so its ends
    // show; nothing is drawn on the knob.
    static constexpr int kTickBeyondKnob = 3;
    QRect detentTick() const;
    // The knob's edge: `sliderHandleChanged` away from the default (a small
    // change shows there, the edge standing out against groove and panel),
    // the quieter `sliderHandleAtDefault` at it.
    QColor knobEdge() const;
    // The default catches the pointer only on the one pixel nearest it
    // (half a pixel of pointer travel either side), so the default is easy
    // to land on and every other pixel still gives its own value.
    static constexpr double kSnap = 0.5;
    // With ⌥ held the knob moves a tenth of the pointer's travel.
    static constexpr double kFine = 10.0;
    // The value at `x` along the groove; `scale` is how far the knob moves
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
    // Where the knob goes for the pointer at `x` (with `modifiers`) during
    // a drag, and the value there.
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
    // Where the press began and the knob was then; the knob moves by the
    // pointer's travel from there.
    double m_pressX = 0.0;
    double m_pressKnob = 0.0;
    // ⌥ held: the knob moves by a tenth of the pointer's travel.
    bool m_fine = false;
};

// An on/off switch, 26 by 16: the neutral ink with its knob when on, the
// launcher's off plate when off. Space toggles it when focused.
class InspectorSwitch final : public QAbstractButton {
    Q_OBJECT
public:
    explicit InspectorSwitch(QWidget *parent = nullptr);
    QSize sizeHint() const override { return {26, 16}; }

protected:
    void paintEvent(QPaintEvent *event) override;
};

// Two or three choices in a recessed well, the chosen one a raised key
// (its legend in the accent when it is not the default). None is chosen
// when the values match no choice.
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
    // The choice a reset restores: any other is drawn as a change.
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

// A bank of latching keys in their recessed well, 24 high as the fields:
// a key held in is latched, its amber bar lit. In an exclusive bank one key
// at most is in (none when the values match none, as Fit and Fill after a
// move); otherwise each latches on its own (Flip's H and V). A click emits
// `activated` with the key's index and latches it at once; the model's
// next snapshot says what stays in.
class KeyBank final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kPadding = 2; // Round the keys, and between them.
    struct Key {
        QString label;
        QString tip;
        QString name; // The key's object name.
    };
    KeyBank(const QList<Key> &keys, bool exclusive, QWidget *parent = nullptr);
    int count() const { return int(m_keys.size()); }
    QAbstractButton *key(int index) const { return m_keys.value(index); }
    bool isExclusive() const { return m_exclusive; }
    // Exclusive: the key that is in (−1 for none).
    void setCurrent(int index);
    int current() const;
    void setLatched(int index, bool latched);
    bool isLatched(int index) const;
    QSize sizeHint() const override;

signals:
    void activated(int index);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QList<QAbstractButton *> m_keys;
    bool m_exclusive;
};

// An effect's power key: a small raised key carrying a jewel lamp, held in
// with its lamp lit green while the effect is on, out and dark while off.
// Space toggles it when focused.
class PowerKey final : public QAbstractButton {
    Q_OBJECT
public:
    explicit PowerKey(QWidget *parent = nullptr);
    QSize sizeHint() const override { return {22, 20}; }
    LampState lampState() const { return isChecked() ? LampState::Ok : LampState::Off; }

protected:
    void paintEvent(QPaintEvent *event) override;
};

// A jewel lamp (paintLamp) `diameter` across, centred in its widget.
class InspectorLamp final : public QWidget {
    Q_OBJECT
public:
    explicit InspectorLamp(qreal diameter, QWidget *parent = nullptr);
    void setState(LampState state);
    LampState state() const { return m_state; }
    qreal diameter() const { return m_diameter; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    qreal m_diameter;
    LampState m_state = LampState::Off;
};

// A bakelite knob beside Rotation (paintKnob), its pointer at the angle.
// Drag up to turn it clockwise, down anticlockwise, 1° a point (⇧ in 15°
// steps, ⌥ a tenth as fast), one edit on release; the wheel steps 1° (⇧
// 15°), a run of steps one edit; a double-click turns it back to 0°.
class RotationKnob final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kSize = 28;
    // The pause after the last wheel step that ends its run (one edit).
    static constexpr int kWheelRunMs = 400;
    explicit RotationKnob(QWidget *parent = nullptr);
    void setAngle(double degrees);
    double angle() const { return m_angle; }
    // Its field's precision: an angle turned to is rounded to it.
    void setDecimals(int decimals) { m_decimals = decimals; }
    QSize sizeHint() const override { return {kSize, kSize}; }
    // A run of wheel steps sent now rather than after its pause.
    void flushWheel();
    bool isWheeling() const { return m_wheeling; }

signals:
    void turned(double degrees);
    void released(double degrees, double start);
    // A double-click: every clip back to 0°, not only the one it shows.
    void resetRequested();

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;

private:
    double dragAngle(double y, Qt::KeyboardModifiers modifiers) const;
    double rounded(double degrees) const;
    double m_angle = 0.0;
    double m_start = 0.0;
    double m_pressY = 0.0;
    int m_decimals = 1;
    bool m_pressed = false;
    bool m_dragging = false;
    bool m_wheeling = false;
    int m_wheelRest = 0; // Wheel travel short of a whole step.
    QTimer *m_wheelRun = nullptr;
};

// The clip's Start, End and Duration behind one display glass
// (paintGlass): each column's small label ("IN", "OUT", "DUR") in the
// glass's label ink over its figures in cream.
class GlassReadout final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kLabelBand = 13; // The labels' band over the figures.
    static constexpr int kPadding = 4;    // Inside the glass, and between columns.
    explicit GlassReadout(QWidget *parent = nullptr);
    void addColumn(const QString &label, QWidget *value);
    // Removes (and deletes) every column.
    void clearColumns();
    QStringList labels() const;
    // The labels' font: 9px bold capitals, tracked.
    static QFont labelFont(const QFont &base);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QHBoxLayout *m_layout = nullptr;
    QList<QPair<QString, QPointer<QWidget>>> m_columns;
};

// A bare symbol button (no bezel): a round wash under the pointer.
class GlyphButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Glyph { Reset, More, Plus, Chevron, Reverse, Menu, Diamond, Rectangle, Ellipse, Pen, Folder, Bubble };
    explicit GlyphButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    QSize sizeHint() const override;
    // The disc behind the glyph (invalid when none) and the glyph's ink: a
    // toggle off is a quiet secondary glyph with no disc; on, full ink on
    // the neutral pressed disc, never an accent wash.
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

// A row's label: drag across it to scrub the row's field, double-click it
// to reset the row. Always one line at 12 points: one too long for the
// label column ends in an ellipsis (the whole of it in its tooltip).
class RowLabel final : public QLabel {
    Q_OBJECT
public:
    explicit RowLabel(const QString &text, QWidget *parent = nullptr);
    void setField(ScrubField *field) { m_field = field; }
    ScrubField *field() const { return m_field; }

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

// One 32pt row on the column's one grid: a 76pt label column, 8 points,
// the controls, then a 16pt reset lane 4 points after them. While the
// value differs from its default a small amber lamp rests in the lane,
// turning into ↺ under the pointer. Every row's controls end at the same
// edge (the lane's), every value field is 72 points wide (room for "+0.40
// EV" and "−8.0 dB" whole), and a slider fills what its field leaves (84
// points at a 300-point column, so it is a control, not a decoration). The
// row presses a recessed well behind each of its pop-ups and text fields.
class InspectorRow final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeight = 32;
    static constexpr int kLabelWidth = 76;
    static constexpr int kResetLane = 16;
    static constexpr int kValueField = 72;
    static constexpr qreal kLampDiameter = 5;

    explicit InspectorRow(const QString &label, QWidget *parent = nullptr);
    RowLabel *label() const { return m_label; }
    QHBoxLayout *controls() const { return m_controls; }
    GlyphButton *resetButton() const { return m_reset; }
    InspectorLamp *lamp() const { return m_lamp; }
    void setChanged(bool changed);
    bool isChanged() const { return m_changed; }
    // An animated value: ◆ in the reset lane, and its tooltip.
    void setAnimated(bool animated, const QString &tip = QString());
    bool isAnimated() const { return m_animated; }
    void setResetHandler(std::function<void()> reset);
    void reset();
    // Where the row presses a well: behind each pop-up and text field of
    // its controls (those marked `inspector`), in the row's coordinates.
    QList<QRect> wells() const;

protected:
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;

private:
    void updateLane();
    RowLabel *m_label = nullptr;
    QHBoxLayout *m_controls = nullptr;
    GlyphButton *m_reset = nullptr;
    InspectorLamp *m_lamp = nullptr;
    QLabel *m_diamond = nullptr;
    std::function<void()> m_resetHandler;
    bool m_changed = false;
    bool m_animated = false;
    bool m_hovered = false;
};

// A module: a framed plate holding a header (a jewel lamp, lit amber when
// anything in it differs from its default, the title engraved, an
// optional count, the summary of what changed, its trailing buttons, ↺ on
// hover when anything in it differs, and the chevron) over its rows.
// A click on the header folds or opens it; ⌥-click opens or closes every
// module; ⌘-click opens this one and closes the rest. Folded modules one
// after another stand together as one rack, a hairline between them; any
// other module keeps a gap under it.
class InspectorSection final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeaderHeight = 24;
    // Inside the plate: round its header and rows, and under its rows.
    static constexpr int kInset = 8;
    static constexpr int kFoot = 4;
    // Under a module that does not join the next.
    static constexpr int kGap = 4;
    static constexpr qreal kLampDiameter = 6;

    explicit InspectorSection(const QString &id, const QString &title, QWidget *parent = nullptr);
    QString id() const { return m_id; }
    QString title() const { return m_title; }
    QWidget *header() const { return m_header; }
    QVBoxLayout *body() const { return m_body; }
    QWidget *bodyWidget() const { return m_bodyWidget; }
    InspectorLamp *lamp() const { return m_lamp; }
    QLabel *titleLabel() const { return m_titleLabel; }
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    void setCount(int count);
    // What changed, as whole items: as many as fit, joined by ", ", then
    // " +N" for the rest, then `suffix` (" · 1 off"); when not even the
    // first fits, `compact` ("1 on · 1 off"); else nothing. Never cut
    // mid-word. `full` is the tooltip and summary().
    struct Summary {
        QStringList items;
        QString suffix;
        QString compact;
        QString full;
    };
    void setSummary(const Summary &summary);
    // One item, shown whole or not at all.
    void setSummary(const QString &summary);
    QString summary() const { return m_summary.full; }
    // What the folded header shows now.
    QString shownSummary() const;
    // The longest form that fits `room` points in `metrics`.
    static QString fitSummary(const Summary &summary, const QFontMetrics &metrics, int room);
    void addTrailing(QWidget *widget);
    void setChanged(bool changed);
    bool isChanged() const { return m_changed; }
    void setResetHandler(std::function<void()> reset);
    GlyphButton *resetButton() const { return m_reset; }
    QSize headerSizeHint() const;
    // Whether its plate runs on into the module above, or the one below, as
    // one rack (its gap under it goes when it joins the next).
    void setJoined(bool above, bool below);
    bool joinsAbove() const { return m_joinAbove; }
    bool joinsBelow() const { return m_joinBelow; }

signals:
    // A click on the header; `modifiers` say whether all, or only this.
    void toggled(bool open, Qt::KeyboardModifiers modifiers);
    // Opened or folded, by a click or from code.
    void foldChanged(bool open);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void paintEvent(QPaintEvent *event) override;

private:
    void updateHeader();
    QString m_id;
    QString m_title;
    QWidget *m_header = nullptr;
    InspectorLamp *m_lamp = nullptr;
    GlyphButton *m_chevron = nullptr;
    QLabel *m_titleLabel = nullptr;
    QLabel *m_countLabel = nullptr;
    QLabel *m_summaryLabel = nullptr;
    QHBoxLayout *m_trailing = nullptr;
    GlyphButton *m_reset = nullptr;
    QWidget *m_bodyWidget = nullptr;
    QVBoxLayout *m_body = nullptr;
    Summary m_summary;
    std::function<void()> m_resetHandler;
    bool m_open = false;
    bool m_changed = false;
    bool m_hovered = false;
    bool m_joinAbove = false;
    bool m_joinBelow = false;
};

// The thumbnail of the identity header: 64 by 36, radius 4, a hairline;
// several clips stack up to three, 4 points apart.
class InspectorThumbnail final : public QWidget {
    Q_OBJECT
public:
    explicit InspectorThumbnail(QWidget *parent = nullptr);
    void setImages(const QList<QImage> &images);
    QList<QImage> images() const { return m_images; }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QList<QImage> m_images;
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

// The clip's name in the identity header; double-click (or Return when
// focused) edits it in place: Return commits, Esc reverts.
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
