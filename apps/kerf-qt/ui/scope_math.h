#pragma once

// Scope maths for the Finish tray (mockup-spec 12, engine gap 7): the
// waveform, the RGB parade and the vectorscope, computed in Qt from a program
// frame (the RGBA that kerf_bridge_frame hands over).
//
// Each scope is a density: a grid of hit counts, one hit per sample, with two
// samples per source pixel spread by a slight deterministic jitter as the
// design draws them. render() turns a density into the additive trace the
// design paints (each hit adds the trace colour at its alpha, as canvas
// 'lighter' and QPainter::CompositionMode_Plus do).
//
// Everything here is a pure function of its arguments: no shared state, so
// any thread may call it, and each call allocates only its result (plus one
// converted copy when the frame is not RGBA8888 or RGBX8888).

#include <QImage>
#include <QPointF>
#include <QRectF>
#include <QRgb>
#include <QSize>
#include <QSizeF>
#include <QtGlobal>

#include <array>
#include <vector>

namespace kerf::scope {

// Rec.709 luma weights, applied to the gamma-encoded R′G′B′ (full range).
constexpr double kr = 0.2126;
constexpr double kg = 0.7152;
constexpr double kb = 0.0722;

// Y′ of an 8-bit R′G′B′ triple, 0..1.
constexpr double luma(int r, int g, int b) { return (kr * r + kg * g + kb * b) / 255.0; }

// A 0..1 value as a 10-bit code, 0..1023 (an 8-bit value v is v x 1023/255).
int code10(double value);

// Rec.709 Cb and Cr of an 8-bit R′G′B′ triple, each -0.5..0.5:
// Cb = (B′ - Y′) / 1.8556, Cr = (R′ - Y′) / 1.5748.
struct Chroma {
    double cb;
    double cr;
};
Chroma chroma(int r, int g, int b);

// A grid of hit counts, row-major, `width` x `height`.
struct Density {
    int width = 0;
    int height = 0;
    std::vector<quint32> counts;
    quint32 peak = 0;   // the largest count
    quint64 total = 0;  // hits that landed inside the grid

    quint32 at(int x, int y) const {
        return x < 0 || y < 0 || x >= width || y >= height ? 0 : counts[size_t(y) * size_t(width) + size_t(x)];
    }
    bool isEmpty() const { return total == 0; }
};

struct Options {
    int samplesPerPixel = 2;     // the design draws two per source pixel
    double codeJitter = 0.006;   // waveform and parade: spread, as a fraction of full scale
    double chromaJitter = 0.8;   // vectorscope: spread in plot points
    quint32 seed = 11;           // the jitter is deterministic for a seed
};

// The row a 10-bit code lands on in a grid `height` rows tall: code 1023 is
// row 0 (the top), code 0 the last row.
int codeRow(int code, int height);

// Waveform: columns follow the frame's x (scaled to `width`), rows are Y′
// codes 0-1023 (scaled to `height`; 1024 gives one row per code).
Density waveformLuma(const QImage &frame, int width, int height, const Options &options = {});

// RGB parade: the R′, G′ and B′ cells, each `width` x `height` laid out as the
// waveform.
std::array<Density, 3> parade(const QImage &frame, int width, int height, const Options &options = {});


// The additive trace of `density` in `trace` (a colour with its alpha, such as
// rgba(208,208,208,.09)): each hit adds the colour at that alpha, clamped.
// Premultiplied ARGB, to be drawn with QPainter::CompositionMode_Plus over the
// plot's well.
QImage render(const Density &density, QRgb trace);

// A program frame made ready for the scopes: RGBA8888, scaled down (keeping
// its aspect, smoothly) to fit `bound` when it is larger. The design measures
// a 160 x 90 grab; more samples only sharpen the trace.
QImage scopeSource(const QImage &frame, QSize bound = QSize(320, 180));

// -------------------------------------------------------------- plot geometry
// Where the design puts things inside each plot (mockup-spec 12).

// Waveform and parade plot: x0 = 34, x1 = W - 6, yT = 8, yB = H - 10; the
// graticule at codes 0/256/512/768/1023 with labels right-aligned at x0 - 4.
struct WaveformGeometry {
    double x0 = 0, x1 = 0, top = 0, bottom = 0;
    static constexpr std::array<int, 5> graticule{0, 256, 512, 768, 1023};
    double y(double code) const { return bottom - code / 1023.0 * (bottom - top); }
    // The parade's three cells, 6 apart, across x0..x1.
    QRectF cell(int index, int cells = 3, double gap = 6) const;
};
WaveformGeometry waveformGeometry(QSizeF plot);

// Vectorscope plot: the centre, the graticule radius R = min(W, H)/2 - 14 (and
// R/2), and `scale` = R / 0.5 points per unit of chroma.
struct VectorscopeGeometry {
    QPointF centre;
    double radius = 0;
    double scale = 0;
    QPointF point(Chroma c, double gain = 1) const {
        return centre + QPointF(c.cb * scale * gain, -c.cr * scale * gain);
    }
};
VectorscopeGeometry vectorscopeGeometry(QSizeF plot);

// Vectorscope over a plot of `size`: each pixel lands at
// geometry.point(chroma, gain), so neutral greys sit at the centre and Cr
// grows upward. The first form uses vectorscopeGeometry(size) (the Finish
// pane); the second takes any graticule, such as the 72 pt proposal chip's
// radius 32 about (36, 36).
Density vectorscope(const QImage &frame, QSize size, double gain = 2.2, const Options &options = {});
Density vectorscope(const QImage &frame, QSize size, const VectorscopeGeometry &geometry, double gain,
                    const Options &options = {});

// The six 75% colour-bar targets, in the design's order and with its labels.
enum class Target { R, Mg, B, Cy, G, Yl };
struct TargetMark {
    Target target;
    const char *label;  // R, Mg, B, Cy, G, Yl
    QRgb color;         // the 75% bar colour the target stands for
    QPointF centre;     // where that colour lands at the given gain
    QRectF box;         // the 8 x 8 target box around it
    QPointF labelCentre; // 12 points further out from the centre
};
std::array<TargetMark, 6> targets(QSizeF plot, double gain = 1);
QPointF targetPosition(Target target, QSizeF plot, double gain = 1);

} // namespace kerf::scope
