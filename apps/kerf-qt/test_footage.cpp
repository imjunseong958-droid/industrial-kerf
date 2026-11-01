#include "test_footage.h"

#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace kerf::testing {

namespace {

struct Rgb {
    double r = 0, g = 0, b = 0;
};

Rgb operator+(Rgb a, Rgb b) { return {a.r + b.r, a.g + b.g, a.b + b.b}; }
Rgb operator*(Rgb a, double k) { return {a.r * k, a.g * k, a.b * k}; }
Rgb mix(Rgb a, Rgb b, double t) { return a * (1.0 - t) + b * t; }

double clamp01(double v) { return std::clamp(v, 0.0, 1.0); }
double smoothstep(double edge0, double edge1, double x) {
    const double t = clamp01((x - edge0) / (edge1 - edge0));
    return t * t * (3.0 - 2.0 * t);
}

// A repeatable pseudo-random value in [0, 1) for a lattice point.
double hash(int x, int y, int seed) {
    uint32_t h = uint32_t(x) * 374761393u + uint32_t(y) * 668265263u + uint32_t(seed) * 2246822519u;
    h = (h ^ (h >> 13)) * 1274126177u;
    h ^= h >> 16;
    return double(h & 0xffffffu) / double(0x1000000);
}

// Smooth value noise in [0, 1].
double noise(double x, double y, int seed) {
    const double fx = std::floor(x), fy = std::floor(y);
    const int xi = int(fx), yi = int(fy);
    const double tx = x - fx, ty = y - fy;
    const double sx = tx * tx * (3.0 - 2.0 * tx), sy = ty * ty * (3.0 - 2.0 * ty);
    const double a = hash(xi, yi, seed), b = hash(xi + 1, yi, seed);
    const double c = hash(xi, yi + 1, seed), d = hash(xi + 1, yi + 1, seed);
    return (a + (b - a) * sx) * (1.0 - sy) + (c + (d - c) * sx) * sy;
}

// Fractal noise: `octaves` layers of noise, each twice as fine and half as strong.
double fbm(double x, double y, int seed, int octaves = 5) {
    double sum = 0.0, amplitude = 0.5, total = 0.0;
    for (int octave = 0; octave < octaves; ++octave) {
        sum += noise(x, y, seed + octave * 101) * amplitude;
        total += amplitude;
        x = x * 2.03 + 17.1;
        y = y * 1.97 + 9.3;
        amplitude *= 0.5;
    }
    return sum / total;
}

QRgb toPixel(Rgb c) {
    const auto channel = [](double v) { return int(std::lround(clamp01(v) * 255.0)); };
    return qRgb(channel(c.r), channel(c.g), channel(c.b));
}
Rgb fromPixel(QRgb p) { return {qRed(p) / 255.0, qGreen(p) / 255.0, qBlue(p) / 255.0}; }

// Fill `image` from `shade`, which takes coordinates in picture heights
// (x from 0 to the aspect ratio, y from 0 to 1).
template <typename Shade> void fill(QImage &image, Shade shade) {
    const double h = image.height();
    for (int py = 0; py < image.height(); ++py) {
        auto *line = reinterpret_cast<QRgb *>(image.scanLine(py));
        for (int px = 0; px < image.width(); ++px) line[px] = toPixel(shade((px + 0.5) / h, (py + 0.5) / h));
    }
}

// A lens and film finish: a soft vignette, a gentle tone curve, and grain
// that is stronger in the shadows.
void finish(QImage &image, int seed) {
    const double w = image.width(), h = image.height();
    const double halfDiagonal = std::hypot(w, h) / 2.0;
    for (int py = 0; py < image.height(); ++py) {
        auto *line = reinterpret_cast<QRgb *>(image.scanLine(py));
        for (int px = 0; px < image.width(); ++px) {
            Rgb c = fromPixel(line[px]);
            const double r = std::hypot(px - w / 2.0, py - h / 2.0) / halfDiagonal;
            c = c * (1.0 - 0.32 * std::pow(r, 2.2));
            const auto curve = [](double v) { return 0.025 + 0.95 * (v < 0.8 ? v : 0.8 + (v - 0.8) * 0.6); };
            c = {curve(c.r), curve(c.g), curve(c.b)};
            const double luma = 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
            const double grain = (hash(px, py, seed) + hash(px, py, seed + 7) - 1.0) * (0.032 - 0.018 * luma);
            line[px] = toPixel(c + Rgb{grain, grain * 0.96, grain * 1.04});
        }
    }
}

QImage harborWide(const QSize &size) {
    QImage image(size, QImage::Format_RGB32);
    const double aspect = double(size.width()) / size.height();
    const double pixel = 1.0 / size.height();
    const double horizon = 0.57;
    const double sunX = 0.66 * aspect, sunY = horizon - 0.08;
    const Rgb warm{1.0, 0.86, 0.70};
    const auto ridge = [](double x) { return 0.035 + 0.055 * fbm(x * 2.4, 3.1, 11, 4); };
    const auto headland = [aspect](double x) {
        return 0.11 * smoothstep(0.5 * aspect, 0.0, x) + 0.02 * fbm(x * 9.0, 5.0, 13, 3);
    };
    // How far `y` is past the edge at `edge`, over a pixel and a half, so
    // silhouettes are smooth.
    const auto past = [pixel](double y, double edge) { return smoothstep(edge - 0.75 * pixel, edge + 0.75 * pixel, y); };
    fill(image, [&](double x, double y) {
        const double sun = (x - sunX) * (x - sunX) + (y - sunY) * (y - sunY) * 1.6;
        const double glow = 0.5 * std::exp(-sun / 0.0035) + 0.22 * std::exp(-sun / 0.05);
        const double land = horizon - headland(x);
        if (y < horizon) {
            const double t = y / horizon;
            Rgb c = t < 0.55 ? mix(Rgb{0.25, 0.30, 0.39}, Rgb{0.55, 0.57, 0.62}, t / 0.55)
                             : mix(Rgb{0.55, 0.57, 0.62}, Rgb{0.86, 0.74, 0.62}, (t - 0.55) / 0.45);
            // Streaks of cloud, lit warm towards the sun and grey away from it.
            const double cloud = smoothstep(0.5, 0.78, fbm(x * 2.2, y * 9.0, 3)) * (1.0 - 0.4 * t);
            const Rgb cloudColor = mix(Rgb{0.40, 0.41, 0.47}, Rgb{0.86, 0.72, 0.62}, clamp01(1.0 - std::sqrt(sun) * 1.6));
            c = mix(c, cloudColor, cloud * 0.75);
            c = c + warm * (glow * (1.0 - cloud * 0.6));
            // Hills in the haze, and a wooded headland on the left.
            c = mix(c, Rgb{0.42, 0.45, 0.51} * (0.95 + 0.1 * fbm(x * 20.0, y * 20.0, 17)), 0.55 * past(y, horizon - ridge(x)));
            const double trees = fbm(x * 45.0, y * 45.0, 19, 4);
            return mix(c, Rgb{0.15, 0.17, 0.19} * (0.8 + 0.5 * trees), past(y, land));
        }
        const double depth = (y - horizon) / (1.0 - horizon);
        const double distance = 1.0 / (depth + 0.06);
        Rgb c = mix(Rgb{0.60, 0.60, 0.64}, Rgb{0.09, 0.16, 0.21}, std::pow(depth, 0.55));
        // Ripples, finer towards the horizon...
        const double ripple = fbm(x * distance * 1.4, distance * 7.0, 23, 4);
        c = c + Rgb{1, 1, 1} * ((ripple - 0.5) * 0.16 * (1.0 - depth * 0.4));
        // ...the headland mirrored in the water, its edge broken by them...
        const double mirrored = horizon + (horizon - land) * 0.9 + (ripple - 0.5) * 0.03;
        c = mix(c, Rgb{0.12, 0.15, 0.18} * (0.8 + 0.5 * ripple), 0.65 * (1.0 - smoothstep(mirrored - 0.01, mirrored + 0.01, y)));
        // ...and the sun's path across the water.
        const double path = std::exp(-((x - sunX) * (x - sunX)) / (0.0015 + depth * 0.03));
        const double sparkle = smoothstep(0.52, 0.75, ripple);
        c = c + warm * (path * (0.15 + 0.8 * sparkle) * (1.0 - depth * 0.7));
        return c;
    });
    // Boats at anchor: dark hulls with masts, small near the horizon.
    QPainter painter(&image);
    painter.setRenderHint(QPainter::Antialiasing);
    const double h = size.height();
    const struct { double x, y, scale; } boats[] = {{0.3, 0.61, 0.9}, {0.47, 0.595, 0.6}, {0.83, 0.66, 1.3}};
    for (const auto &boat : boats) {
        const double bx = boat.x * aspect * h, by = boat.y * h, s = boat.scale * h * 0.012;
        QPainterPath hull;
        hull.moveTo(bx - 3.2 * s, by - 0.6 * s);
        hull.lineTo(bx + 3.4 * s, by - 0.6 * s);
        hull.lineTo(bx + 2.6 * s, by + 0.5 * s);
        hull.lineTo(bx - 2.5 * s, by + 0.5 * s);
        hull.closeSubpath();
        painter.fillPath(hull, QColor(28, 30, 34));
        painter.fillRect(QRectF(bx - 1.2 * s, by - 1.5 * s, 1.8 * s, 0.9 * s), QColor(48, 46, 46));
        painter.setPen(QPen(QColor(40, 40, 44), std::max(0.6, 0.18 * s)));
        painter.drawLine(QPointF(bx + 0.2 * s, by - 0.6 * s), QPointF(bx + 0.2 * s, by - 6.0 * s));
        // Its reflection, softened by the water.
        painter.fillRect(QRectF(bx - 2.4 * s, by + 0.5 * s, 4.8 * s, 1.4 * s), QColor(20, 26, 32, 70));
    }
    painter.end();
    finish(image, 31);
    return image;
}

QImage fisherClose(const QSize &size) {
    QImage image(size, QImage::Format_RGB32);
    const double aspect = double(size.width()) / size.height();
    const double pixel = 1.0 / size.height();
    const double deck = 0.46;
    const double coilX = 0.58 * aspect, coilY = 0.74, coilRx = std::min(0.2, 0.36 * aspect), coilRy = 0.095;
    // Out-of-focus lights on the water behind: soft discs, warm and cool.
    struct Bokeh { double x, y, r; Rgb color; };
    std::vector<Bokeh> lights;
    for (int i = 0; i < 16; ++i)
        lights.push_back({hash(i, 1, 41) * aspect, 0.04 + hash(i, 2, 41) * 0.4, 0.02 + hash(i, 3, 41) * 0.045,
                          hash(i, 4, 41) < 0.65 ? Rgb{0.95, 0.78, 0.55} : Rgb{0.55, 0.72, 0.76}});
    // The rope's tail, leaving the coil for the bottom right corner.
    const auto tail = [&](double x) { return coilY + 0.05 + 0.5 * std::pow(std::max(0.0, x - coilX) / std::max(aspect - coilX, 0.01), 1.6); };
    // A rope's cross-section shading at `across` (0 to 1 over its width),
    // with its strands twisting along `along`.
    const auto rope = [](double across, double along, double x, double y) {
        const double round = std::pow(std::sin(clamp01(across) * 3.14159265358979), 0.5);
        const double strands = 0.5 + 0.5 * std::sin(along + across * 7.0);
        return Rgb{0.70, 0.60, 0.45} * (0.25 + 0.55 * round + 0.25 * strands * round) *
               (0.9 + 0.2 * fbm(x * 90.0, y * 90.0, 61, 3));
    };
    fill(image, [&](double x, double y) {
        // Behind: a blurred quay at golden hour over darker water.
        Rgb c = mix(Rgb{0.48, 0.40, 0.32}, Rgb{0.17, 0.25, 0.27}, smoothstep(0.0, deck, y));
        c = c * (0.85 + 0.3 * fbm(x * 1.5, y * 1.5, 43, 3));
        for (const Bokeh &light : lights) {
            const double d = std::hypot(x - light.x, y - light.y) / light.r;
            c = c + light.color * (0.2 * (1.0 - smoothstep(0.8, 1.0, d)) * (0.7 + 0.3 * d));
        }
        // In front: weathered planks running away from us towards a point
        // above the frame, their grain along them, dark seams between.
        const double edge = smoothstep(deck - pixel, deck + pixel, y);
        if (edge > 0.0) {
            const double vx = 0.5 * aspect, vy = deck - 0.9;
            const double along = 1.0 / (y - vy);
            const double across = (x - vx) * along * 7.0;
            const int index = int(std::floor(across));
            const double within = across - index;
            const Rgb wood = mix(Rgb{0.47, 0.36, 0.27}, Rgb{0.57, 0.53, 0.47}, 0.25 + 0.6 * hash(index, 0, 47));
            const double grain = fbm(within * 3.0 + index * 5.3, along * 14.0, 53, 5);
            const double fibres = fbm(within * 40.0 + index * 3.1, along * 3.0, 59, 4);
            Rgb boards = wood * (0.7 + 0.45 * grain) * (0.88 + 0.24 * fibres);
            const double seam = smoothstep(0.93, 0.985, within) + (1.0 - smoothstep(0.0, 0.03, within)) * 0.7;
            boards = mix(boards, Rgb{0.09, 0.07, 0.05}, clamp01(seam));
            // A coil of rope lying on the boards, with its shadow, and its
            // tail running off the frame.
            const double rx = (x - coilX) / coilRx, ry = (y - coilY) / coilRy;
            const double angle = std::atan2(ry, rx);
            const double rho = std::hypot(rx, ry) + (fbm(angle * 2.0 + 4.0, 1.0, 63, 3) - 0.5) * 0.08;
            const double shadow = std::hypot(rx - 0.1, (y - coilY - 0.025) / coilRy);
            boards = boards * (0.5 + 0.5 * smoothstep(0.8, 1.2, shadow));
            const double tailDistance = std::abs(y - tail(x)) / 0.022;
            if (x > coilX + coilRx * 0.6 && tailDistance < 1.0) {
                boards = boards * 0.6;
                boards = mix(boards, rope((1.0 - tailDistance) * 0.5 + 0.5 - (y < tail(x) ? 0.0 : 0.5), x * 260.0, x, y),
                             smoothstep(1.0, 0.85, tailDistance));
            }
            if (rho < 1.0 && rho > 0.24) {
                const double ring = (rho - 0.24) / 0.76 * 4.5;
                boards = mix(boards, rope(ring - std::floor(ring), angle * 22.0 + ring * 3.0, x, y) *
                                         (1.05 - 0.25 * ry - 0.1 * rx),
                             smoothstep(1.0, 0.97, rho));
            } else if (rho <= 0.24) {
                boards = boards * 0.4;
            }
            c = mix(c, boards, edge);
        }
        // Low sun from the left.
        return c * (1.08 - 0.28 * x / std::max(aspect, 0.01));
    });
    finish(image, 67);
    return image;
}

QImage dronePass(const QSize &size) {
    QImage image(size, QImage::Format_RGB32);
    const double aspect = double(size.width()) / size.height();
    const Rgb fields[] = {{0.36, 0.42, 0.26}, {0.46, 0.47, 0.30}, {0.56, 0.51, 0.38}, {0.30, 0.36, 0.24},
                          {0.50, 0.45, 0.32}, {0.42, 0.40, 0.30}};
    fill(image, [&](double x, double y) {
        const double coast = 0.42 * aspect + 0.1 * std::sin(y * 3.1 + 0.6) + 0.08 * (fbm(1.3, y * 3.0, 71, 4) - 0.5);
        const double d = x - coast;
        if (d < 0.0) {
            // The sea, shallow and green by the shore, deep and blue beyond,
            // with wind ripples.
            const double depth = clamp01(-d / 0.35);
            Rgb c = mix(Rgb{0.34, 0.55, 0.54}, Rgb{0.06, 0.17, 0.26}, smoothstep(0.0, 1.0, std::pow(depth, 0.7)));
            const double ripple = fbm(x * 26.0 + y * 8.0, y * 30.0 - x * 6.0, 73, 4);
            c = c + Rgb{1, 1, 1} * ((ripple - 0.5) * 0.08);
            // Surf: broken foam just off the beach.
            const double surf = (1.0 - smoothstep(0.0, 0.045, -d)) * smoothstep(0.42, 0.68, fbm(x * 55.0, y * 55.0, 79, 4));
            return mix(c, Rgb{0.86, 0.88, 0.86}, surf);
        }
        const double sand = 0.05 + 0.025 * fbm(x * 8.0, y * 8.0, 83, 3);
        if (d < sand) return Rgb{0.78, 0.71, 0.58} * (0.92 + 0.12 * fbm(x * 80.0, y * 80.0, 89, 3));
        // Inland: a patchwork of fields of uneven shapes (a grid bent by
        // noise), some ploughed in rows, hedgerows and copses between them,
        // and a road following the shore.
        const double wx = x + (fbm(x * 2.0, y * 2.0, 91, 3) - 0.5) * 0.25;
        const double wy = y + (fbm(x * 2.0 + 5.0, y * 2.0, 93, 3) - 0.5) * 0.25;
        const double u = (wx * 0.95 - wy * 0.31) * 5.0, v = (wx * 0.31 + wy * 0.95) * 4.0;
        const int cu = int(std::floor(u)), cv = int(std::floor(v));
        Rgb c = fields[int(hash(cu, cv, 97) * 6) % 6];
        c = c * (0.85 + 0.25 * fbm(x * 30.0, y * 30.0, 101, 4));
        if (hash(cu, cv, 107) < 0.5) {
            const double turn = hash(cu, cv, 109) * 3.14159265358979;
            c = c * (0.92 + 0.08 * std::sin((u * std::cos(turn) + v * std::sin(turn)) * 60.0));
        }
        const double hedge = std::min({u - cu, cu + 1 - u, v - cv, cv + 1 - v});
        c = mix(Rgb{0.17, 0.22, 0.13} * (0.8 + 0.4 * fbm(x * 70.0, y * 70.0, 111, 3)), c, smoothstep(0.0, 0.035, hedge));
        const double copse = fbm(x * 9.0, y * 9.0, 113, 4);
        c = mix(c, Rgb{0.14, 0.21, 0.12} * (0.7 + 0.6 * fbm(x * 120.0, y * 120.0, 117, 3)), smoothstep(0.64, 0.68, copse));
        const double road = std::abs(d - sand - 0.12 - 0.02 * std::sin(y * 9.0));
        return mix(c, Rgb{0.42, 0.41, 0.39}, 1.0 - smoothstep(0.004, 0.008, road));
    });
    finish(image, 103);
    return image;
}

} // namespace

QImage footageFrame(Shot shot, const QSize &size) {
    if (size.isEmpty()) return {};
    switch (shot) {
    case Shot::FisherClose: return fisherClose(size);
    case Shot::DronePass: return dronePass(size);
    case Shot::HarborWide: break;
    }
    return harborWide(size);
}

} // namespace kerf::testing
