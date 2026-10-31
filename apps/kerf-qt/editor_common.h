#pragma once

#include <QDoubleSpinBox>
#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QStyledItemDelegate>
#include <QTransform>
#include <QWidget>

#include <functional>

class QComboBox;

namespace kerf {

constexpr int kTimelineLabelWidth = 196;
constexpr int kTimelineTail = 30;
constexpr int kRulerHeight = 26;
// Medium: the sound-track default and the add-track row.
constexpr int kTrackHeight = 44;
// Large: picture tracks start taller so the filmstrip reads.
constexpr int kVideoTrackHeight = 60;
constexpr int kMinTrackHeight = 24;
constexpr int kMaxTrackHeight = 160;
constexpr int kResizeGrip = 4;
constexpr int kClipInset = 2;
constexpr int kTrimHandle = 8;
// Track header preview slot, right-aligned beside the track name.
constexpr int kHeaderPreviewWidth = 48;
constexpr int kHeaderPreviewHeight = 27;

struct TrackHeightPreset {
    const char *name;
    int height;
};

// Track height steps, named the way Pro Tools names them.
constexpr TrackHeightPreset kTrackHeightPresets[] = {
    {"Mini", kMinTrackHeight}, {"Small", 34}, {"Medium", kTrackHeight}, {"Large", kVideoTrackHeight}, {"Jumbo", 96}};
// One pixel stays under a frame, so the pointer time cannot step in multi-second jumps.
constexpr double kMinPixelsPerSecond = 80.0;

// While the pointer is driving the viewer, keep a frame when it is closer to
// the target than the one already on screen. Equal distance still replaces so
// an exact decode can take over a keyframe labeled at the same time.
bool nearerScrubFrame(double shownTime, bool hasShown, double frameTime, double target, double fps);

QString validateInboundMcpConfig(const QByteArray &raw);

QString mcpExecutable(const QString &projectDirectory);

QString prettyTime(double seconds);

QString timelineTimecode(double seconds, double fps);

QString timelineRulerLabel(double seconds, double fps, double duration);

struct RulerScale {
    double majorSeconds = 1.0;
    int divisions = 10;
};

RulerScale rulerScale(double pixelsPerSecond, double fps);

QString clipName(const QJsonObject &clip);

QString sourceName(const QJsonObject &source);

QSize resolvedCanvasSize(const QJsonObject &project, const QJsonArray &sources);

// Model colors are #rrggbb, with a trailing alpha byte when not opaque.
QColor modelColor(const QString &hex);

QString modelColorName(const QColor &color);

bool isAudioTrack(const QJsonObject &track);

// Pixel-exact border drawn inside the rectangle, independent of pen geometry.
void frameInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color);

// macOS's up and down chevrons (a pop-up button's, or a stepper's),
// centred on `mid`, each reaching `reach` points from its base.
void drawUpDownChevrons(QPainter &painter, const QPointF &mid, const QColor &color, qreal reach);

// A pop-up button's chevrons in `color` as an image file the style sheet
// can name (with an @2x twin for Retina screens), written once per colour
// per run; empty if it cannot be written.
QString popUpChevronsImagePath(const QColor &color);

// A seconds field with macOS's stepper: one pair of chevrons centred at its
// right edge, over the style sheet's two transparent halves that step it
// (the launcher's and the editor's spin boxes).
class StepperSpinBox : public QDoubleSpinBox {
public:
    using QDoubleSpinBox::QDoubleSpinBox;

protected:
    void paintEvent(QPaintEvent *event) override;
    // The chevrons' colour: the editor's secondary text, or its disabled text.
    virtual QColor stepperColor() const;
};

// The rows of a pop-up button's menu, as macOS draws them: 22 points tall,
// a check by the current choice, and the row under the pointer filled with
// the accent under white text (the editor's pop-ups, and the launcher's).
class PopupItemDelegate : public QStyledItemDelegate {
public:
    explicit PopupItemDelegate(QComboBox *combo);

    QSize sizeHint(const QStyleOptionViewItem &option, const QModelIndex &index) const override;
    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override;

protected:
    struct Colors {
        QColor selection;     // The row under the pointer...
        QColor selectionText; // ...and its text.
        QColor text;          // Every other row's text.
    };
    // The editor's menu colours.
    virtual Colors colors() const;

private:
    QComboBox *m_combo;
};

// Draw the application in the editor's style: Fusion, with push buttons
// that take focus from Tab alone, as macOS's do, so a click leaves none
// behind. Every pop-up button gets PopupItemDelegate's rows (unless it
// brings its own), pop-up lists and menus are clipped to their rounded
// corners, and a button focused from the keyboard shows macOS's soft accent
// ring outside its bezel (its `bezelRadius` property, 5 points unless set).
// Called once, before any widget is made.
void useEditorStyle();

// SF Symbols' waveform: six round-capped bars on an 18-point grid scaled to
// `size` points, centred on `center`, in a 1.5-point pen (sound with no
// waveform yet, or offline).
void drawWaveformSymbol(QPainter &painter, const QPointF &center, qreal size, const QColor &color);

// A symbol `draw`n in the editor's glyph colour on a `points`-sized grid,
// with a Disabled twin drawn in its disabled glyph colour. Qt's own
// disabled pixmap lightens on a dark palette, which would make an
// unavailable command look brighter than an available one.
QIcon glyphIcon(const QSize &points, const std::function<void(QPainter &, const QColor &)> &draw);
// One such symbol in `color`, at twice its size for Retina screens.
QPixmap glyphPixmap(const QSize &points, const std::function<void(QPainter &, const QColor &)> &draw, const QColor &color);

// A disclosure chevron: › closed, ⌄ open, at the icon's left edge so it
// lines up with the content's margin (the launcher's Details row, the
// editor's Advanced and Details).
QIcon disclosureIcon(bool open, const QColor &color);

// SF Symbols' exclamationmark.triangle.fill at `size` points: a rounded
// triangle in `color` with the mark cut out of it.
QPixmap warningTriangle(const QColor &color, int size = 14);

// Cache source coordinates use device pixels while the paint target uses logical pixels.
void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip);

enum class TransportGlyph { Start, PreviousFrame, Play, Pause, NextFrame, End, MarkIn, MarkOut, Insert, Overwrite };

// Transport symbols drawn on a 16-point grid so they share weight and size:
// the Program monitor's playback, and the Source monitor's marks and edits.
QIcon transportIcon(TransportGlyph glyph);

// What a drag or click on the Program monitor draws: nothing, or a new mask
// on the selected picture clip.
enum class MaskTool { None, Rectangle, Ellipse, Pen };

// Mask tool symbols on the transport icons' grid; a chosen (checked) tool's
// symbol is the accent, as in the tool palette.
QIcon maskToolIcon(MaskTool tool);

// A clip property at `clipTime` seconds into the clip: its static value,
// its keyframes' value there (linear between keys), or `fallback`.
double propertyAt(const QJsonObject &props, const QString &name, double clipTime, double fallback);

// Where a clip's picture lands on the canvas, as the compositor places it:
// fitted to the canvas when `fit` (media and nested sequences), cropped,
// scaled, rotated about its anchor, and moved. It maps fractions of the
// `picture`-pixel picture (0, 0 its top left, 1, 1 its bottom right), the
// space masks are drawn in, to canvas pixels.
QTransform picturePlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas,
                            double clipTime);

// A snapshot mask's outline, in fractions of the clip's picture; open
// paths are not closed.
QPainterPath maskPath(const QJsonObject &mask);

} // namespace kerf
