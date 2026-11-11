#pragma once

// The editor's design tokens: every colour, type style, radius, spacing,
// control height, motion and layout size the review-thread design uses, and
// nothing else. Values are the mockup's (research/qt-port/mockup-spec.md
// sections 0.1-0.4), where one mockup pixel is one device-independent point.
// The editor is dark only, so each token has one value.
//
// Header only, and free of widgets: constexpr data in kerf::tokens, plus a
// few inline helpers that turn it into QColor, QFont and QEasingCurve.

#include <QColor>
#include <QEasingCurve>
#include <QFont>
#include <QFontDatabase>
#include <QPointF>
#include <QRgb>
#include <QString>
#include <QStringList>
#include <QtGlobal>

namespace kerf::tokens {

// ---------------------------------------------------------------- colour (0.2)
// Each colour is a QRgb (0xAARRGGBB); color() makes a QColor of it.

constexpr QRgb surround = qRgb(0x0e, 0x0e, 0x0e);    // around the region picker's frame
constexpr QRgb surroundDim = qRgb(0x05, 0x05, 0x05); // window background
constexpr QRgb well = qRgb(0x12, 0x12, 0x12);        // bin grid, timeline lanes, scope plots, vectorscope chip
constexpr QRgb chrome = qRgb(0x17, 0x17, 0x17);      // toolbar, strips, inspector, bands, status, pin bar, diff box
constexpr QRgb card = qRgb(0x1c, 0x1c, 0x1c);        // comments panel, Source viewer, menus, tooltips, clips
constexpr QRgb raised = qRgb(0x26, 0x26, 0x26);      // hover fill, segmented track, proposal card, composer, meter track
constexpr QRgb on = qRgb(0x30, 0x30, 0x30);          // selected/on fill, avatars, scrub markers, sliding segment fill
constexpr QRgb hover2 = qRgb(0x36, 0x36, 0x36);      // hover over a raised surface, flash, selected avatar
constexpr QRgb ink = qRgb(0xed, 0xed, 0xed);         // primary text, slider knob
constexpr QRgb ink2 = qRgb(0xa8, 0xa8, 0xa8);        // secondary text, icon default
constexpr QRgb ink3 = qRgb(0x82, 0x82, 0x82);        // tertiary text, meta
constexpr QRgb dis = qRgb(0x5c, 0x5c, 0x5c);         // disabled
constexpr QRgb track = qRgb(0x3a, 0x3a, 0x3a);       // slider/progress track, minor ruler ticks, scope graticule
constexpr QRgb track2 = qRgb(0x6a, 0x6a, 0x6a);      // slider fill, major ticks, played bar

// Alpha as the mockup writes it, a fraction 0..1, rounded to 0..255.
constexpr int alpha(double fraction) { return int(fraction * 255.0 + 0.5); }

constexpr QRgb line = qRgba(255, 255, 255, alpha(0.08));  // hairlines
constexpr QRgb line2 = qRgba(255, 255, 255, alpha(0.14)); // control outlines
constexpr QRgb lineh = qRgba(255, 255, 255, alpha(0.22)); // hover outline

constexpr QRgb accent = qRgb(0x33, 0x66, 0xe6);                   // primary button, focus ring, selected range, pinned stripe
constexpr QRgb accentHover = qRgb(0x2b, 0x5a, 0xd6);              // primary hover
constexpr QRgb accentInk = qRgb(0x8f, 0xb0, 0xff);                // timecode links, accepted text, "Ask Kerf" action
constexpr QRgb accentWash = qRgba(0x33, 0x66, 0xe6, alpha(0.14)); // defined by the design, unused in the mockup
constexpr QRgb accentText = qRgb(0xff, 0xff, 0xff);               // text on an accent fill (.btn.p)
constexpr QRgb pos = qRgb(0x4c, 0xc3, 0x8a);                      // pass dots, diff additions, connected
constexpr QRgb neg = qRgb(0xec, 0x5d, 0x61);                      // fail, blocking, overlap box, diff removals, meter top
constexpr QRgb playhead = qRgb(0xe5, 0x48, 0x4d);                 // playheads
constexpr QRgb warn = qRgb(0xd6, 0xa3, 0x45);                     // meter -18...-6 dBFS band

// Literal colours drawn on the picture and in the lanes.
namespace overlay {
constexpr QRgb lowerThirdBackground = qRgba(12, 12, 12, alpha(0.78));
constexpr QRgb lowerThirdStripe = qRgb(0xe9, 0xd8, 0xb8);
constexpr QRgb lowerThirdTitle = qRgb(0xf4, 0xf1, 0xea);
constexpr QRgb lowerThirdSub = qRgb(0xb9, 0xb2, 0xa5);
constexpr QRgb captionBackground = qRgba(0, 0, 0, alpha(0.72));
constexpr QRgb captionText = qRgb(0xff, 0xff, 0xff);
// A waveform's peak and, over it, its RMS; room tone in a darker grey.
constexpr QRgb waveformPeak = qRgba(144, 144, 144, alpha(0.41));
constexpr QRgb waveformRms = qRgba(144, 144, 144, alpha(0.75));
constexpr QRgb roomTonePeak = qRgba(106, 106, 106, alpha(0.41));
constexpr QRgb roomToneRms = qRgba(106, 106, 106, alpha(0.75));
constexpr QRgb rubberBand = qRgb(0xa8, 0xa8, 0xa8);
constexpr QRgb keyframe = qRgb(0xed, 0xed, 0xed);
// The black around a letterboxed picture.
constexpr QRgb deviceScreen = qRgb(0x00, 0x00, 0x00);
} // namespace overlay

// The popover shadow (--pop): 0 8px 24px rgba(0,0,0,.45), always drawn with an
// inside 1 pt line2 outline.
struct Shadow {
    qreal dx, dy, blur;
    QRgb color;
};
constexpr Shadow pop{0, 8, 24, qRgba(0, 0, 0, alpha(0.45))};
// The slider knob's shadow: 0 1px 2px rgba(0,0,0,.6).
constexpr Shadow knob{0, 1, 2, qRgba(0, 0, 0, alpha(0.6))};
// The sliding segmented fill's top highlight: inset 0 1px 0 rgba(255,255,255,.06).
constexpr QRgb segmentHighlight = qRgba(255, 255, 255, alpha(0.06));
// The checkbox's outline: inset 0 0 0 1px rgba(255,255,255,.16).
constexpr QRgb checkboxOutline = qRgba(255, 255, 255, alpha(0.16));

inline QColor color(QRgb rgba) { return QColor::fromRgba(rgba); }

// ------------------------------------------------------------------ type (0.3)

// A size/line pair: pixel size and line height in points, weight, and face.
struct TypeStyle {
    int px;
    int line;
    int weight; // CSS weight: 400 regular, 500 medium
    bool mono;
};

constexpr TypeStyle body{13, 20, 400, false};          // body, comment text, inputs
constexpr TypeStyle bodyName{13, 20, 500, false};      // comment names
constexpr TypeStyle display{34, 44, 500, false};       // the start window's question
constexpr TypeStyle reading{14, 22, 400, false};       // what is said in the conversation, and the composer
constexpr TypeStyle title{14, 20, 500, false};         // panel titles: Comments, Source, clip name
constexpr TypeStyle control{12, 16, 400, false};       // controls, labels, rows, chips
constexpr TypeStyle controlStrong{12, 16, 500, false}; // buttons, dock names, segment on
constexpr TypeStyle meta{11, 16, 400, false};          // meta, roles, small labels, subtitle
constexpr TypeStyle metaStrong{11, 16, 500, false};    // avatar initials
constexpr TypeStyle timecode{12, 16, 400, true};       // timecodes in rows and links, values
constexpr TypeStyle timecodeStrong{12, 16, 500, true}; // the transport's current timecode
constexpr TypeStyle monoSmall{11, 16, 400, true};      // counts, shortcuts, units, ticks
constexpr TypeStyle monoLarge{13, 20, 500, true};      // timeline corner timecode, loudness stats
constexpr TypeStyle timecodeLarge{20, 24, 500, true};  // the timecode under a vertical picture
constexpr TypeStyle caption{15, 20, 400, false};       // caption on the picture
constexpr TypeStyle badge{11, 12, 400, true};          // bin duration badge

// Letter spacing is 0 everywhere but the scrub marker initials.
constexpr qreal markerInitialsSpacing = -0.2;

// The bundled families (resources/fonts) and what stands in if one fails to load.
constexpr char uiFamily[] = "Inter";
constexpr char monoFamily[] = "JetBrains Mono";
constexpr char monoFallback[] = "Menlo";

// The QFont for `style`: the bundled face, else the system's UI font or
// Menlo (then the system fixed font), at the style's pixel size and weight,
// with tabular figures.
inline QFont font(const TypeStyle &style) {
    QFont font;
    if (style.mono)
        font.setFamilies({QString::fromLatin1(monoFamily), QString::fromLatin1(monoFallback),
                          QFontDatabase::systemFont(QFontDatabase::FixedFont).family()});
    else
        font.setFamilies({QString::fromLatin1(uiFamily), QFontDatabase::systemFont(QFontDatabase::GeneralFont).family()});
    font.setPixelSize(style.px);
    font.setWeight(QFont::Weight(style.weight));
    font.setLetterSpacing(QFont::AbsoluteSpacing, 0);
#if QT_VERSION >= QT_VERSION_CHECK(6, 7, 0)
    font.setFeature(QFont::Tag("tnum"), 1);
    if (style.mono) font.setFeature(QFont::Tag("zero"), 0);
#endif
    return font;
}

// ---------------------------------------------------------- radii and spacing (0.4)

namespace radius {
constexpr qreal small = 4;   // small chips, clips, thumbs, keyframe button, segment inner
constexpr qreal marker = 5;  // agent scrub marker
constexpr qreal control = 6; // buttons, icon buttons, menu items, chips
constexpr qreal card = 8;    // cards, menus, composer, proposal card, HUD
constexpr qreal panel = 12;  // comments panel, Source viewer
} // namespace radius

// Spacing is on a 4 grid; these are the steps the design uses.
namespace space {
constexpr int s2 = 2;
constexpr int s4 = 4;
constexpr int s6 = 6;
constexpr int s8 = 8;
constexpr int s12 = 12;
constexpr int s16 = 16;
// A panel's inner padding: 16 left and 12 right in headers, 16 both in lists.
constexpr int panelLeft = 16;
constexpr int panelHeaderRight = 12;
constexpr int panelListRight = 16;
} // namespace space

// Control heights: track toggles and inner buttons, controls, toolbar
// controls and menu items, and the larger rows.
namespace height {
constexpr int h20 = 20;
constexpr int h24 = 24;
constexpr int h28 = 28;
constexpr int h32 = 32;
constexpr int h40 = 40;
} // namespace height

// Focus: a 2 pt accent outline 1 pt outside the control.
constexpr qreal focusWidth = 2;
constexpr qreal focusOffset = 1;

// ---------------------------------------------------------------- motion (0.2)

namespace motion {
constexpr int hover = 120;   // hover and colour changes, ms
constexpr int fade = 160;    // popovers and fades
constexpr int segment = 200; // the segmented control's sliding fill
constexpr int move = 220;    // the kit's larger moves
constexpr int stage = 850;   // the window's regions moving to a new arrangement
// Motion UI: what each scene takes (MOTION_UI.md says which is which).
constexpr int settle = 420;       // falling back or coming forward: dims, the veil
constexpr int appear = 320;       // a thing fading in where it is
constexpr int rise = 620;         // a thing moving up into place
constexpr int riseStagger = 90;   // between the start window's parts
constexpr int flight = 720;       // a tile going to where it lands
constexpr int flightLand = 240;   // its fade once landed, over what it became
constexpr int flightStagger = 160;
constexpr int standInRelease = 260; // what stood in for the picture, fading
constexpr int page = 520;         // Settings' pages coming up into the room
constexpr int handOver = 460;     // the start window fading over the editor
constexpr int pop = 460;          // a footage tile appearing
constexpr int popStagger = 70;
constexpr int fanStagger = 110;   // between the version cards
constexpr int timelineLead = 150; // how long before the stage lands the timeline starts in
constexpr int cutSettle = 500;    // edits being quiet before a proposed cut renders
constexpr int workPulse = 1200;   // the working chip's dot, once
constexpr int compareSweep = 6800; // the before/after wipe, there and back
constexpr int workPulseSteps = 24; // the dot's redraws in a pulse: it is slow, and need not be drawn every frame
constexpr int posterMost = 1280;   // the widest a poster is kept while it moves about
// How far things go.
constexpr int pageRise = 16;          // points Settings' pages come up
constexpr qreal compareFrom = 0.28;   // the wipe's line, across the picture
constexpr qreal compareTo = 0.78;
constexpr qreal chatBehind = 0.28;    // the conversation while the picture is watched
constexpr qreal timelineBehind = 0.6; // the room over the timeline under version cards
constexpr qreal cardLean = 1.5;       // degrees between version cards
constexpr qreal clipLift = 8.0;       // points a watched clip rises off its lane
constexpr qreal landOvershoot = 1.05; // a flight going past where it lands
constexpr qreal popOvershoot = 1.4;   // a tile going past its size
// The one easing curve: cubic-bezier(.22, 1, .36, 1).
constexpr qreal easeX1 = 0.22;
constexpr qreal easeY1 = 1.0;
constexpr qreal easeX2 = 0.36;
constexpr qreal easeY2 = 1.0;
} // namespace motion

inline QEasingCurve easing() {
    QEasingCurve curve(QEasingCurve::BezierSpline);
    curve.addCubicBezierSegment(QPointF(motion::easeX1, motion::easeY1), QPointF(motion::easeX2, motion::easeY2),
                                QPointF(1, 1));
    return curve;
}

// ---------------------------------------------------------------- layout (0.1)
// Default sizes at the 1600 x 1000 design; the fluid parts take what is left.

namespace layout {
constexpr int canvasWidth = 1600;
constexpr int canvasHeight = 1000;
constexpr int toolbar = 40;     // toolbar height
constexpr int strip = 28;       // the sheets' strips, the timeline's
constexpr int bin = 272;        // the Media sheet's width
// The minimal layout (ui/stage_shell.h).
constexpr int sidebar = 240;            // the folders and conversations, at the left
constexpr int sidebarFoot = 48;         // the account at its foot
constexpr int sidebarWindowLeast = 1100; // the window's width under which the sidebar gives way
constexpr int rail = 400;               // the conversation, beside an open panel
constexpr int railLeast = 320;          // its least width
constexpr int workLeast = 600;          // what it leaves the panel before it gives way
constexpr int conversationColumn = 720; // the width its text keeps when it has the room to itself
constexpr int reelBar = 40;             // the strip over the picture while watching changes
constexpr int timelineWideLeast = 260;  // the timeline's least height under a wide picture
constexpr double timelineWideShare = 0.5625; // and its share of the content's height there
constexpr int pictureTransportLow = 88; // the transport's block under a picture with no timeline to carry it
constexpr int picturePad = 12;          // around the picture in its slot
constexpr int pictureRadius = 10;       // the picture's corners
constexpr int reelReach = 120;          // how far the stage grows over the conversation while watching changes
constexpr int pictureColumnLeast = 240; // a vertical picture's column, at least
constexpr int timelineUprightLeast = 220; // and under a vertical one
constexpr double timelineUprightShare = 0.36; // and its share of the height there
constexpr int seam = 1;                // every seam is a 1 pt line
} // namespace layout

} // namespace kerf::tokens
