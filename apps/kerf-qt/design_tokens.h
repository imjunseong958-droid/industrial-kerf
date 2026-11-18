#pragma once

// The editor's design tokens: every colour, type style, radius, spacing,
// control height, motion and layout size the editor's design uses, and
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
constexpr QRgb card = qRgb(0x1c, 0x1c, 0x1c);        // Source viewer, menus, tooltips, clips
constexpr QRgb sidebar = qRgb(0x1e, 0x1e, 0x1e);     // the folders and conversations, a step up from the room
constexpr QRgb raised = qRgb(0x26, 0x26, 0x26);      // hover fill, segmented track, proposal card, composer, meter track
constexpr QRgb on = qRgb(0x30, 0x30, 0x30);          // selected/on fill, avatars, scrub markers, sliding segment fill
constexpr QRgb hover2 = qRgb(0x36, 0x36, 0x36);      // hover over a raised surface, flash, selected avatar
// What opens from a press (menus, popovers, the composer's suggestions) is a
// step up from what it opens over, and the row under the pointer a step more.
constexpr QRgb popup = raised;
constexpr QRgb popupHover = hover2;
// The inks are paper, not white: the same steps as the mockup's greys, warmed
// a little, as type on an old page is. The surfaces stay neutral.
constexpr QRgb ink = qRgb(0xec, 0xe6, 0xda);         // primary text, slider knob
constexpr QRgb ink2 = qRgb(0xad, 0xa7, 0x9b);        // secondary text, icon default
constexpr QRgb ink3 = qRgb(0x87, 0x82, 0x78);        // tertiary text, meta
constexpr QRgb dis = qRgb(0x5c, 0x5c, 0x5c);         // disabled
constexpr QRgb track = qRgb(0x3a, 0x3a, 0x3a);       // slider/progress track, minor ruler ticks, scope graticule
constexpr QRgb track2 = qRgb(0x6a, 0x6a, 0x6a);      // slider fill, major ticks, played bar

// Alpha as the mockup writes it, a fraction 0..1, rounded to 0..255.
constexpr int alpha(double fraction) { return int(fraction * 255.0 + 0.5); }

constexpr QRgb line = qRgba(255, 255, 255, alpha(0.08));  // hairlines
constexpr QRgb line2 = qRgba(255, 255, 255, alpha(0.14)); // control outlines
constexpr QRgb lineh = qRgba(255, 255, 255, alpha(0.22)); // hover outline

// The colours that say something are faded, as the inks are warmed: printed
// rather than lit, so they sit with the paper inks instead of shouting over
// them. Each keeps its contrast: white on the accent 5.3:1, the accent's ring
// on the chrome 3:1, the rest over 6:1 on the chrome. Only the computer's
// amber (activity::gpu) stays bright, as the one thing meant to catch the eye.
constexpr QRgb accent = qRgb(0x4a, 0x6a, 0xb0);                   // primary button, focus ring, selected range, pinned stripe
constexpr QRgb accentHover = qRgb(0x40, 0x5e, 0x9e);              // primary hover
constexpr QRgb accentInk = qRgb(0xa0, 0xb4, 0xd8);                // timecode links, accepted text
constexpr QRgb accentWash = qRgba(0x4a, 0x6a, 0xb0, alpha(0.14)); // defined by the design, unused in the mockup
constexpr QRgb accentText = qRgb(0xff, 0xff, 0xff);               // text on an accent fill (.btn.p)
constexpr QRgb pos = qRgb(0x86, 0xa8, 0x7c);                      // sage: pass dots, diff additions, connected
constexpr QRgb neg = qRgb(0xd8, 0x80, 0x6e);                      // faded brick: fail, blocking, overlap box, diff removals, meter top
constexpr QRgb playhead = qRgb(0xa0, 0x84, 0xd4);                 // playheads: a dusty violet, the one in the window, found at once
constexpr QRgb warn = qRgb(0xc8, 0xa2, 0x64);                     // ochre: meter -18...-6 dBFS band, what wants a look

// A clip an open proposal changes: sage, as an addition is in a diff. Not
// Kerf's blue, which says Kerf is at work, nor the violet playhead beside it.
constexpr QRgb proposed = pos;

// What a small mark says of a check. A check that did not pass wants a look,
// and its request ("Ask Kerf", "Run QC") fixes it: the same ochre for both,
// quieter than the computer's amber, which says only that something runs.
namespace status {
constexpr QRgb passed = pos;  // the check passed
constexpr QRgb request = warn; // it wants a look, and the request to fix it
constexpr QRgb failed = neg;  // what could not be done, what cannot be undone
} // namespace status

// What the editor is doing, in two hues only: Kerf at work (the Kerf cursor's
// blue) and the computer making pictures (a render, a model making a shot);
// and what a conversation is doing, as the dot before its title in the sidebar says.
namespace activity {
constexpr QRgb kerf = accentInk;                // Kerf looking or cutting: the light round the picture, its chip's dot
constexpr QRgb gpu = qRgb(0xe0, 0xa4, 0x48);    // rendering or making: the light, a clip's stripes, a tile's
constexpr QRgb gpuStripe = qRgba(0xe0, 0xa4, 0x48, alpha(0.30)); // the stripes over a clip or a tile being made
constexpr QRgb chip = qRgba(0, 0, 0, alpha(0.55));               // the chip's ground over the picture
// A conversation, in the sidebar.
// Working and making show on its title, as a light passing over it; the rest
// as a dot at its row's end, until it has been looked at.
constexpr QRgb review = accentInk;  // something waits for you to look at it
constexpr QRgb settled = accentInk; // done while you were elsewhere
constexpr QRgb failed = neg;        // it stopped on an error
} // namespace activity

// A shot's graph (ui/recipe_graph.h): what goes along an edge, each in one of
// the faded colours the editor has already, and no new one. A picture in
// motion is the accent's ink, a dusty blue as light as the others here; a
// still the sage, the way a thing goes the playhead's violet, a number over
// time the ochre. Words and any other file are quiet, in ink3.
namespace nodes {
constexpr QRgb canvas = well;        // the room the cards are laid out in
constexpr QRgb dots = qRgba(255, 255, 255, alpha(0.07)); // its grid, a dot a step
constexpr QRgb node = card;          // a card
constexpr QRgb video = accentInk;
constexpr QRgb image = pos;
constexpr QRgb path = playhead;
constexpr QRgb signal = warn;
constexpr QRgb words = ink3;
constexpr QRgb file = ink3;
} // namespace nodes

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
    // What is said in the conversation: set in the face and size chosen for
    // reading (faces()), where the rest of the interface keeps its own.
    bool prose = false;
};

constexpr TypeStyle body{13, 20, 400, false};          // body, inputs
constexpr TypeStyle bodyName{13, 20, 500, false};      // names in rows
constexpr TypeStyle side{15, 20, 400, false};          // the sidebar's rows, and lists set like them
constexpr TypeStyle sideName{15, 20, 500, false};      // the chosen one among them
constexpr TypeStyle sideNote{14, 20, 400, false};      // what is said beside them: a folder's name, the account, an empty list
constexpr TypeStyle display{34, 44, 500, false};       // the start window's question
constexpr TypeStyle reading{14, 22, 400, false, true}; // what is said in the conversation, and the composer
constexpr TypeStyle title{14, 20, 500, false};         // panel titles: the conversation, Source, clip name
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
constexpr TypeStyle menuItem{14, 20, 400, false};      // a menu's or a popover's rows
constexpr TypeStyle menuQuiet{13, 18, 400, false};     // what a row says at its end (a key, a value), a menu's headers, a popover's notes

// Letter spacing is 0 everywhere but the scrub marker initials.
constexpr qreal markerInitialsSpacing = -0.2;

// The bundled families (resources/fonts) and what stands in if one fails to load.
constexpr char uiFamily[] = "Inter";
constexpr char monoFamily[] = "JetBrains Mono";
constexpr char monoFallback[] = "Menlo";
// The other faces the editor carries, to choose from (Settings, Appearance):
// Pretendard, which is Inter with Hangul; and a serif pair for reading, Noto
// Serif for Latin with Nanum Myeongjo for Hangul.
constexpr char hangulFamily[] = "Pretendard";
// IBM Plex, one design across Latin, Hangul and the mono: an engineered
// face with a little of the typewriter in it. It is the face Kerf starts in;
// Inter, the mockup's, is one to choose.
constexpr char plexFamily[] = "IBM Plex Sans";
constexpr char plexHangulFamily[] = "IBM Plex Sans KR";
constexpr char plexMonoFamily[] = "IBM Plex Mono";
constexpr char serifFamily[] = "Noto Serif";
constexpr char serifHangulFamily[] = "NanumMyeongjo";

// The faces in use. The interface's families, first choice first: a glyph
// one lacks comes from the next, so Hangul under Inter is Pretendard's, not
// whatever the system has. What is said in the conversation may have a face
// and a size of its own (TypeStyle::prose); with no families it has the
// interface's. editor_theme.h's setEditorFaces() changes them and has every
// widget take them.
struct Faces {
    QStringList ui{QString::fromLatin1(plexFamily), QString::fromLatin1(plexHangulFamily), QString::fromLatin1(hangulFamily)};
    QStringList prose;
    // The mono's families, first choice first, before JetBrains Mono, which
    // the faces that have no mono of their own are set with.
    QStringList mono{QString::fromLatin1(plexMonoFamily)};
    int proseStep = 0; // points added to the reading size
};
inline Faces &faces() {
    static Faces chosen;
    return chosen;
}

// The QFont for `style`: the chosen face, else the system's UI font or
// Menlo (then the system fixed font), at the style's pixel size and weight,
// with tabular figures.
inline QFont font(const TypeStyle &style) {
    QFont font;
    const Faces &chosen = faces();
    if (style.mono) {
        QStringList families = chosen.mono;
        families << QString::fromLatin1(monoFamily) << QString::fromLatin1(monoFallback)
                 << QFontDatabase::systemFont(QFontDatabase::FixedFont).family();
        font.setFamilies(families);
    } else {
        QStringList families = style.prose && !chosen.prose.isEmpty() ? chosen.prose : chosen.ui;
        families << QFontDatabase::systemFont(QFontDatabase::GeneralFont).family();
        font.setFamilies(families);
    }
    font.setPixelSize(style.px + (style.prose ? chosen.proseStep : 0));
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
constexpr qreal panel = 12;  // Source viewer, cards
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
constexpr int panel = 420;   // a pane opened or closed by hand: the sidebar, the panel, the Media band
// Motion UI: what each scene takes (MOTION_UI.md says which is which).
constexpr int settle = 420;       // falling back or coming forward: dims, the veil
constexpr int appear = 320;       // a thing fading in where it is
constexpr int rise = 620;         // a thing moving up into place
constexpr int flight = 720;       // a tile going to where it lands
constexpr int flightLand = 240;   // its fade once landed, over what it became
constexpr int flightStagger = 160;
constexpr int standInRelease = 260; // what stood in for the picture, fading
constexpr int page = 520;         // Settings' pages coming up into the room
constexpr int pop = 460;          // a footage tile appearing
constexpr int popStagger = 70;
constexpr int fanStagger = 110;   // between the version cards
constexpr int timelineLead = 150; // how long before the stage lands the timeline starts in
constexpr int cutSettle = 500;    // edits being quiet before a proposed cut renders
constexpr int workPulse = 1200;   // the working chip's dot, once
constexpr int titleShimmer = 2200; // the light once over a working conversation's title in the sidebar, and the rest after it
constexpr int titleShimmerSteps = 66; // its redraws in that time: 30 a second, the title alone
constexpr int workShimmer = 1600; // the light once over "Working" while Kerf works
constexpr int compareSweep = 6800; // the before/after wipe, there and back
constexpr int cardTurn = 2200;     // the light once round a cut's card while Kerf makes it
constexpr int cardFrame = 50;      // how often that light is drawn again
constexpr int cardCrossfade = 600; // the card's picture becoming the newest clip
constexpr int cardRing = 900;      // done: the ring going out, and the picture lightening
constexpr int cardDone = 2400;     // done: how long it says so
constexpr int frameStream = 160;   // frames closer than this are playing: drawn as they come
constexpr int frameSettle = 220;   // after the last of them, the picture is drawn at its best
constexpr int workPulseSteps = 24; // the dot's redraws in a pulse: it is slow, and need not be drawn every frame
constexpr int unfoldStagger = 30;  // between the sidebar's archived conversations as they unfold
constexpr int posterMost = 1280;   // the widest a poster is kept while it moves about
// What the editor is doing (MOTION_UI.md: Stage at work, Making a shot).
constexpr int edgeTurn = 3200;     // the light once round the picture while something works
constexpr int edgeFrame = 50;      // how often that light is drawn again
constexpr int edgeFade = 420;      // the light and its chip coming and going
constexpr int stripeCycle = 1600;  // the stripes over a clip or a tile being made, moving one stripe on
constexpr int stripeFrame = 80;    // how often they are drawn again
constexpr int clockFrame = 1000;   // a making's clock, and all that moves under Reduce Motion
// How far things go.
constexpr int pageRise = 16;          // points Settings' pages come up
constexpr qreal edgeTail = 0.22;      // the light's tail, a share of the way round
constexpr qreal edgeWidth = 2.0;      // points the light is wide, outside the picture's edge
constexpr qreal edgeGap = 1.0;        // and how far out from it
constexpr qreal edgeRest = 0.28;      // the rest of the ring, under the light, as a share of it
constexpr qreal stripeSpace = 10.0;   // points from one stripe to the next
constexpr qreal titleCross = 0.7;     // the share of a title's pass the light takes to cross it
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
constexpr int bin = 272;        // the Media bin's width where it has a column of its own (the theme's sample)
constexpr int mediaBand = 152;  // the Media band's height, between the picture and the timeline
constexpr int mediaBandTimelineLeast = 200; // what it leaves the timeline under it before the picture gives way
// The minimal layout (ui/stage_shell.h).
constexpr int sidebar = 280;            // the folders and conversations, at the left, until its edge is dragged
constexpr int sidebarLeast = 232;       // how narrow a drag leaves it
constexpr int sidebarLeaves = 640;      // and what it leaves of the window at least, however wide it is dragged
constexpr int sidebarGrip = 16;         // what of its edge takes the drag, half either side of it
constexpr int sidebarGripLine = 2;      // the line it shows under the pointer
// The widest a drag leaves the sidebar in a window `window` wide.
constexpr int sidebarMost(int window) { return window - sidebarLeaves > sidebarLeast ? window - sidebarLeaves : sidebarLeast; }
// What runs along the window's foot (the sidebar's account and Settings, the
// row under the composer) is centred on one line, this far over the bottom
// edge, as what runs along its top is centred in the toolbar; and set alike:
// controls 28 tall, icons 16, words in UI 14. On both lines the words'
// capitals, not their line boxes, are centred where the icons' ink is
// (ui::capBaseline, ui::CapLabel); a smaller line beside a larger one sits on
// its baseline.
constexpr int footLine = 26;
constexpr int footControl = 28;
constexpr int footIcon = 16;
constexpr int sidebarFoot = 36;         // the account at its foot
constexpr int sidebarWindowLeast = 1100; // the window's width under which the sidebar gives way
constexpr int rail = 448;               // the conversation, beside an open panel
constexpr int railLeast = 320;          // its least width
constexpr int workLeast = 600;          // what it leaves the panel before it gives way
constexpr int conversationColumn = 720; // the width its text keeps when it has the room to itself
constexpr int reelBar = 40;             // the strip over the picture while watching changes
constexpr int timelineWideLeast = 260;  // the timeline's least height under a wide picture
constexpr double timelineWideShare = 0.5625; // and its share of the content's height there
constexpr int pictureTransportLow = 88; // the transport's block under a picture with no timeline to carry it
constexpr int picturePad = 12;          // around the picture in its slot
constexpr int pictureRadius = 10;       // the picture's corners
constexpr int cardPictureWidest = 440;  // a cut's picture on its card in the conversation, at most
constexpr int cardPictureTallest = 320; // and how tall an upright one is, at most
constexpr int cardRing = 3;             // around it, for the light while Kerf makes it
constexpr int reelReach = 120;          // how far the stage grows over the conversation while watching changes
constexpr int pictureColumnLeast = 240; // a vertical picture's column, at least
constexpr int timelineUprightLeast = 220; // and under a vertical one
constexpr double timelineUprightShare = 0.36; // and its share of the height there
constexpr int seam = 1;                // every seam is a 1 pt line
// A shot's graph (ui/recipe_graph.h).
constexpr int graphCardWidth = 208;     // a node's card
constexpr int graphCardHeight = 56;     // its title and the line under it
constexpr int graphCardCurve = 36;      // what a signal's curve or a path adds to it
constexpr int graphColumnGap = 112;      // between one column of cards and the next, for the edges to turn in
constexpr int graphRowGap = 20;         // between a card and the one under it
constexpr int graphGrid = 24;           // the canvas's dots
constexpr int graphInspector = 312;     // the card that says and sets what the chosen node holds
constexpr int graphPad = 12;            // what floats over the canvas keeps this far from its edges
constexpr int graphPanelWidth = 1480;   // the panel it opens in, over the window, at most
constexpr int graphPanelHeight = 900;
constexpr int graphBar = 48;            // the panel's head: the shot, what it wants, Make
constexpr int graphPartWidth = 232;     // a node of a model's workflow: its name over a row an input
constexpr int graphPartGap = 64;        // between its columns: its rows name what comes in, so the edges need less room
constexpr int graphPartHead = 28;       // its name's row
constexpr int graphPartRow = 20;        // a row of it, an input
} // namespace layout

} // namespace kerf::tokens
