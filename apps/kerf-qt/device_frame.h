#pragma once

#include <QList>
#include <QPainterPath>
#include <QPixmap>
#include <QRectF>
#include <QRgb>
#include <QString>

class QPainter;

namespace kerf {

struct EditorTheme;

// The Program stage shows a wide picture in a laptop and a portrait, square
// or 4:5 one in a phone. Both are drawn flat in code at the exact body and
// screen proportions of two real products, a MacBook Pro 14" (M4) and an
// iPhone 16 Pro, from their published sizes (research/v6-device-ref.md and
// mockup-spec 3.2). Nothing else of the products is copied: no logo, no
// lettering, no notch, Dynamic Island, punch-hole, camera, status bar,
// speaker or keyboard. The bezel is thin and even on all four sides, the top
// as thin as the others.
//
// The screens keep their own shapes, so a 16:9 or 9:16 picture across the
// screen's width leaves black screen above and below it, as a video played
// full screen on that display does. That black is screen (#000), not bezel:
// the bezel is the glass round it (#0b0b0c) and the metal round that.
//
// Every size below is the product's in millimetres, then derived as a share
// of the body's (or the lid's) width, so a frame of any size has the same
// proportions. The radii Apple does not publish are estimates and say so.

// ------------------------------------------------------------------ millimetres

namespace device_mm {

// MacBook Pro 14" (M4): Apple's tech specs. The panel is 3024 x 1964 pixels
// at 254 ppi.
constexpr double laptopWidth = 312.6;
constexpr double laptopScreenWidth = 302.4;
constexpr double laptopScreenHeight = 196.4;
// The border round the screen, the same on every side: black glass, then the
// aluminium rim.
constexpr double laptopGlass = 3.5;
constexpr double laptopRim = 1.6;
constexpr double laptopBorder = laptopGlass + laptopRim; // (312.6 - 302.4) / 2
constexpr double laptopLidHeight = laptopScreenHeight + 2.0 * laptopBorder;
// The base's front face seen under the lid: 15.5 closed less about 4.7 of lid.
constexpr double laptopBase = 10.8;
constexpr double laptopHeight = laptopLidHeight + laptopBase;
// The thumb scoop's depth in the base's front edge.
constexpr double laptopScoopDepth = 2.5;

// iPhone 16 Pro: Apple's tech specs. The panel is 1206 x 2622 pixels at
// 460 ppi; its corners are 62 of its 402 points.
constexpr double phoneWidth = 71.5;
constexpr double phoneHeight = 149.6;
constexpr double phoneScreenWidth = 1206.0 / 460.0 * 25.4;  // 66.592
constexpr double phoneScreenHeight = 2622.0 / 460.0 * 25.4; // 144.780
constexpr double phoneSideBorder = (phoneWidth - phoneScreenWidth) / 2.0;  // 2.454
constexpr double phoneTopBorder = (phoneHeight - phoneScreenHeight) / 2.0; // 2.410
// The black glass inside the titanium band (an estimate, about 1.2).
constexpr double phoneGlass = 1.2;
constexpr double phoneScreenRadius = 62.0 / 402.0 * phoneScreenWidth; // 10.270
// Concentric with the screen's corners.
constexpr double phoneBodyRadius = phoneScreenRadius + phoneSideBorder; // 12.724

} // namespace device_mm

// The frames' materials: the mockup's graphite, flat, with a 1 pt black
// outline and no shadow, glow or reflection. The glass and the screen's
// black are design tokens (tokens::overlay::deviceGlass, deviceScreen).
namespace device_material {
constexpr QRgb lidTop = qRgb(0x4a, 0x4b, 0x50);
constexpr QRgb lidBottom = qRgb(0x3f, 0x40, 0x45);
constexpr QRgb lidInset = qRgb(0x55, 0x56, 0x5b); // a 1 pt line inside the outline
constexpr QRgb baseTop = qRgb(0x50, 0x51, 0x56);
constexpr QRgb baseBottom = qRgb(0x3e, 0x3f, 0x44);
constexpr QRgb hinge = qRgb(0x1e, 0x1f, 0x22); // the lid's bottom rim
constexpr QRgb bandTop = qRgb(0x4b, 0x4c, 0x51);
constexpr QRgb bandBottom = qRgb(0x40, 0x41, 0x46);
constexpr QRgb outline = qRgb(0x00, 0x00, 0x00);
// Tone steps, in HSL lightness: the base's top lip is 4% lighter than its
// face, the thumb scoop 3% darker, the phone's buttons 4% darker than its band.
constexpr double lipLighter = 0.04;
constexpr double scoopDarker = 0.03;
constexpr double buttonDarker = 0.04;
} // namespace device_material

// ------------------------------------------------------------------------ phone

// Phone, or Off (the plain card). A per-user preference (QSettings
// viewer/devicePreview), never the project's. A 1:1 or 4:5 picture in the
// phone shows as a feed post (see deviceShowsFeed).
//
// Island and PunchHole are the old choices, now aliases of Phone:
// deprecated, removed in W3.
enum class DevicePreview { Phone, Off, Island = Phone, PunchHole = Phone };

// "phone" or "off".
QString devicePreviewKey(DevicePreview preview);
// The preview a settings value names: "off" is Off; "phone", the old
// "island", "punchhole" and "punch", and anything else are Phone.
DevicePreview devicePreviewFromKey(const QString &key);
// "Phone" or "Off".
QString devicePreviewLabel(DevicePreview preview);
// The menu's name for `preview` with a picture of `pictureAspect` (width
// over height): "Phone · Feed" for a 1:1 or 4:5 picture in the phone.
QString devicePreviewLabel(DevicePreview preview, double pictureAspect);
// The choices a menu offers, in order: Phone, Off.
QList<DevicePreview> devicePreviewChoices();
// Whether a picture of `pictureAspect` shows in the phone as a feed post:
// square and 4:5 (and anything between), not 9:16.
bool deviceShowsFeed(double pictureAspect);

struct DeviceGeometry {
    QRectF body;    // The phone, its band included.
    QRectF glass;   // The black glass inside the band.
    QRectF screen;  // The display, 1206:2622.
    QRectF picture; // The picture, across the screen's width, centred on it.
    qreal bodyRadius = 0;   // Drawn as a continuous corner (see deviceBodyPath).
    qreal glassRadius = 0;
    qreal screenRadius = 0;
    qreal bezel = 0;    // The border at the sides, band and glass.
    qreal topBezel = 0; // The border at the top and the foot (within 0.2 pt of the sides').
};

// The phone's proportions as shares of its body's width (the screen's
// radius of the screen's width).
constexpr double kDeviceBodyAspect = device_mm::phoneWidth / device_mm::phoneHeight;          // 0.47794
constexpr double kDeviceScreenWidth = device_mm::phoneScreenWidth / device_mm::phoneWidth;   // 0.93136
constexpr double kDeviceScreenAspect = device_mm::phoneScreenWidth / device_mm::phoneScreenHeight; // 0.45995
constexpr double kDeviceBezel = device_mm::phoneSideBorder / device_mm::phoneWidth;          // 0.034322
constexpr double kDeviceTopBezel = device_mm::phoneTopBorder / device_mm::phoneWidth;        // 0.033706
constexpr double kDeviceGlass = device_mm::phoneGlass / device_mm::phoneWidth;
constexpr double kDeviceScreenRadius = device_mm::phoneScreenRadius / device_mm::phoneScreenWidth; // 0.15423
constexpr double kDeviceBodyRadius = device_mm::phoneBodyRadius / device_mm::phoneWidth;     // 0.17796
// The side buttons, in points: 2 wide with 1.3 of it proud of the band
// (0.4 mm at the stage's scale), corners of 1.
constexpr double kDeviceButtonWidth = 2.0;
constexpr double kDeviceButtonProud = 1.3;
constexpr double kDeviceButtonRadius = 1.0;
// Each button's span down the body, as shares of its height from the top
// (estimates from Apple's drawings).
struct DeviceButtonSpan {
    bool left;
    double from;
    double to;
};
constexpr DeviceButtonSpan kDeviceButtons[] = {
    {true, 0.155, 0.195},  // Action
    {true, 0.245, 0.305},  // volume up
    {true, 0.33, 0.39},    // volume down
    {false, 0.26, 0.37},   // side button
    {false, 0.63, 0.72},   // Camera Control
};

// The body's width over its height.
double deviceBodyAspect();
// The phone that fills `body` (of deviceBodyAspect()), showing a picture of
// `pictureAspect` (width over height) across its screen's width.
DeviceGeometry deviceGeometry(const QRectF &body, double pictureAspect);
// The body's outline: a continuous (squircle-like) corner of bodyRadius,
// which leaves the straight edge with no jump in curvature and passes
// through the same 45-degree point as a circular corner of that radius.
QPainterPath deviceBodyPath(const DeviceGeometry &device);
// The side buttons, Action, volume up and down on the left, the side button
// and Camera Control on the right.
QList<QRectF> deviceButtons(const DeviceGeometry &device);
// The phone has no status bar and no camera on its screen: both empty.
// Deprecated, removed in W3.
QRectF deviceStatusBar(const DeviceGeometry &device);
QRectF deviceCamera(const DeviceGeometry &device, DevicePreview preview);

// The feed post's interface round a 1:1 or 4:5 picture, a skeleton in
// tokens::raised (corners of 4) on the screen's black, at the mockup's
// offsets from the picture's top left: an avatar and two lines over it,
// three action dots and two caption lines under it. Shrunk with a screen
// narrower than the stage's (213.7 pt) so it stays on the screen.
struct DeviceFeed {
    QRectF avatar;
    QRectF name;
    QRectF handle;
    QRectF actions[3];
    QRectF caption;
    QRectF captionMore;
};
DeviceFeed deviceFeed(const DeviceGeometry &device);

// The side buttons, the band with its outline, the glass and the black
// screen. `theme` is unused (the frame has one look); kept for the callers.
void paintDeviceBody(QPainter &painter, const DeviceGeometry &device, const EditorTheme &theme);
// The feed's skeleton (see DeviceFeed), on the screen's black.
void paintDeviceFeed(QPainter &painter, const DeviceGeometry &device);
// Draws nothing: the phone has no island, punch-hole or status bar. Nothing
// calls it since W2; deprecated, removed in W3.
void paintDeviceChrome(QPainter &painter, const DeviceGeometry &device, DevicePreview preview, const EditorTheme &theme);

// ----------------------------------------------------------------------- laptop

// Laptop, or Off (the plain card). A per-user preference (QSettings
// viewer/laptopPreview), never the project's.
enum class LaptopPreview { Laptop, Off };

QString laptopPreviewKey(LaptopPreview preview);
// The preview a settings value names; Laptop for anything else.
LaptopPreview laptopPreviewFromKey(const QString &key);
QString laptopPreviewLabel(LaptopPreview preview);

struct LaptopGeometry {
    QRectF body;    // Lid top to base bottom; the base is as wide as the lid.
    QRectF lid;     // The lid, its rim and glass included.
    QRectF glass;   // The black glass inside the rim.
    QRectF screen;  // The display, 3024:1964.
    QRectF picture; // The picture, across the screen's width, centred on it.
    QRectF notch;   // Always empty: there is no notch. Deprecated, removed in W3.
    QRectF hinge;   // The lid's bottom rim, in the hinge's tone, inset by the rim at both ends.
    QRectF base;    // The base's front face under the lid.
    QRectF scoop;   // The thumb scoop, cut from the base's top lip.
    qreal lidRadius = 0;       // The lid's top corners: the screen's plus the border (concentric).
    qreal lidFootRadius = 0;   // The lid's bottom corners.
    qreal glassRadius = 0;     // The glass's top corners.
    qreal screenRadius = 0;    // The screen's top corners; its bottom ones are square.
    qreal baseRadius = 0;      // The base's bottom corners.
    qreal baseTopRadius = 0;   // The base's top corners.
    qreal rim = 0;             // The aluminium round the glass.
    qreal bezel = 0;           // The border, rim and glass, on every side.
    qreal topBezel = 0;        // The same as bezel: the top is as thin as the sides.
};

// The laptop's proportions as shares of its lid's width (the screen's
// radius of the screen's width, the scoop's width of the base's).
constexpr double kLaptopBodyAspect = device_mm::laptopWidth / device_mm::laptopHeight;        // 1.43790
constexpr double kLaptopScreenWidth = device_mm::laptopScreenWidth / device_mm::laptopWidth;  // 0.96737
constexpr double kLaptopScreenAspect = device_mm::laptopScreenWidth / device_mm::laptopScreenHeight; // 1.53971
constexpr double kLaptopBezel = device_mm::laptopBorder / device_mm::laptopWidth;             // 0.016315
constexpr double kLaptopGlass = device_mm::laptopGlass / device_mm::laptopWidth;              // 0.011197
constexpr double kLaptopRim = device_mm::laptopRim / device_mm::laptopWidth;                  // 0.005118
constexpr double kLaptopLidHeight = device_mm::laptopLidHeight / device_mm::laptopWidth;      // 0.66091
constexpr double kLaptopBaseHeight = device_mm::laptopBase / device_mm::laptopWidth;          // 0.034549
// The base is the lid's width (no overhang), so the body is too.
constexpr double kLaptopBaseWidth = 1.0;
constexpr double kLaptopBodyWidth = kLaptopBaseWidth;
// Corners, estimates (Apple publishes none): the screen's top ones 1.0% of
// its width, its bottom ones square; the lid's foot 0.45%; the base's bottom
// 0.7%, its top 1 pt.
constexpr double kLaptopScreenRadius = 0.010;
constexpr double kLaptopLidFootRadius = 0.0045;
constexpr double kLaptopBaseRadius = 0.007;
constexpr double kLaptopBaseTopRadius = 1.0; // points
// The base's top lip, in points, and the thumb scoop: 23% of the base's
// width, 2.5 mm deep, its ends elliptical.
constexpr double kLaptopLip = 1.0;
constexpr double kLaptopScoopWidth = 0.23;
constexpr double kLaptopScoopDepth = device_mm::laptopScoopDepth / device_mm::laptopWidth;
// The glass-edge line just outside the screen, in points.
constexpr double kLaptopGlassEdge = 1.0;

// The body's width over its height.
double laptopBodyAspect();
// The share of the body's width a picture across the screen takes.
double laptopPictureShare();
// The laptop that fills `body` (of laptopBodyAspect()), showing a picture of
// `pictureAspect` (width over height) across its screen's width.
LaptopGeometry laptopGeometry(const QRectF &body, double pictureAspect);
// Always empty: the laptop has no notch or camera. Deprecated, removed in W3.
QRectF laptopCamera(const LaptopGeometry &laptop);
// The lid with its rim, glass, glass-edge line, black screen and hinge, and
// the base with its top lip and thumb scoop. `theme` is unused.
void paintLaptopBody(QPainter &painter, const LaptopGeometry &laptop, const EditorTheme &theme);

// ------------------------------------------------------------------------ cache

// A device's frame (everything that does not change from one frame of video
// to the next) drawn once into a pixmap for a size, offset within a pixel,
// picture shape (for the phone's feed) and pixel ratio, then only blitted, so
// showing the picture in a phone or a laptop costs playback one blit.
class DeviceFrameCache {
public:
    enum class Kind { Phone, Laptop };
    // Room round the body for the phone's side buttons.
    static constexpr int kMargin = 2;
    // Draws the frame of `kind` whose body is `body` for a picture of
    // `pictureAspect`, rendering it first only when the kind, the body's
    // size or offset within a pixel, the picture's shape or the pixel ratio
    // changed. A phone with a 1:1 or 4:5 picture carries the feed.
    void paint(QPainter &painter, Kind kind, const QRectF &body, double pictureAspect, qreal ratio);
    // The same for a 9:16 phone or a 16:9 laptop. `theme` is unused.
    void paint(QPainter &painter, Kind kind, const QRectF &body, const EditorTheme &theme, qreal ratio);
    // How many times a frame was rendered rather than blitted.
    int renders() const { return m_renders; }

private:
    QPixmap m_pixmap;
    Kind m_kind = Kind::Phone;
    QRectF m_local; // The body within the pixmap.
    bool m_feed = false;
    double m_aspect = 0;
    qreal m_ratio = 0;
    int m_renders = 0;
};

// The broadcast safe zones over the picture (mockup-spec 3.3): action safe,
// a 1 pt line just inside a rect 3.5% in from every edge (93% of the
// picture), and title safe, 5% in (90%), shown by 16 pt corner ticks and a
// 12 pt cross at the centre. They apply to every format.
constexpr double kActionSafeInset = 0.035;
constexpr double kTitleSafeInset = 0.05;
constexpr double kSafeTick = 16.0;
constexpr double kSafeCross = 12.0;
struct SafeZones {
    QRectF action; // 93% of the picture
    QRectF title;  // 90% of the picture
    QRectF cross;  // the 12 x 12 cross at the picture's centre
};
SafeZones safeZones(const QRectF &picture);
// The action-safe line (rgba(255,255,255,.22)), the title-safe ticks and
// the cross (rgba(255,255,255,.45)). `theme` is unused.
void paintSafeZones(QPainter &painter, const QRectF &picture, const EditorTheme &theme);

} // namespace kerf
