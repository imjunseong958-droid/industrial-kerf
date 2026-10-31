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
