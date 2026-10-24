#pragma once

#include <QIcon>
#include <QJsonArray>
#include <QJsonObject>
#include <QPainter>
#include <QPainterPath>
#include <QPixmap>
#include <QTransform>
#include <QWidget>

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

QColor trackAccent(const QJsonObject &track, int row);

// Model colors are #rrggbb, with a trailing alpha byte when not opaque.
QColor modelColor(const QString &hex);

QString modelColorName(const QColor &color);

bool isAudioTrack(const QJsonObject &track);

// Clip surfaces derive from the track hue: a mid-tone name bar over a dark
// body, so picture, filmstrip, and waveform carry the brightness instead.
struct ClipColors {
    QColor band;
    QColor body;
    QColor wave;
    QColor highlight;
};

ClipColors clipColors(const QColor &accent, bool selected, bool hovered);

// Pixel-exact border drawn inside the rectangle, independent of pen geometry.
void frameInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color);

// Cache source coordinates use device pixels while the paint target uses logical pixels.
void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip);

enum class TransportGlyph { Start, PreviousFrame, Play, Pause, NextFrame, End };

// Transport symbols drawn on a 16-point grid so they share weight and size.
QIcon transportIcon(TransportGlyph glyph);

// What a drag or click on the Program monitor draws: nothing, or a new mask
// on the selected picture clip.
enum class MaskTool { None, Rectangle, Ellipse, Pen };

// Mask tool symbols on the transport icons' grid.
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
