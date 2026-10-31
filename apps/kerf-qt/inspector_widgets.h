#pragma once

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

// The Inspector's controls, painted from the editor theme. A fill means a
// value can be changed; plain text cannot. The only purple is the focus
// ring.

// The one control behind every number and timecode: the value right-aligned
// in tabular figures with its unit after it. Drag across it (or its row's
// label) to scrub, one step per 2 points (⇧ ×10, ⌥ ÷10); click to type;
// ↑↓ step while focused. A gesture is one edit: the value follows the
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
    // Its effect is switched off: the value in the disabled ink (it still edits).
    void setDimmed(bool dimmed);
    bool isDimmed() const { return m_dimmed; }
    // Read-only: an animated value, shown at the playhead.
    void setReadOnly(bool readOnly);
    bool isReadOnly() const { return m_readOnly; }
    QString text() const;
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
    double clamped(double display, bool *clamped = nullptr) const;
    double stepFor(Qt::KeyboardModifiers modifiers) const;
    void endTyping(bool commit);
    void flash();

    NumberFormat m_format;
    double m_value = 0.0;
    double m_default = 0.0;
    bool m_hasDefault = false;
    bool m_mixed = false;
    bool m_dimmed = false;
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
// default the knob snaps to within 4 points. A gain fader maps its travel
// so 0 dB sits at 75%.
class InspectorSlider final : public QWidget {
    Q_OBJECT
    friend class EditorTest;
public:
    explicit InspectorSlider(QWidget *parent = nullptr);
    void setRange(double minimum, double maximum);
    void setOrigin(double origin);
    void setDetent(double detent);
    void setGainTravel(bool gain);
    void setValue(double value);
    double value() const { return m_value; }
    // Several clips disagree: the knob hides.
    void setMixed(bool mixed);
    QSize sizeHint() const override { return {92, ScrubField::kHeight}; }
    // The knob's centre, for tests.
    QPointF knobCenter() const;
    double valueAt(double x) const;

signals:
    void moved(double value);
    void released(double value, double start);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    double travelOf(double value) const;
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
    bool m_dragging = false;
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

// Two or three choices on a radius-6 track; the chosen one raised with a
// hairline. None is chosen when the values match no choice.
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
    int count() const { return int(m_segments.size()); }
    QRect segmentRect(int index) const;
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
};

// A 16pt dial beside Rotation: a hand at the angle. Drag around it to turn
// (⇧ snaps to 15°); one edit on release.
class RotationDial final : public QWidget {
    Q_OBJECT
public:
    explicit RotationDial(QWidget *parent = nullptr);
    void setAngle(double degrees);
    double angle() const { return m_angle; }
    QSize sizeHint() const override { return {16, 16}; }

signals:
    void turned(double degrees);
    void released(double degrees, double start);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;

private:
    double angleAt(const QPointF &point, Qt::KeyboardModifiers modifiers) const;
    double m_angle = 0.0;
    double m_start = 0.0;
    bool m_dragging = false;
};

// A bare symbol button (no bezel): a round wash under the pointer.
class GlyphButton final : public QAbstractButton {
    Q_OBJECT
public:
    enum class Glyph { Reset, More, Plus, Chevron, Reverse, Menu, Diamond, Rectangle, Ellipse, Pen, Folder };
    explicit GlyphButton(Glyph glyph, QWidget *parent = nullptr);
    Glyph glyph() const { return m_glyph; }
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    Glyph m_glyph;
    bool m_open = false;
};

// Draw one of the glyphs into `rect` in `color`.
void drawInspectorGlyph(QPainter &painter, GlyphButton::Glyph glyph, const QRectF &rect, const QColor &color, bool open = false);

// A row's label: drag across it to scrub the row's field, double-click it
// to reset the row. One too long for the label column wraps onto a second
// line at 11 points (the whole of it in its tooltip).
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

// One 32pt row on the column's one grid: an 84pt label column, 8 points,
// the controls, then a 20pt reset lane 4 points after them, whose ↺ shows
// on hover only while the value differs from its default. Every row's
// controls end at the same edge (the lane's), every value field is 72
// points wide, and a slider fills what its field leaves.
class InspectorRow final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kHeight = 32;
    static constexpr int kLabelWidth = 84;
    static constexpr int kResetLane = 20;
    static constexpr int kValueField = 72;

    explicit InspectorRow(const QString &label, QWidget *parent = nullptr);
    RowLabel *label() const { return m_label; }
    QHBoxLayout *controls() const { return m_controls; }
    GlyphButton *resetButton() const { return m_reset; }
    void setChanged(bool changed);
    bool isChanged() const { return m_changed; }
    // An animated value: ◆ in the reset lane, and its tooltip.
    void setAnimated(bool animated, const QString &tip = QString());
    bool isAnimated() const { return m_animated; }
    void setResetHandler(std::function<void()> reset);
    void reset();

protected:
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;

private:
    void updateLane();
    RowLabel *m_label = nullptr;
    QHBoxLayout *m_controls = nullptr;
    GlyphButton *m_reset = nullptr;
    QLabel *m_diamond = nullptr;
    std::function<void()> m_resetHandler;
    bool m_changed = false;
    bool m_animated = false;
    bool m_hovered = false;
};

// A folded section: a 32pt header (a hairline above, chevron, title, an
// optional count, the summary of what changed, its trailing buttons, and
// ↺ on hover when anything in it differs), then its rows. ⌥-click opens
// or closes every section; ⌘-click opens this one and closes the rest.
class InspectorSection final : public QWidget {
    Q_OBJECT
public:
    explicit InspectorSection(const QString &id, const QString &title, QWidget *parent = nullptr);
    QString id() const { return m_id; }
    QString title() const { return m_title; }
    QWidget *header() const { return m_header; }
    QVBoxLayout *body() const { return m_body; }
    QWidget *bodyWidget() const { return m_bodyWidget; }
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    void setCount(int count);
    void setSummary(const QString &summary);
    QString summary() const { return m_summary; }
    void addTrailing(QWidget *widget);
    void setChanged(bool changed);
    void setResetHandler(std::function<void()> reset);
    GlyphButton *resetButton() const { return m_reset; }
    QSize headerSizeHint() const;

signals:
    // A click on the header; `modifiers` say whether all, or only this.
    void toggled(bool open, Qt::KeyboardModifiers modifiers);

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void updateHeader();
    QString m_id;
    QString m_title;
    QWidget *m_header = nullptr;
    GlyphButton *m_chevron = nullptr;
    QLabel *m_titleLabel = nullptr;
    QLabel *m_countLabel = nullptr;
    QLabel *m_summaryLabel = nullptr;
    QHBoxLayout *m_trailing = nullptr;
    GlyphButton *m_reset = nullptr;
    QWidget *m_bodyWidget = nullptr;
    QVBoxLayout *m_body = nullptr;
    QString m_summary;
    std::function<void()> m_resetHandler;
    bool m_open = false;
    bool m_changed = false;
    bool m_hovered = false;
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
