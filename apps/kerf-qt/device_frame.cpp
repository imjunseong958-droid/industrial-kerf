#include "device_frame.h"
#include "design_tokens.h"
#include "editor_theme.h"

#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

QColor rgb(QRgb value) { return QColor::fromRgba(value); }

// `color` with its HSL lightness moved by `delta` (0.04 is 4% lighter).
QColor lightened(QRgb color, double delta) {
    float h = 0, s = 0, l = 0, a = 0;
    rgb(color).getHslF(&h, &s, &l, &a);
    return QColor::fromHslF(h, s, float(std::clamp(double(l) + delta, 0.0, 1.0)), a);
}

// The colour `t` (0..1) of the way from `from` to `to`.
QColor mixed(QRgb from, QRgb to, double t) {
    t = std::clamp(t, 0.0, 1.0);
    const auto channel = [t](int a, int b) { return int(std::lround(a + (b - a) * t)); };
    return QColor(channel(qRed(from), qRed(to)), channel(qGreen(from), qGreen(to)), channel(qBlue(from), qBlue(to)));
}

QLinearGradient vertical(const QRectF &r, QRgb top, QRgb bottom) {
    QLinearGradient gradient(r.topLeft(), r.bottomLeft());
    gradient.setColorAt(0.0, rgb(top));
    gradient.setColorAt(1.0, rgb(bottom));
    return gradient;
}

// A rectangle whose top corners are circular arcs of `top` and bottom ones of `bottom`.
QPainterPath roundedTop(const QRectF &r, double top, double bottom) {
    const double most = std::max(0.0, std::min(r.width(), r.height()) / 2.0);
    top = std::clamp(top, 0.0, most);
    bottom = std::clamp(bottom, 0.0, most);
    QPainterPath path;
    path.moveTo(r.left() + top, r.top());
    path.lineTo(r.right() - top, r.top());
    if (top > 0) path.arcTo(QRectF(r.right() - 2.0 * top, r.top(), 2.0 * top, 2.0 * top), 90, -90);
    path.lineTo(r.right(), r.bottom() - bottom);
    if (bottom > 0) path.arcTo(QRectF(r.right() - 2.0 * bottom, r.bottom() - 2.0 * bottom, 2.0 * bottom, 2.0 * bottom), 0, -90);
    path.lineTo(r.left() + bottom, r.bottom());
    if (bottom > 0) path.arcTo(QRectF(r.left(), r.bottom() - 2.0 * bottom, 2.0 * bottom, 2.0 * bottom), 270, -90);
    path.lineTo(r.left(), r.top() + top);
    if (top > 0) path.arcTo(QRectF(r.left(), r.top(), 2.0 * top, 2.0 * top), 180, -90);
    path.closeSubpath();
    return path;
}

// A continuous corner of `radius`: it leaves each edge 1.5287 radii from the
// corner with no curvature (both control points on the edge), and its two
// halves meet at a circular corner's 45-degree point, tangent there.
constexpr double kCornerReach = 1.528665;
constexpr double kCornerControl = 1.088493;
constexpr double kCornerInner = 2.0 - 1.4142135623730951; // 0.58579: the 45-degree point's tangent meets the edge here
constexpr double kCornerMid = 1.0 - 0.7071067811865476;   // 0.29289: the 45-degree point's offset from each edge

QPainterPath continuousRect(const QRectF &r, double radius) {
    radius = std::clamp(radius, 0.0, std::min(r.width(), r.height()) / 2.0 / kCornerReach);
    const double reach = kCornerReach * radius;
    QPainterPath path;
    // `u` points back along the edge that arrives at `corner`, `v` along the one that leaves it.
    const auto corner = [&](const QPointF &c, const QPointF &u, const QPointF &v) {
        const auto at = [&](double a, double b) { return c + a * radius * u + b * radius * v; };
        path.cubicTo(at(kCornerControl, 0), at(kCornerInner, 0), at(kCornerMid, kCornerMid));
        path.cubicTo(at(0, kCornerInner), at(0, kCornerControl), at(0, kCornerReach));
    };
    path.moveTo(r.left() + reach, r.top());
    path.lineTo(r.right() - reach, r.top());
    corner(r.topRight(), QPointF(-1, 0), QPointF(0, 1));
    path.lineTo(r.right(), r.bottom() - reach);
    corner(r.bottomRight(), QPointF(0, -1), QPointF(-1, 0));
    path.lineTo(r.left() + reach, r.bottom());
    corner(r.bottomLeft(), QPointF(1, 0), QPointF(0, -1));
    path.lineTo(r.left(), r.top() + reach);
    corner(r.topLeft(), QPointF(0, 1), QPointF(1, 0));
    path.closeSubpath();
    return path;
}

// The picture of `aspect` across `screen`'s width, centred on its height; a
// picture taller than the screen fits its height instead.
QRectF pictureOn(const QRectF &screen, double aspect) {
    aspect = std::max(0.01, aspect);
    QSizeF size(screen.width(), screen.width() / aspect);
    if (size.height() > screen.height()) size = QSizeF(screen.height() * aspect, screen.height());
    return QRectF(screen.center() - QPointF(size.width() / 2.0, size.height() / 2.0), size);
}

// The screen width the feed's offsets are the mockup's at: the phone's at the
// 1600 x 1000 stage, 480 tall.
constexpr double kFeedReference = 480.0 * kDeviceBodyAspect * kDeviceScreenWidth; // 213.66

void paintPhone(QPainter &p, const DeviceGeometry &device) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    const QRectF body = device.body;
    // The buttons under the band, 4% darker than the band where they sit.
    for (const QRectF &button : deviceButtons(device)) {
        const double t = (button.center().y() - body.top()) / std::max(1.0, body.height());
        const QColor band = mixed(device_material::bandTop, device_material::bandBottom, t);
        p.setBrush(lightened(band.rgb(), -device_material::buttonDarker));
        p.drawRoundedRect(button, kDeviceButtonRadius, kDeviceButtonRadius);
    }
    // The band: a 1 pt black outline, then the graphite.
    p.setBrush(rgb(device_material::outline));
    p.drawPath(deviceBodyPath(device));
    DeviceGeometry inner = device;
    inner.body = body.adjusted(1, 1, -1, -1);
    inner.bodyRadius = std::max(0.0, device.bodyRadius - 1.0);
    p.setBrush(vertical(body, device_material::bandTop, device_material::bandBottom));
    p.drawPath(deviceBodyPath(inner));
    // The glass, and the screen's black on it.
    p.setBrush(rgb(tokens::overlay::deviceGlass));
    p.drawRoundedRect(device.glass, device.glassRadius, device.glassRadius);
    p.setBrush(rgb(tokens::overlay::deviceScreen));
    p.drawRoundedRect(device.screen, device.screenRadius, device.screenRadius);
    p.restore();
}

void paintLaptop(QPainter &p, const LaptopGeometry &laptop) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    const QRectF lid = laptop.lid;
    const QRectF base = laptop.base;
    // The lid: a 1 pt black outline, a 1 pt lighter line inside it, and the
    // graphite, all inside the lid's own bounds.
    const auto lidPath = [&](double inset) {
        return roundedTop(lid.adjusted(inset, inset, -inset, -inset), laptop.lidRadius - inset, laptop.lidFootRadius - inset);
    };
    p.setBrush(rgb(device_material::outline));
    p.drawPath(lidPath(0));
    p.setBrush(rgb(device_material::lidInset));
    p.drawPath(lidPath(1));
    p.setBrush(vertical(lid, device_material::lidTop, device_material::lidBottom));
    p.drawPath(lidPath(2));
    // The glass inside the rim, the glass-edge line just outside the screen,
    // and the screen's black; the screen's top corners round, its bottom
    // ones square.
    p.setBrush(rgb(tokens::overlay::deviceGlass));
    p.drawPath(roundedTop(laptop.glass, laptop.glassRadius, std::max(0.0, laptop.lidFootRadius - laptop.rim)));
    const double edge = kLaptopGlassEdge;
    p.setBrush(rgb(device_material::outline));
    p.drawPath(roundedTop(laptop.screen.adjusted(-edge, -edge, edge, edge), laptop.screenRadius + edge, 0.0));
    p.setBrush(rgb(tokens::overlay::deviceScreen));
    p.drawPath(roundedTop(laptop.screen, laptop.screenRadius, 0.0));
    // The hinge: the lid's bottom rim in its dark tone.
    p.setBrush(rgb(device_material::hinge));
    p.drawRect(laptop.hinge);
    // The base: outlined, its face darker toward the foot, under a top lip
    // 4% lighter, the thumb scoop cut from the lip, 3% darker than the face.
    if (!base.isEmpty()) {
        const auto basePath = [&](double inset) {
            return roundedTop(base.adjusted(inset, inset, -inset, -inset), laptop.baseTopRadius - inset, laptop.baseRadius - inset);
        };
        p.setBrush(rgb(device_material::outline));
        p.drawPath(basePath(0));
        p.setBrush(vertical(base, device_material::baseTop, device_material::baseBottom));
        p.drawPath(basePath(1));
        const double lip = std::min(kLaptopLip, base.height() / 2.0);
        p.setBrush(lightened(device_material::baseTop, device_material::lipLighter));
        p.drawRect(QRectF(base.left() + 1.0, base.top(), base.width() - 2.0, lip));
        const QRectF scoop = laptop.scoop;
        if (!scoop.isEmpty()) {
            // Its ends elliptical, as the mockup's 24 / 4 corners on a 112-wide scoop.
            const double rx = std::min(scoop.width() / 2.0, 24.0 / 112.0 * scoop.width());
            const double ry = scoop.height();
            QPainterPath cut;
            cut.moveTo(scoop.topLeft());
            cut.arcTo(QRectF(scoop.left(), scoop.top() - ry, 2.0 * rx, 2.0 * ry), 180, 90);
            cut.lineTo(scoop.right() - rx, scoop.bottom());
            cut.arcTo(QRectF(scoop.right() - 2.0 * rx, scoop.top() - ry, 2.0 * rx, 2.0 * ry), 270, 90);
            cut.closeSubpath();
            const QColor face = mixed(device_material::baseTop, device_material::baseBottom, 0.5 * scoop.height() / base.height());
            p.setBrush(lightened(face.rgb(), -device_material::scoopDarker));
            p.drawPath(cut);
        }
    }
    p.restore();
}

} // namespace

// ------------------------------------------------------------------------ phone

QString devicePreviewKey(DevicePreview preview) {
    return preview == DevicePreview::Off ? QStringLiteral("off") : QStringLiteral("phone");
}

DevicePreview devicePreviewFromKey(const QString &key) {
    // "phone", and the old "island", "punchhole" and "punch", are the phone.
    return key == QLatin1String("off") ? DevicePreview::Off : DevicePreview::Phone;
}

QString devicePreviewLabel(DevicePreview preview) {
    return preview == DevicePreview::Off ? QStringLiteral("Off") : QStringLiteral("Phone");
}

QString devicePreviewLabel(DevicePreview preview, double pictureAspect) {
    if (preview != DevicePreview::Off && deviceShowsFeed(pictureAspect)) return QStringLiteral("Phone · Feed");
    return devicePreviewLabel(preview);
}

QList<DevicePreview> devicePreviewChoices() { return {DevicePreview::Phone, DevicePreview::Off}; }

bool deviceShowsFeed(double pictureAspect) { return pictureAspect >= 0.75 - 1e-3 && pictureAspect <= 1.0 + 1e-3; }

double deviceBodyAspect() { return kDeviceBodyAspect; }

DeviceGeometry deviceGeometry(const QRectF &body, double pictureAspect) {
    DeviceGeometry out;
    out.body = body;
    const double w = body.width();
    out.bezel = kDeviceBezel * w;
    out.topBezel = kDeviceTopBezel * w;
    out.bodyRadius = kDeviceBodyRadius * w;
    // The screen at its own shape, centred in the body.
    const QSizeF screen(w - 2.0 * out.bezel, (w - 2.0 * out.bezel) / kDeviceScreenAspect);
    out.screen = QRectF(body.center() - QPointF(screen.width() / 2.0, screen.height() / 2.0), screen);
    out.screenRadius = kDeviceScreenRadius * screen.width();
    const double glass = kDeviceGlass * w;
    out.glass = out.screen.adjusted(-glass, -glass, glass, glass);
    out.glassRadius = out.screenRadius + glass;
    out.picture = pictureOn(out.screen, pictureAspect);
    return out;
}

QPainterPath deviceBodyPath(const DeviceGeometry &device) { return continuousRect(device.body, device.bodyRadius); }

QList<QRectF> deviceButtons(const DeviceGeometry &device) {
    const QRectF body = device.body;
    QList<QRectF> out;
    for (const DeviceButtonSpan &span : kDeviceButtons) {
        const double x = span.left ? body.left() - kDeviceButtonProud : body.right() + kDeviceButtonProud - kDeviceButtonWidth;
        out.append(QRectF(x, body.top() + span.from * body.height(), kDeviceButtonWidth, (span.to - span.from) * body.height()));
    }
    return out;
}

QRectF deviceStatusBar(const DeviceGeometry &) { return {}; }

QRectF deviceCamera(const DeviceGeometry &, DevicePreview) { return {}; }

DeviceFeed deviceFeed(const DeviceGeometry &device) {
    const double s = std::min(1.0, device.screen.width() / kFeedReference);
    const QRectF pic = device.picture;
    const double px = pic.left(), py = pic.top(), below = pic.bottom();
    const auto box = [s](double x, double y, double w, double h) { return QRectF(x, y, w * s, h * s); };
    DeviceFeed out;
    out.avatar = box(px + 12 * s, py - 36 * s, 24, 24);
    out.name = box(px + 44 * s, py - 32 * s, 72, 6);
    out.handle = box(px + 44 * s, py - 20 * s, 44, 6);
    for (int index = 0; index < 3; ++index) out.actions[index] = box(px + (12 + 28 * index) * s, below + 12 * s, 16, 16);
    out.caption = box(px + 12 * s, below + 40 * s, 160, 6);
    out.captionMore = box(px + 12 * s, below + 52 * s, 112, 6);
    return out;
}

void paintDeviceBody(QPainter &painter, const DeviceGeometry &device, const EditorTheme &) { paintPhone(painter, device); }

void paintDeviceFeed(QPainter &p, const DeviceGeometry &device) {
    const DeviceFeed feed = deviceFeed(device);
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(rgb(tokens::raised));
    QPainterPath screen;
    screen.addRoundedRect(device.screen, device.screenRadius, device.screenRadius);
    p.setClipPath(screen, Qt::IntersectClip);
    const auto bar = [&p](const QRectF &r) {
        const double radius = std::min({tokens::radius::small, r.height() / 2.0, r.width() / 2.0});
        p.drawRoundedRect(r, radius, radius);
    };
    p.drawEllipse(feed.avatar);
    bar(feed.name);
    bar(feed.handle);
    for (const QRectF &dot : feed.actions) p.drawEllipse(dot);
    bar(feed.caption);
    bar(feed.captionMore);
    p.restore();
}

void paintDeviceChrome(QPainter &, const DeviceGeometry &, DevicePreview, const EditorTheme &) {}

// ----------------------------------------------------------------------- laptop

QString laptopPreviewKey(LaptopPreview preview) {
    return preview == LaptopPreview::Off ? QStringLiteral("off") : QStringLiteral("laptop");
}

LaptopPreview laptopPreviewFromKey(const QString &key) {
    return key == QLatin1String("off") ? LaptopPreview::Off : LaptopPreview::Laptop;
}

QString laptopPreviewLabel(LaptopPreview preview) {
    return preview == LaptopPreview::Off ? QStringLiteral("Off") : QStringLiteral("Laptop");
}

double laptopBodyAspect() { return kLaptopBodyAspect; }

double laptopPictureShare() { return kLaptopScreenWidth / kLaptopBodyWidth; }

LaptopGeometry laptopGeometry(const QRectF &body, double pictureAspect) {
    LaptopGeometry out;
    out.body = body;
    const double w = body.width() / kLaptopBodyWidth;
    out.lid = QRectF(body.center().x() - w / 2.0, body.top(), w, kLaptopLidHeight * w);
    out.base = QRectF(body.center().x() - kLaptopBaseWidth * w / 2.0, out.lid.bottom(), kLaptopBaseWidth * w,
                      std::max(0.0, body.bottom() - out.lid.bottom()));
    out.rim = kLaptopRim * w;
    out.bezel = kLaptopBezel * w;
    out.topBezel = out.bezel;
    out.glass = out.lid.adjusted(out.rim, out.rim, -out.rim, -out.rim);
    out.screen = out.lid.adjusted(out.bezel, out.bezel, -out.bezel, -out.bezel);
    out.screenRadius = kLaptopScreenRadius * out.screen.width();
    out.lidRadius = out.screenRadius + out.bezel;
    out.glassRadius = out.screenRadius + (out.bezel - out.rim);
    out.lidFootRadius = kLaptopLidFootRadius * w;
    out.hinge = QRectF(out.lid.left() + out.rim, out.lid.bottom() - out.rim, w - 2.0 * out.rim, out.rim);
    out.baseRadius = kLaptopBaseRadius * w;
    out.baseTopRadius = kLaptopBaseTopRadius;
    const double scoopWidth = kLaptopScoopWidth * out.base.width();
    out.scoop = QRectF(out.base.center().x() - scoopWidth / 2.0, out.base.top(), scoopWidth,
                       std::min(kLaptopScoopDepth * w, out.base.height() / 2.0));
    out.picture = pictureOn(out.screen, pictureAspect);
    return out;
}

QRectF laptopCamera(const LaptopGeometry &) { return {}; }

void paintLaptopBody(QPainter &painter, const LaptopGeometry &laptop, const EditorTheme &) { paintLaptop(painter, laptop); }

// ------------------------------------------------------------------------ cache

void DeviceFrameCache::paint(QPainter &painter, Kind kind, const QRectF &body, double pictureAspect, qreal ratio) {
    // The pixmap starts on a whole point; the body keeps its offset within it.
    const QPointF origin(std::floor(body.left()) - kMargin, std::floor(body.top()) - kMargin);
    const QRectF local = body.translated(-origin);
    // Only a phone's feed depends on the picture's shape.
    const bool feed = kind == Kind::Phone && deviceShowsFeed(pictureAspect);
    const double aspect = feed ? pictureAspect : 0.0;
    const bool same = !m_pixmap.isNull() && kind == m_kind && local == m_local && feed == m_feed && aspect == m_aspect &&
                      ratio == m_ratio;
    if (!same) {
        const QSize size(int(std::ceil(local.right())) + kMargin, int(std::ceil(local.bottom())) + kMargin);
        m_pixmap = QPixmap(QSize(int(std::ceil(size.width() * ratio)), int(std::ceil(size.height() * ratio))));
        m_pixmap.setDevicePixelRatio(ratio);
        m_pixmap.fill(Qt::transparent);
        QPainter frame(&m_pixmap);
        if (kind == Kind::Laptop) {
            paintLaptop(frame, laptopGeometry(local, 16.0 / 9.0));
        } else {
            const DeviceGeometry phone = deviceGeometry(local, feed ? aspect : 9.0 / 16.0);
            paintPhone(frame, phone);
            if (feed) paintDeviceFeed(frame, phone);
        }
        frame.end();
        m_kind = kind;
        m_local = local;
        m_feed = feed;
        m_aspect = aspect;
        m_ratio = ratio;
        ++m_renders;
    }
    painter.drawPixmap(origin, m_pixmap);
}

void DeviceFrameCache::paint(QPainter &painter, Kind kind, const QRectF &body, const EditorTheme &, qreal ratio) {
    paint(painter, kind, body, kind == Kind::Laptop ? 16.0 / 9.0 : 9.0 / 16.0, ratio);
}

// ------------------------------------------------------------------ safe zones

namespace {
constexpr QRgb kSafeTickColor = qRgba(255, 255, 255, tokens::alpha(0.45));
} // namespace

SafeZones safeZones(const QRectF &picture) {
    const double w = picture.width();
    const double h = picture.height();
    SafeZones out;
    out.action = picture.adjusted(kActionSafeInset * w, kActionSafeInset * h, -kActionSafeInset * w, -kActionSafeInset * h);
    out.title = picture.adjusted(kTitleSafeInset * w, kTitleSafeInset * h, -kTitleSafeInset * w, -kTitleSafeInset * h);
    out.cross = QRectF(picture.center() - QPointF(kSafeCross / 2.0, kSafeCross / 2.0), QSizeF(kSafeCross, kSafeCross));
    return out;
}

void paintSafeZones(QPainter &p, const QRectF &picture, const EditorTheme &) {
    if (picture.width() < 2.0 * kSafeTick || picture.height() < 2.0 * kSafeTick) return;
    const SafeZones zones = safeZones(picture);
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.setPen(Qt::NoPen);
    // Action safe: an inset 1 pt line, drawn as four bars inside the rect.
    const QRectF a = zones.action;
    p.setBrush(tokens::color(tokens::lineh));
    p.drawRect(QRectF(a.left(), a.top(), a.width(), 1.0));
    p.drawRect(QRectF(a.left(), a.bottom() - 1.0, a.width(), 1.0));
    p.drawRect(QRectF(a.left(), a.top() + 1.0, 1.0, a.height() - 2.0));
    p.drawRect(QRectF(a.right() - 1.0, a.top() + 1.0, 1.0, a.height() - 2.0));
    // Title safe: an L of 16 x 16 in each corner, inside the rect, and the cross.
    const QRectF t = zones.title;
    p.setBrush(tokens::color(kSafeTickColor));
    const double k = kSafeTick;
    p.drawRect(QRectF(t.left(), t.top(), k, 1.0));
    p.drawRect(QRectF(t.left(), t.top() + 1.0, 1.0, k - 1.0));
    p.drawRect(QRectF(t.right() - k, t.top(), k, 1.0));
    p.drawRect(QRectF(t.right() - 1.0, t.top() + 1.0, 1.0, k - 1.0));
    p.drawRect(QRectF(t.left(), t.bottom() - 1.0, k, 1.0));
    p.drawRect(QRectF(t.left(), t.bottom() - k, 1.0, k - 1.0));
    p.drawRect(QRectF(t.right() - k, t.bottom() - 1.0, k, 1.0));
    p.drawRect(QRectF(t.right() - 1.0, t.bottom() - k, 1.0, k - 1.0));
    const QRectF c = zones.cross;
    p.drawRect(QRectF(c.left(), c.top() + kSafeCross / 2.0, kSafeCross, 1.0));
    p.drawRect(QRectF(c.left() + kSafeCross / 2.0, c.top(), 1.0, kSafeCross));
    p.restore();
}

} // namespace kerf
