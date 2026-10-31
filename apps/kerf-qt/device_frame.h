#pragma once

#include <QRectF>
#include <QString>

class QPainter;

namespace kerf {

struct EditorTheme;

// A portrait picture shown in a phone, as a simulator shows an app: a
// generic phone drawn flat in code (no real product's shape, logo or
// size), its screen taller than the picture (about 9:19.5) with the
// picture across its width and black above and below, and a minimal status
// bar in the black at the top. "Off" is the plain card. A per-user
// preference (QSettings viewer/devicePreview), never the project's.
enum class DevicePreview { Island, PunchHole, Off };

QString devicePreviewKey(DevicePreview preview);
// The preview a settings value names; Island for anything else.
DevicePreview devicePreviewFromKey(const QString &key);
QString devicePreviewLabel(DevicePreview preview);

struct DeviceGeometry {
    QRectF body;   // The phone, its frame included.
    QRectF screen; // Inside the bezel.
    QRectF picture; // The picture, across the screen's width, centred on it.
    qreal bodyRadius = 0;
    qreal screenRadius = 0;
    qreal bezel = 0;
};

// The phone's proportions: body radius 13% and bezel 3% of the body's
// width, screen radius the body's less the bezel, the screen 9:19.5.
constexpr double kDeviceBodyRadius = 0.13;
constexpr double kDeviceBezel = 0.03;
constexpr double kDeviceScreenAspect = 9.0 / 19.5;
// The side buttons stand this far proud of the body.
constexpr double kDeviceButtonProud = 2.0;
// The body's width over its height.
double deviceBodyAspect();
// The phone that fills `body` (of deviceBodyAspect()), showing a picture of
// `pictureAspect` (width over height).
DeviceGeometry deviceGeometry(const QRectF &body, double pictureAspect);
// The status bar's band along the top of the screen.
QRectF deviceStatusBar(const DeviceGeometry &device);
// The camera: an island (a pill) or a punch-hole (a dot) at the top centre.
QRectF deviceCamera(const DeviceGeometry &device, DevicePreview preview);

// The shadow, the body with its edge and side buttons, and the black screen.
void paintDeviceBody(QPainter &painter, const DeviceGeometry &device, const EditorTheme &theme);
// Over the picture: the camera and the status bar ("12:00", signal, battery).
void paintDeviceChrome(QPainter &painter, const DeviceGeometry &device, DevicePreview preview, const EditorTheme &theme);

// Where a vertical-video app's interface usually covers the picture: a band
// along the top, a column of action buttons on the right, and a band for the
// caption, handle and music along the bottom. Generic shapes, no platform's.
struct SafeZones {
    QRectF top;
    QRectF actions;
    QRectF caption;
};
SafeZones safeZones(const QRectF &picture);
void paintSafeZones(QPainter &painter, const QRectF &picture, const EditorTheme &theme);

} // namespace kerf
