#include "scope_math.h"

#include <algorithm>
#include <cmath>
#include <utility>

namespace kerf::scope {

namespace {

// A small deterministic generator, one per call, so calls share nothing.
struct Jitter {
    quint32 state;
    explicit Jitter(quint32 seed) : state(seed ? seed : 0x9e3779b9u) {}
    double uniform() { // 0..1
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        return (state >> 8) * (1.0 / 16777216.0);
    }
    // The design's bell: three uniforms summed, centred and widened (sd ~ 0.7).
    double bell() { return (uniform() + uniform() + uniform() - 1.5) * 1.4; }
};

// The frame as RGBA bytes, converted only when it is in another format.
struct Pixels {
    QImage holder;
    const uchar *bits = nullptr;
    qsizetype stride = 0;
    int width = 0;
    int height = 0;

    explicit Pixels(const QImage &frame) {
        holder = frame;
        if (frame.format() != QImage::Format_RGBA8888 && frame.format() != QImage::Format_RGBX8888)
            holder = frame.convertToFormat(QImage::Format_RGBA8888);
        bits = holder.constBits();
        stride = holder.bytesPerLine();
        width = holder.width();
        height = holder.height();
    }
    const uchar *pixel(int x, int y) const { return bits + y * stride + x * 4; }
};

Density emptyDensity(int width, int height) {
    Density d;
    d.width = qMax(0, width);
    d.height = qMax(0, height);
    d.counts.assign(size_t(d.width) * size_t(d.height), 0);
    return d;
}

void hit(Density &d, int x, int y) {
    if (x < 0 || y < 0 || x >= d.width || y >= d.height) return;
    quint32 &cell = d.counts[size_t(y) * size_t(d.width) + size_t(x)];
    ++cell;
    d.peak = std::max(d.peak, cell);
    ++d.total;
}

// One waveform-style trace: column from x, row from the channel's code.
template <typename Channel>
void trace(Density &d, const Pixels &px, const Options &options, quint32 seed, Channel channel) {
    if (d.width == 0 || d.height == 0 || px.width == 0 || px.height == 0) return;
    Jitter jitter(seed);
    const int samples = qMax(1, options.samplesPerPixel);
    const double columns = double(d.width) / px.width;
    for (int y = 0; y < px.height; ++y) {
        for (int x = 0; x < px.width; ++x) {
            const uchar *p = px.pixel(x, y);
            const double value = channel(p[0], p[1], p[2]);
            for (int s = 0; s < samples; ++s) {
                const int column = int((x + jitter.uniform()) * columns);
                const double v = std::clamp(value + jitter.bell() * options.codeJitter, 0.0, 1.0);
                hit(d, std::min(column, d.width - 1), codeRow(code10(v), d.height));
            }
        }
    }
}

constexpr std::array<std::pair<Target, const char *>, 6> kTargetNames{
    {{Target::R, "R"}, {Target::Mg, "Mg"}, {Target::B, "B"}, {Target::Cy, "Cy"}, {Target::G, "G"}, {Target::Yl, "Yl"}}};

QRgb barColor(Target target) {
    constexpr int v = 191; // 75% of 255, rounded
    switch (target) {
    case Target::R: return qRgb(v, 0, 0);
    case Target::Mg: return qRgb(v, 0, v);
    case Target::B: return qRgb(0, 0, v);
    case Target::Cy: return qRgb(0, v, v);
    case Target::G: return qRgb(0, v, 0);
    case Target::Yl: return qRgb(v, v, 0);
    }
    return 0;
}

// The exact 75% chroma (the design computes it from 0.75 x 255, not 191).
Chroma barChroma(Target target) {
    const QRgb c = barColor(target);
    const double r = qRed(c) ? 0.75 : 0, g = qGreen(c) ? 0.75 : 0, b = qBlue(c) ? 0.75 : 0;
    const double y = kr * r + kg * g + kb * b;
    return {(b - y) / 1.8556, (r - y) / 1.5748};
}

} // namespace

int code10(double value) { return int(std::lround(std::clamp(value, 0.0, 1.0) * 1023.0)); }

Chroma chroma(int r, int g, int b) {
    const double rf = r / 255.0, gf = g / 255.0, bf = b / 255.0;
    const double y = kr * rf + kg * gf + kb * bf;
    return {(bf - y) / 1.8556, (rf - y) / 1.5748};
}

int codeRow(int code, int height) {
    if (height <= 1) return 0;
    const int clamped = std::clamp(code, 0, 1023);
    return int(std::lround((1023 - clamped) * double(height - 1) / 1023.0));
}

Density waveformLuma(const QImage &frame, int width, int height, const Options &options) {
    Density d = emptyDensity(width, height);
    const Pixels px(frame);
    trace(d, px, options, options.seed, [](int r, int g, int b) { return luma(r, g, b); });
    return d;
}

std::array<Density, 3> parade(const QImage &frame, int width, int height, const Options &options) {
    std::array<Density, 3> cells{emptyDensity(width, height), emptyDensity(width, height), emptyDensity(width, height)};
    const Pixels px(frame);
    trace(cells[0], px, options, options.seed, [](int r, int, int) { return r / 255.0; });
    trace(cells[1], px, options, options.seed + 1, [](int, int g, int) { return g / 255.0; });
    trace(cells[2], px, options, options.seed + 2, [](int, int, int b) { return b / 255.0; });
    return cells;
}

Density vectorscope(const QImage &frame, QSize size, double gain, const Options &options) {
    return vectorscope(frame, size, vectorscopeGeometry(QSizeF(size)), gain, options);
}

Density vectorscope(const QImage &frame, QSize size, const VectorscopeGeometry &geometry, double gain,
                    const Options &options) {
    Density d = emptyDensity(size.width(), size.height());
    const Pixels px(frame);
    if (d.width == 0 || d.height == 0) return d;
    Jitter jitter(options.seed);
    const int samples = qMax(1, options.samplesPerPixel);
    for (int y = 0; y < px.height; ++y) {
        for (int x = 0; x < px.width; ++x) {
            const uchar *p = px.pixel(x, y);
            const QPointF at = geometry.point(chroma(p[0], p[1], p[2]), gain);
            for (int s = 0; s < samples; ++s) {
                const double jx = jitter.bell() * options.chromaJitter, jy = jitter.bell() * options.chromaJitter;
                hit(d, int(std::floor(at.x() + jx)), int(std::floor(at.y() + jy)));
            }
        }
    }
    return d;
}

QImage render(const Density &density, QRgb traceColor) {
    QImage out(qMax(1, density.width), qMax(1, density.height), QImage::Format_ARGB32_Premultiplied);
    out.fill(Qt::transparent);
    if (density.isEmpty()) return out;
    const double a = qAlpha(traceColor) / 255.0;
    const double r = qRed(traceColor), g = qGreen(traceColor), b = qBlue(traceColor);
    for (int y = 0; y < density.height; ++y) {
        auto *line = reinterpret_cast<QRgb *>(out.scanLine(y));
        for (int x = 0; x < density.width; ++x) {
            const quint32 n = density.counts[size_t(y) * size_t(density.width) + size_t(x)];
            if (!n) continue;
            const double k = n * a;
            auto add = [k](double channel) { return int(std::min(255.0, std::round(k * channel))); };
            line[x] = qRgba(add(r), add(g), add(b), add(255));
        }
    }
    return out;
}

QImage scopeSource(const QImage &frame, QSize bound) {
    if (frame.isNull()) return {};
    QImage out = frame;
    if (bound.isValid() && (frame.width() > bound.width() || frame.height() > bound.height()))
        out = frame.scaled(bound, Qt::KeepAspectRatio, Qt::SmoothTransformation);
    if (out.format() != QImage::Format_RGBA8888) out = out.convertToFormat(QImage::Format_RGBA8888);
    return out;
}

QRectF WaveformGeometry::cell(int index, int cells, double gap) const {
    cells = qMax(1, cells);
    const double width = (x1 - x0 - gap * (cells - 1)) / cells;
    return QRectF(x0 + index * (width + gap), top, width, bottom - top);
}

WaveformGeometry waveformGeometry(QSizeF plot) {
    WaveformGeometry g;
    g.x0 = 34;
    g.x1 = plot.width() - 6;
    g.top = 8;
    g.bottom = plot.height() - 10;
    return g;
}

VectorscopeGeometry vectorscopeGeometry(QSizeF plot) {
    VectorscopeGeometry g;
    g.centre = QPointF(plot.width() / 2, plot.height() / 2);
    g.radius = qMax(0.0, std::min(plot.width(), plot.height()) / 2 - 14);
    g.scale = g.radius / 0.5;
    return g;
}

std::array<TargetMark, 6> targets(QSizeF plot, double gain) {
    const VectorscopeGeometry geometry = vectorscopeGeometry(plot);
    std::array<TargetMark, 6> out{};
    for (size_t i = 0; i < kTargetNames.size(); ++i) {
        const auto [target, label] = kTargetNames[i];
        TargetMark &mark = out[i];
        mark.target = target;
        mark.label = label;
        mark.color = barColor(target);
        mark.centre = geometry.point(barChroma(target), gain);
        mark.box = QRectF(mark.centre - QPointF(4, 4), QSizeF(8, 8));
        const QPointF out12 = mark.centre - geometry.centre;
        const double length = std::hypot(out12.x(), out12.y());
        mark.labelCentre = length > 0 ? mark.centre + out12 / length * 12 : mark.centre;
    }
    return out;
}

QPointF targetPosition(Target target, QSizeF plot, double gain) {
    return vectorscopeGeometry(plot).point(barChroma(target), gain);
}

} // namespace kerf::scope
