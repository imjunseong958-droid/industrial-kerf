#pragma once

#include <QPixmap>
#include <QRectF>
#include <QString>

class QPainter;

namespace kerf {

struct EditorTheme;

// A portrait picture shown in a phone, as a simulator shows an app: a
// generic phone drawn flat in code (no real product's shape, logo or
// size), its screen taller than the picture (about 9:18.5) with the
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

// The phone's proportions: body radius 13% and bezel 2.5% of the body's
// width, screen radius the body's less the bezel, the screen 9:18.5 (a
// tall phone's, short enough that the picture is large in a 575 stage).
constexpr double kDeviceBodyRadius = 0.13;
constexpr double kDeviceBezel = 0.025;
// Its shadow on the stage: even all round (no offset), so the stage's edge
// never cuts it.
constexpr double kDeviceShadowBlur = 10.0;
constexpr double kDeviceScreenAspect = 9.0 / 18.5;
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

// A wide picture shown in a laptop, as the phone shows a portrait one: a
// generic laptop of today drawn flat in code (no real product's shape,
// logo or size) in a warm stone metal, not the phone's graphite: a lid of
// black glass in a thin metal rim, its screen 16:10 with the 16:9 picture
// across its width, the thin black bars above and below it, and a small
// camera notch in the bar over the picture; under the lid, a thin base a
// little wider, lit along its top. "Off" is the plain card. A per-user
// preference (QSettings viewer/laptopPreview), never the project's.
enum class LaptopPreview { Laptop, Off };

QString laptopPreviewKey(LaptopPreview preview);
// The preview a settings value names; Laptop for anything else.
LaptopPreview laptopPreviewFromKey(const QString &key);
QString laptopPreviewLabel(LaptopPreview preview);

struct LaptopGeometry {
    QRectF body;    // Lid top to base bottom, as wide as the base.
    QRectF lid;     // The lid, its rim and bezel included.
    QRectF screen;  // Inside the bezel, 16:10.
    QRectF picture; // The picture, across the screen's width, centred on it.
    QRectF notch;   // The camera's notch, hanging from the bezel into the bar over the picture.
    QRectF base;    // The deck under the lid.
    qreal lidRadius = 0;
    qreal screenRadius = 0; // The screen's top corners; its bottom ones are square.
    qreal rim = 0;          // The metal round the glass.
    qreal bezel = 0;        // At the sides and the foot of the screen, the rim included.
    qreal topBezel = 0;     // Over it, a little thicker.
};

// The laptop's proportions, as shares of the lid's width: a thin bezel of
// 1.8% (2.4% over the screen), 0.4% of it (2 points at least) the metal
// rim, the screen 16:10 with its top corners rounded by 1.1%, the lid's top
// corners by 2.8%;
// the notch 7% wide and 1.8% deep; the base 108% of the lid's width and 3%
// of its height. The body the stage lays out is the base's full width, the
// lid centred in it (4% of the lid to spare at each side), so the stage's
// margins and the gap before the rail are measured from the laptop's widest
// part.
constexpr double kLaptopBezel = 0.018;
constexpr double kLaptopTopBezel = 0.024;
constexpr double kLaptopRim = 0.004;
constexpr double kLaptopScreenAspect = 16.0 / 10.0;
constexpr double kLaptopScreenRadius = 0.011;
constexpr double kLaptopLidRadius = 0.028;
constexpr double kLaptopNotchWidth = 0.07;
constexpr double kLaptopNotchDepth = 0.018;
constexpr double kLaptopBaseWidth = 1.08;
constexpr double kLaptopBodyWidth = kLaptopBaseWidth;
constexpr double kLaptopBaseHeight = 0.03;
// The body's width over its height.
double laptopBodyAspect();
// The share of the body's width a picture across the screen takes.
double laptopPictureShare();
// The laptop that fills `body` (of laptopBodyAspect()), showing a picture of
// `pictureAspect` (width over height).
LaptopGeometry laptopGeometry(const QRectF &body, double pictureAspect);
// The camera dot in the middle of the notch.
QRectF laptopCamera(const LaptopGeometry &laptop);
// The shadow, the lid with its rim, glass, black screen, notch and camera,
// the hinge, and the base with its lit top face, its thumb-cut and its feet.
void paintLaptopBody(QPainter &painter, const LaptopGeometry &laptop, const EditorTheme &theme);

// A device's frame (its shadow, body and black screen: everything that does
// not change from one frame of video to the next) drawn once into a pixmap
// for a size, appearance and pixel ratio, then only blitted, so showing the
// picture in a phone or a laptop costs playback one pixmap blit.
class DeviceFrameCache {
public:
    enum class Kind { Phone, Laptop };
    // Room round the body for the shadow and the phone's side buttons.
    static constexpr int kMargin = 24;
    // Draws the frame of `kind` whose body is `body`, rendering it first
    // only when the kind, the body's size (or its offset within a pixel),
    // the appearance or the pixel ratio changed.
    void paint(QPainter &painter, Kind kind, const QRectF &body, const EditorTheme &theme, qreal ratio);
    // How many times a frame was rendered rather than blitted.
    int renders() const { return m_renders; }

private:
    QPixmap m_pixmap;
    Kind m_kind = Kind::Phone;
    QRectF m_local; // The body within the pixmap.
    QRgb m_frame = 0;
    QRgb m_screen = 0;
    bool m_dark = false;
    qreal m_ratio = 0;
    int m_renders = 0;
};

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
