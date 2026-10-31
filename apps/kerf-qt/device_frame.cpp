#include "device_frame.h"
#include "editor_theme.h"

#include <QFont>
#include <QFontMetricsF>
#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace kerf {

QString devicePreviewKey(DevicePreview preview) {
    switch (preview) {
    case DevicePreview::PunchHole: return QStringLiteral("punchhole");
    case DevicePreview::Off: return QStringLiteral("off");
    case DevicePreview::Island: break;
    }
    return QStringLiteral("island");
}

DevicePreview devicePreviewFromKey(const QString &key) {
    if (key == QLatin1String("punchhole")) return DevicePreview::PunchHole;
    if (key == QLatin1String("off")) return DevicePreview::Off;
    return DevicePreview::Island;
}

QString devicePreviewLabel(DevicePreview preview) {
    switch (preview) {
    case DevicePreview::PunchHole: return QStringLiteral("Phone · Punch-Hole");
    case DevicePreview::Off: return QStringLiteral("Off");
    case DevicePreview::Island: break;
    }
    return QStringLiteral("Phone · Island");
}

double deviceBodyAspect() {
    // The screen is the body's width less two bezels; the body's height is
    // the screen's plus two bezels.
    const double screen = 1.0 - 2.0 * kDeviceBezel;
    return 1.0 / (screen / kDeviceScreenAspect + 2.0 * kDeviceBezel);
}

DeviceGeometry deviceGeometry(const QRectF &body, double pictureAspect) {
    DeviceGeometry out;
    out.body = body;
    out.bezel = kDeviceBezel * body.width();
    out.bodyRadius = kDeviceBodyRadius * body.width();
    out.screenRadius = std::max(0.0, out.bodyRadius - out.bezel);
    out.screen = body.adjusted(out.bezel, out.bezel, -out.bezel, -out.bezel);
    // Across the screen's width, centred on its height; a picture taller
    // than the screen fits its height instead.
    QSizeF picture(out.screen.width(), out.screen.width() / std::max(0.01, pictureAspect));
    if (picture.height() > out.screen.height())
        picture = QSizeF(out.screen.height() * pictureAspect, out.screen.height());
    out.picture = QRectF(out.screen.center() - QPointF(picture.width() / 2.0, picture.height() / 2.0), picture);
    return out;
}

QRectF deviceStatusBar(const DeviceGeometry &device) {
    const double s = device.screen.width();
    return QRectF(device.screen.left(), device.screen.top(), s, 0.144 * s);
}

QRectF deviceCamera(const DeviceGeometry &device, DevicePreview preview) {
    const double s = device.screen.width();
    const double cx = device.screen.center().x();
    if (preview == DevicePreview::Island) {
        const QSizeF size(0.30 * s, 0.088 * s);
        return QRectF(cx - size.width() / 2.0, device.screen.top() + 0.028 * s, size.width(), size.height());
    }
    if (preview == DevicePreview::PunchHole) {
        const double d = std::max(4.0, 0.05 * s);
        const double y = deviceStatusBar(device).center().y() - d / 2.0;
        return QRectF(cx - d / 2.0, y, d, d);
    }
    return {};
}

void paintDeviceBody(QPainter &p, const DeviceGeometry &device, const EditorTheme &theme) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    // A soft shadow round the phone: layers that grow and fade, centred on
    // it, as a 10-point blur would leave; it stays inside the stage's margin.
    constexpr int kLayers = 6;
    for (int layer = 0; layer < kLayers; ++layer) {
        const double spread = kDeviceShadowBlur * (1.0 - double(layer) / kLayers) / 2.0;
        QColor shade = theme.deviceShadow;
        shade.setAlphaF(theme.deviceShadow.alphaF() / kLayers * 1.6);
        p.setBrush(shade);
        p.drawRoundedRect(device.body.adjusted(-spread, -spread, spread, spread), device.bodyRadius + spread, device.bodyRadius + spread);
    }
    // The side buttons, a little proud of the body: volume on the left,
    // power on the right.
    const QRectF body = device.body;
    const double h = body.height();
    const auto button = [&](bool left, double from, double to) {
        const double x = left ? body.left() - kDeviceButtonProud : body.right() - 3.0;
        p.setBrush(theme.deviceEdge);
        p.drawRoundedRect(QRectF(x, body.top() + from * h, kDeviceButtonProud + 3.0, (to - from) * h), 1.0, 1.0);
    };
    button(true, 0.20, 0.26);
    button(true, 0.29, 0.35);
    button(false, 0.24, 0.34);
    // The body, its 1px lighter edge, and the screen.
    p.setBrush(theme.deviceFrame);
    p.drawRoundedRect(body, device.bodyRadius, device.bodyRadius);
    p.setPen(QPen(theme.deviceEdge, 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawRoundedRect(body.adjusted(0.5, 0.5, -0.5, -0.5), device.bodyRadius - 0.5, device.bodyRadius - 0.5);
    p.setPen(Qt::NoPen);
    p.setBrush(theme.deviceScreen);
    p.drawRoundedRect(device.screen, device.screenRadius, device.screenRadius);
    p.restore();
}

void paintDeviceChrome(QPainter &p, const DeviceGeometry &device, DevicePreview preview, const EditorTheme &theme) {
    if (preview == DevicePreview::Off) return;
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF bar = deviceStatusBar(device);
    const QRectF camera = deviceCamera(device, preview);
    // The camera on the screen's black: its faint rim and a lens's glint,
    // so it reads as a simulator draws it.
    p.setPen(QPen(theme.deviceEdge, 1.0));
    p.setBrush(theme.deviceScreen);
    const QRectF rim = camera.adjusted(0.5, 0.5, -0.5, -0.5);
    p.drawRoundedRect(rim, rim.height() / 2.0, rim.height() / 2.0);
    p.setPen(Qt::NoPen);
    p.setBrush(theme.deviceEdge);
    const QPointF lens = preview == DevicePreview::Island ? QPointF(camera.right() - camera.height() / 2.0, camera.center().y()) : camera.center();
    p.drawEllipse(lens, camera.height() * 0.18, camera.height() * 0.18);
    // The time in the left side's middle; signal and battery in the right's.
    const double side = (bar.width() - std::max(camera.width(), 0.30 * bar.width())) / 2.0;
    const QRectF left(bar.left(), bar.top(), side, bar.height());
    const QRectF right(bar.right() - side, bar.top(), side, bar.height());
    QFont font = p.font();
    font.setPixelSize(11);
    font.setWeight(QFont::DemiBold);
    p.setFont(font);
    p.setPen(theme.deviceInk);
    p.drawText(left, Qt::AlignCenter, QStringLiteral("12:00"));
    const QFontMetricsF metrics(font);
    const double glyphHeight = std::round(metrics.capHeight());
    const double y = std::round(right.center().y() + glyphHeight / 2.0);
    // Four bars of signal, rising, then the battery and its nub.
    const double signalWidth = 4 * 2.0 + 3 * 1.5;
    const double batteryWidth = 18.0;
    double x = std::round(right.center().x() - (signalWidth + 5.0 + batteryWidth) / 2.0);
    p.setPen(Qt::NoPen);
    p.setBrush(theme.deviceInk);
    for (int step = 0; step < 4; ++step) {
        const double barHeight = glyphHeight * (0.4 + 0.2 * step);
        p.drawRoundedRect(QRectF(x, y - barHeight, 2.0, barHeight), 0.6, 0.6);
        x += 3.5;
    }
    x += 5.0 - 1.5;
    const QRectF battery(x, y - glyphHeight, batteryWidth - 2.0, glyphHeight);
    p.setPen(QPen(theme.deviceInk, 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawRoundedRect(battery.adjusted(0.5, 0.5, -0.5, -0.5), 2.5, 2.5);
    p.setPen(Qt::NoPen);
    p.setBrush(theme.deviceInk);
    p.drawRoundedRect(battery.adjusted(2.0, 2.0, -5.0, -2.0), 1.0, 1.0);
    p.drawRoundedRect(QRectF(battery.right() + 0.5, battery.center().y() - 2.0, 1.5, 4.0), 0.5, 0.5);
    p.restore();
}

QString laptopPreviewKey(LaptopPreview preview) {
    return preview == LaptopPreview::Off ? QStringLiteral("off") : QStringLiteral("laptop");
}

LaptopPreview laptopPreviewFromKey(const QString &key) {
    return key == QLatin1String("off") ? LaptopPreview::Off : LaptopPreview::Laptop;
}

QString laptopPreviewLabel(LaptopPreview preview) {
    return preview == LaptopPreview::Off ? QStringLiteral("Off") : QStringLiteral("Laptop");
}

namespace {

// The lid's height over its width: the bezel over the screen, the screen,
// and the bezel under it.
double lidAspect() { return kLaptopTopBezel + (1.0 - 2.0 * kLaptopBezel) / kLaptopScreenAspect + kLaptopBezel; }

} // namespace

double laptopBodyAspect() { return kLaptopBaseWidth / (lidAspect() * (1.0 + kLaptopBaseHeight)); }

double laptopPictureShare() { return (1.0 - 2.0 * kLaptopBezel) / kLaptopBaseWidth; }

LaptopGeometry laptopGeometry(const QRectF &body, double pictureAspect) {
    LaptopGeometry out;
    out.body = body;
    const double lidWidth = body.width() / kLaptopBaseWidth;
    const double lidHeight = lidWidth * lidAspect();
    out.lid = QRectF(body.center().x() - lidWidth / 2.0, body.top(), lidWidth, lidHeight);
    out.base = QRectF(body.left(), out.lid.bottom(), body.width(), body.bottom() - out.lid.bottom());
    out.bezel = kLaptopBezel * lidWidth;
    out.topBezel = kLaptopTopBezel * lidWidth;
    out.lidRadius = kLaptopLidRadius * lidWidth;
    out.screen = QRectF(out.lid.left() + out.bezel, out.lid.top() + out.topBezel, lidWidth - 2.0 * out.bezel,
                        (lidWidth - 2.0 * out.bezel) / kLaptopScreenAspect);
    out.screenRadius = std::max(1.5, 0.006 * lidWidth);
    // Across the screen's width, centred on its height; a picture taller
    // than the screen fits its height instead.
    QSizeF picture(out.screen.width(), out.screen.width() / std::max(0.01, pictureAspect));
    if (picture.height() > out.screen.height())
        picture = QSizeF(out.screen.height() * pictureAspect, out.screen.height());
    out.picture = QRectF(out.screen.center() - QPointF(picture.width() / 2.0, picture.height() / 2.0), picture);
    return out;
}

QRectF laptopCamera(const LaptopGeometry &laptop) {
    const double d = std::max(3.0, 0.3 * laptop.topBezel);
    const QPointF centre(laptop.lid.center().x(), laptop.lid.top() + laptop.topBezel / 2.0);
    return QRectF(centre - QPointF(d / 2.0, d / 2.0), QSizeF(d, d));
}

namespace {

// A rectangle whose top corners are rounded by `top` and bottom ones by `bottom`.
QPainterPath roundedTop(const QRectF &r, double top, double bottom) {
    top = std::min({top, r.width() / 2.0, r.height() / 2.0});
    bottom = std::min({bottom, r.width() / 2.0, r.height() / 2.0});
    QPainterPath path;
    path.moveTo(r.left() + top, r.top());
    path.lineTo(r.right() - top, r.top());
    path.quadTo(r.right(), r.top(), r.right(), r.top() + top);
    path.lineTo(r.right(), r.bottom() - bottom);
    path.quadTo(r.right(), r.bottom(), r.right() - bottom, r.bottom());
    path.lineTo(r.left() + bottom, r.bottom());
    path.quadTo(r.left(), r.bottom(), r.left(), r.bottom() - bottom);
    path.lineTo(r.left(), r.top() + top);
    path.quadTo(r.left(), r.top(), r.left() + top, r.top());
    path.closeSubpath();
    return path;
}

} // namespace

void paintLaptopBody(QPainter &p, const LaptopGeometry &laptop, const EditorTheme &theme) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    const QRectF lid = laptop.lid;
    const QRectF base = laptop.base;
    // The base's front corners are rounded by most of its height, as a thin
    // slab's are seen from the front; its back ones hardly at all.
    const double baseRadius = 0.8 * base.height();
    const QPainterPath lidPath = roundedTop(lid, laptop.lidRadius, 1.5);
    const QPainterPath basePath = roundedTop(base, 1.0, baseRadius);
    // The same soft shadow as the phone's, even all round.
    constexpr int kLayers = 6;
    for (int layer = 0; layer < kLayers; ++layer) {
        const double spread = kDeviceShadowBlur * (1.0 - double(layer) / kLayers) / 2.0;
        QColor shade = theme.deviceShadow;
        shade.setAlphaF(theme.deviceShadow.alphaF() / kLayers * 1.6);
        p.setBrush(shade);
        QPainterPath shadow = roundedTop(lid.adjusted(-spread, -spread, spread, spread), laptop.lidRadius + spread, 1.5 + spread);
        shadow.addPath(roundedTop(base.adjusted(-spread, -spread, spread, spread), 1.0 + spread, baseRadius + spread));
        shadow.setFillRule(Qt::WindingFill);
        p.drawPath(shadow);
    }
    // The lid: graphite, its 1px lighter edge, the black screen, and the
    // camera dot in the bezel over it.
    p.setBrush(theme.deviceFrame);
    p.drawPath(lidPath);
    p.setPen(QPen(theme.deviceEdge, 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawPath(roundedTop(lid.adjusted(0.5, 0.5, -0.5, -0.5), laptop.lidRadius - 0.5, 1.0));
    p.setPen(Qt::NoPen);
    p.setBrush(theme.deviceScreen);
    p.drawPath(roundedTop(laptop.screen, laptop.screenRadius, 0.0));
    const QRectF camera = laptopCamera(laptop);
    p.setBrush(theme.deviceScreen);
    p.drawEllipse(camera);
    p.setBrush(theme.deviceEdge);
    p.drawEllipse(camera.center(), camera.width() * 0.18, camera.width() * 0.18);
    // The base: graphite, a lighter top face, its edge, and the thumb-cut in
    // the middle of its front, a shallow scoop in the top face.
    p.setBrush(theme.deviceFrame);
    p.drawPath(basePath);
    const double face = std::max(1.0, std::round(0.38 * base.height()));
    p.setBrush(theme.deviceDeck);
    p.drawRect(QRectF(base.left() + 1.0, base.top(), base.width() - 2.0, face));
    p.setPen(QPen(theme.deviceEdge, 1.0));
    p.setBrush(Qt::NoBrush);
    p.drawPath(roundedTop(base.adjusted(0.5, 0.5, -0.5, -0.5), 0.5, baseRadius - 0.5));
    p.setPen(Qt::NoPen);
    const double cutWidth = 0.13 * lid.width();
    const double cutDepth = std::max(1.5, 0.5 * base.height());
    p.setBrush(theme.deviceHinge);
    p.drawPath(roundedTop(QRectF(base.center().x() - cutWidth / 2.0, base.top(), cutWidth, cutDepth), 0.0, cutDepth));
    // The hinge: a dark line where the lid meets the base, across the lid.
    const double hinge = std::max(1.5, 0.004 * lid.width());
    p.drawRect(QRectF(lid.left() + laptop.lidRadius, lid.bottom() - hinge / 2.0, lid.width() - 2.0 * laptop.lidRadius, hinge));
    p.restore();
}

void DeviceFrameCache::paint(QPainter &painter, Kind kind, const QRectF &body, const EditorTheme &theme, qreal ratio) {
    // The pixmap starts on a whole point; the body keeps its offset within it.
    const QPointF origin(std::floor(body.left()) - kMargin, std::floor(body.top()) - kMargin);
    const QRectF local = body.translated(-origin);
    const bool same = !m_pixmap.isNull() && kind == m_kind && local == m_local && theme.dark == m_dark &&
                      theme.deviceFrame.rgba() == m_frame && theme.deviceScreen.rgba() == m_screen && ratio == m_ratio;
    if (!same) {
        const QSize size(int(std::ceil(local.right())) + kMargin, int(std::ceil(local.bottom())) + kMargin);
        m_pixmap = QPixmap(QSize(int(std::ceil(size.width() * ratio)), int(std::ceil(size.height() * ratio))));
        m_pixmap.setDevicePixelRatio(ratio);
        m_pixmap.fill(Qt::transparent);
        QPainter frame(&m_pixmap);
        if (kind == Kind::Laptop)
            paintLaptopBody(frame, laptopGeometry(local, 16.0 / 9.0), theme);
        else
            paintDeviceBody(frame, deviceGeometry(local, 9.0 / 16.0), theme);
        frame.end();
        m_kind = kind;
        m_local = local;
        m_dark = theme.dark;
        m_frame = theme.deviceFrame.rgba();
        m_screen = theme.deviceScreen.rgba();
        m_ratio = ratio;
        ++m_renders;
    }
    painter.drawPixmap(origin, m_pixmap);
}

SafeZones safeZones(const QRectF &picture) {
    const double w = picture.width();
    const double h = picture.height();
    const QPointF o = picture.topLeft();
    SafeZones out;
    out.top = QRectF(o, QSizeF(w, 0.08 * h));
    out.actions = QRectF(o + QPointF(0.84 * w, 0.42 * h), QSizeF(0.13 * w, 0.38 * h));
    out.caption = QRectF(o + QPointF(0.03 * w, 0.82 * h), QSizeF(0.78 * w, 0.14 * h));
    return out;
}

void paintSafeZones(QPainter &p, const QRectF &picture, const EditorTheme &theme) {
    const SafeZones zones = safeZones(picture);
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    QFont font = p.font();
    font.setPixelSize(11);
    font.setWeight(QFont::DemiBold);
    p.setFont(font);
    const auto zone = [&](const QRectF &area, const QString &label, bool vertical, Qt::Alignment alignment = {}) {
        p.setPen(Qt::NoPen);
        p.setBrush(theme.safeZoneFill);
        p.drawRoundedRect(area, 4, 4);
        QPen line(theme.safeZoneLine, 1.0, Qt::DashLine);
        p.setPen(line);
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(area.adjusted(0.5, 0.5, -0.5, -0.5), 4, 4);
        // The label on a small dark chip, so it reads over the picture and
        // the player's bar alike.
        const QFontMetricsF metrics(font);
        const QSizeF chip(metrics.horizontalAdvance(label) + 8.0, metrics.height() + 2.0);
        p.save();
        QRectF box;
        if (vertical) {
            // Up the column, as a narrow one has no room across.
            p.translate(area.center());
            p.rotate(-90);
            box = QRectF(QPointF(-chip.width() / 2.0, -chip.height() / 2.0), chip);
        } else {
            const QRectF inner = area.adjusted(4, 4, -4, -4);
            const double x = (alignment & Qt::AlignRight) ? inner.right() - chip.width() : inner.left();
            const double y = (alignment & Qt::AlignTop) ? inner.top() : inner.center().y() - chip.height() / 2.0;
            box = QRectF(QPointF(x, y), chip);
        }
        p.setPen(Qt::NoPen);
        p.setBrush(theme.safeZoneLabel);
        p.drawRoundedRect(box, 4, 4);
        p.setPen(theme.safeZoneText);
        p.drawText(box, Qt::AlignCenter, label);
        p.restore();
    };
    // Each label clear of the player's controls: the top bar's at its right,
    // clear of the play cluster; the caption's at its top, over the time.
    zone(zones.top, QStringLiteral("Top bar"), false, Qt::AlignRight | Qt::AlignVCenter);
    zone(zones.actions, QStringLiteral("Actions"), true);
    zone(zones.caption, QStringLiteral("Caption, handle, music"), false, Qt::AlignLeft | Qt::AlignTop);
    p.restore();
}

} // namespace kerf
