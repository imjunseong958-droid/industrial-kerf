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

// The track headers' column: badge, eye, mute and lock on one line, then
// the name, as the structure the owner chose.
constexpr int kTimelineLabelWidth = 220;
// Where time zero sits: past the track headers by more than half the
// playhead's head, so the head shows whole at the start.
constexpr int kTimelineOrigin = kTimelineLabelWidth + 6;
constexpr int kTimelineTail = 30;
constexpr int kRulerHeight = 26;
// Track heights start as tall as a dense edit allows with its pictures and
// sound in view: a picture clip is a 29px filmstrip and a sound clip a 25px
// waveform, each with its name on a chip over the corner (a name bar of its
// own only from the Large preset up). Both stay one-line headers, under
// kDetailedTrackHeight. Six tracks fit the timeline at 1280x800, 1366x768
// and 1440x900, and eight at Small.
constexpr int kTrackHeight = 30;
constexpr int kVideoTrackHeight = 34;
// The add-track row under the last track.
constexpr int kAddLaneHeight = 32;
// Mini: one line per track (badge, toggles, name, and clip names).
constexpr int kMinTrackHeight = 18;
constexpr int kMaxTrackHeight = 160;
// How far from the line between two headers a drag still resizes the
// upper track: a sixth of the shorter track, at least 2px and at most this.
constexpr int kResizeGrip = 4;
constexpr int kClipInset = 2;
constexpr int kTrimHandle = 8;
// Track header preview slot, right-aligned beside the track name.
constexpr int kHeaderPreviewWidth = 48;
constexpr int kHeaderPreviewHeight = 27;
// Below this a header is one line: no clip count under the name, and no preview.
constexpr int kDetailedTrackHeight = 40;

struct TrackHeightPreset {
    const char *name;
    int height; // 0: each track's default (kVideoTrackHeight or kTrackHeight).
};

// Track height steps, named the way Pro Tools names them.
constexpr TrackHeightPreset kTrackHeightPresets[] = {
    {"Mini", kMinTrackHeight}, {"Small", 26}, {"Medium", 0}, {"Large", 60}, {"Jumbo", 96}};
// The heights Option-scroll and Option-Up/Down step every track through.
constexpr int kTrackHeightSteps[] = {18, 22, 26, 30, 36, 44, 52, 60, 72, 86, 104, 128, 160};
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

// Imported media as people know it: the name of the file they imported,
// without its extension and without the content hash the copy in media/
// carries ("Beach Day-1a2b3c4d.mov" is "Beach Day"; see content_name in
// kerf-collab's media.rs). sourceFileName keeps the extension.
QString sourceName(const QJsonObject &source);
QString sourceFileName(const QJsonObject &source);
// Every source's name by id, told apart where they would read the same:
// two media named "Beach Day" are "Beach Day.mov" and "Beach Day.wav", and
// two "Interview.mov" keep their copies' names, hash and all, so the bin,
// the timeline and an @mention never name two media alike.
QHash<QString, QString> sourceNames(const QJsonArray &sources);

// The editor's one monospaced face (the system's: SF Mono on macOS), for
// timecode and other figures that must not shift as they change.
QFont monoFont(int pixelSize, QFont::Weight weight = QFont::Normal);

QSize resolvedCanvasSize(const QJsonObject &project, const QJsonArray &sources);

// The frame sizes offered as presets, and how every format picker (the
// toolbar's, the Inspector's Frame size) names a size: "1:1 · 1080 × 1080"
// (its shape as the window's subtitle names it, thin spaces round the ×),
// "Custom · 1280 × 720" for any size that is not a preset.
struct CanvasPreset {
    const char *ratio;
    int width;
    int height;
};
QList<CanvasPreset> canvasPresets();
QString canvasSizeLabel(const QSize &size);

// Model colors are #rrggbb, with a trailing alpha byte when not opaque.
QColor modelColor(const QString &hex);

QString modelColorName(const QColor &color);

bool isAudioTrack(const QJsonObject &track);
// A track as the timeline's header names it: its badge by position (picture
// tracks count V1, V2… in the project's order, a subtitle track among them;
// sound tracks A1, A2…) and its name, "Video 2" or "Audio 1" without one.
// The Inspector says the same, so the two never disagree.
struct TrackBadge {
    QString badge;
    QString name;
    QString label() const { return badge.isEmpty() ? name : badge + QLatin1Char(' ') + name; }
};
TrackBadge trackBadge(const QJsonObject &project, const QString &trackId);

// Pixel-exact border drawn inside the rectangle, independent of pen geometry.
void frameInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color);

// macOS's up and down chevrons (a pop-up button's, or a stepper's),
// centred on `mid`, each reaching `reach` points from its base.
void drawUpDownChevrons(QPainter &painter, const QPointF &mid, const QColor &color, qreal reach);

// A pop-up button's chevrons in `color` as an image file the style sheet
// can name (with an @2x twin for Retina screens), written once per colour
// per run; empty if it cannot be written.
QString popUpChevronsImagePath(const QColor &color);
// The same for a checked check box's tick, drawn in `color` (`onAccent`).
QString checkMarkImagePath(const QColor &color);
// The same for a dock tab's close symbol (an ×) and its area's tabs menu
// (a ⌄). The dock manager's own symbols are SVG files, which a Qt without
// its SVG plugin (as Linux distributions split it out) leaves blank.
QString closeMarkImagePath(const QColor &color);
QString tabsMenuImagePath(const QColor &color);
// And for a menu's check (in its 18-point check column) and a submenu's
// chevron, so the menus Qt draws mark choices as the pop-ups do.
QString menuCheckImagePath(const QColor &color);
QString menuChevronImagePath(const QColor &color);

// macOS's menu check, starting at `at` on the row's middle (a pop-up's
// rows, and the menus' check column).
void drawMenuCheck(QPainter &painter, const QPointF &at, const QColor &color);

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
// and pop-up buttons that take focus from Tab alone, as macOS's do, so a
// click leaves none behind, and menus without symbols. Every pop-up button
// gets PopupItemDelegate's rows (unless it brings its own), pop-up lists
// and menus are clipped to their rounded corners, and a field being typed
// in, or a button or pop-up focused from the keyboard, shows macOS's soft
// accent ring outside its bezel (its `bezelRadius` property, 5 points
// unless set; 3 for a multi-line field). A window or frame whose
// `ringsOwnFields` property is true (the launcher, the Assistant's
// composer) shows its fields' and pop-ups' focus itself. Called once,
// before any widget is made.
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

// Cache source coordinates use device pixels while the paint target uses
// logical pixels. The cache's top left sits at `origin`.
void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip, const QPoint &origin = QPoint());

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
// The pen mask tool's symbol, a nib, centred on `center` in a 16pt box.
void drawPenNib(QPainter &painter, const QPointF &center, const QColor &color);

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
