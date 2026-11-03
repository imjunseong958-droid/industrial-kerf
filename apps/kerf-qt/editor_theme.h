#pragma once

#include <QColor>
#include <QJsonObject>
#include <QObject>
#include <QPalette>
#include <QString>

namespace kerf {

// The editor is dark only: one graphite ramp and one blue accent, from
// design_tokens.h. What follows in this block is the old appearance API,
// kept so code that still names it compiles.
//
// Deprecated, removed in W3: every scheme draws the one dark theme.
enum class ColorScheme { System, Light, Dark };

// A colour written as CSS writes it: #rrggbb, or #rrggbbaa with the alpha
// last (QColor reads eight digits as #aarrggbb).
QColor cssColor(const char *css);
// `over` laid on `under` at `amount` (0 to 1), alpha included.
QColor mixColor(const QColor &under, const QColor &over, qreal amount);
// WCAG contrast ratio of two opaque colours (1 to 21).
qreal contrastRatio(const QColor &a, const QColor &b);
// `color` moved towards white (on a dark `background`) or black (on a light
// one), in `steps` steps, until it reads at `ratio`; unchanged if it does.
QColor readableOn(const QColor &color, const QColor &background, qreal ratio, int steps = 20);
// `color` made only as much more opaque as it takes to read at `ratio` when
// laid on the opaque `background` (opaque at most).
QColor readableAlphaOn(const QColor &color, const QColor &background, qreal ratio);

// How one kind of clip is drawn on the timeline. Labels on `header` meet
// 4.5:1.
struct ClipStyle {
    QColor fill;   // The body under the filmstrip or waveform.
    QColor header; // The name bar along the clip's top edge (the whole clip on short rows).
    QColor border; // The 1px outline of a clip that is not selected.
    QColor label;  // The clip's name (and speed) on `header`.
    QColor wave;   // Waveform ink on `fill`, and the hue for the track header's preview.
};

// What a clip is, for its colours: picture media and nested sequences,
// sound, music (a sound track named or typed music or score), a generator
// or title, or a caption.
enum class ClipKind { Video, Audio, Music, Generator, Caption };

// The kind of `clip` on `track`, as the snapshot describes them (track
// `type` and `name`, clip `kind`). An empty clip classifies the track.
ClipKind clipKind(const QJsonObject &track, const QJsonObject &clip = {});
// The kind a track's header stands for: its first clip's, so a picture
// track of titles reads as titles, else the track's own. The timeline's
// header bar and the Inspector's track list both use it.
ClipKind trackKind(const QJsonObject &track);

// Every colour the editor draws, by role. The application style sheet is
// generated from these, and painted widgets read them at paint time from
// editorTheme(). Each role is one of design_tokens.h's colours (the graphite
// ramp, the lines, the accent family, pos/neg/warn and the literal overlay
// colours), or one laid on the surface it is drawn on, or a guard's step
// from one where a role must read at a ratio. The project launcher draws
// from the same ramp. The blue accent marks Kerf, focus, proposals and the
// default action; the playhead is its own red.
struct EditorTheme {
    // Deprecated, removed in W3: always true.
    bool dark = true;

    // The design's tokens themselves (design_tokens.h 0.2), so the style
    // sheet and painted widgets can name the ramp directly.
    QColor surround;    // Program stage background.
    QColor surroundDim; // The stage with Dim on; the window behind everything.
    QColor card;        // Comments panel, Source viewer, menus, tooltips, clips.
    QColor on;          // Selected or on fill, avatars, the sliding segment.
    QColor hover2;      // Hover over a `raised` surface.
    QColor ink;         // Primary text, slider knob.
    QColor ink2;        // Secondary text, icon default.
    QColor ink3;        // Tertiary text, meta: 4.66:1 on chrome, under 4.5:1 on card and raised.
    QColor dis;         // Disabled.
    QColor track;       // Slider and progress track, minor ticks.
    QColor track2;      // Slider fill, major ticks, played bar.
    QColor line;        // Hairlines (white 8%).
    QColor line2;       // Control outlines (white 14%).
    QColor lineh;       // Hover outline (white 22%).
    QColor accentHover; // The primary button under the pointer.
    QColor accentInk;   // Timecode links, accepted text, "Ask Kerf" actions.
    QColor accentWash;  // The accent at 14%.
    QColor pos;         // Pass, diff additions, connected.
    QColor neg;         // Fail, blocking, diff removals.
    QColor warn;        // The meter's -18...-6 band, warnings.

    // Accent: Kerf's own (focus, the default button, the playhead, checked
    // tools, progress). It does not follow the system accent.
    QColor accent;
    QColor accentTop;         // Deprecated, removed in W3: the accent itself (bezels are flat).
    QColor accentPressed;     // An accent button held down.
    QColor accentText;        // The accent as text or a thin line, 4.5:1 on every surface it is drawn on:
                              // `panel` (the transports' timecodes), `rulerCorner` (the timeline's
                              // corner time), and `statusBar` (the sync chip).
    QColor accentTint;        // A quiet accent wash on `panel`: a checked tool, an info badge, the chosen
                              // mask tool and the selected mask. `accentText` reads on it at 3:1.
    QColor accentTintPressed; // `accentTint` darkened while held, as a bezel darkens; `accentText`
                              // still reads on it at 3:1.
    QColor onAccent;          // Text and glyphs on `accent`, 4.5:1: white.
    QColor onAccentSecondary; // Secondary text on `selection`: `selectionText` held back only as far as
                              // it still reads at 4.5:1.

    // Surfaces, from the back of the window forwards.
    QColor window;           // Behind everything: the main window, the dock container.
    QColor chrome;           // Toolbar, menu bar, and tool palette.
    QColor panel;            // A docked panel's body: monitors' frames, timeline, inspector, transport.
    QColor panelTitle;       // A dock's title strip, and its inactive tabs.
    QColor panelTitleActive; // The current tab in a title strip.
    QColor raised;           // Cards inside a panel or dialog.
    QColor group;            // A grouped box's fill (Settings, the proposal), as macOS groups rows.
    QColor groupBorder;      // ...and its edge.
    QColor well;             // The deepest content area: lists, the project bin, preset list.
    QColor dialog;           // Dialog and settings window background.

    // Lines.
    QColor dockSeparator;      // Splitter handles between docked panels.
    QColor dockSeparatorHover; // A splitter handle under the pointer.
    QColor divider;            // The hairline between bars and panels (toolbar foot, title strip foot).
    QColor borderSubtle;       // Quiet edges: menus, menu separators.
    QColor border;             // Ordinary edges: pills, group boxes.
    QColor borderStrong;       // Edges that must read: hovered controls.

    // Text.
    QColor text;          // Body text and labels.
    QColor textStrong;    // Titles, the selected tab, emphasised values.
    QColor textSecondary; // Captions, hints, inactive tabs, and disclosure chevrons.
    QColor placeholderText; // A field's placeholder, which carries guidance: 4.5:1 on `field`.
    QColor textDisabled;  // Disabled controls.
    QColor tabInactiveText; // A dock's tabs that are not current, 4.5:1 on `panelTitle`.

    // Selection and pointer feedback.
    QColor selection;             // A chosen row in a list that has focus.
    QColor selectionText;         // Text on `selection`.
    QColor selectionInactive;     // A chosen row in a list without focus.
    QColor selectionInactiveText; // Text on `selectionInactive`.
    QColor textSelection;         // Selected text in a field.
    QColor hover;                 // Translucent wash under the pointer (rows, quiet buttons).
    QColor pressed;               // Translucent wash while a quiet button is held.

    // Fields: line edits, text edits, spin boxes, pop-ups' lists.
    QColor field;
    QColor fieldBorder;
    QColor fieldDisabled;
    QColor focusRing; // The border of the field being typed in (the accent).
    QColor focusHalo; // The soft ring outside a button focused from the keyboard (the accent at 35%).

    // An unchecked check box or radio button: no fill of its own and the
    // design's outline (white at 16%, laid on chrome).
    QColor checkFill;
    QColor checkBorder;

    // Push buttons (mockup-spec 0.5 .btn, secondary): the raised fill with a
    // line2 edge, flat (top and bottom edges the same).
    QColor button;
    QColor buttonEdge;
    QColor buttonTop;
    QColor buttonBottom;
    QColor buttonHover;
    QColor buttonPressed;
    QColor buttonText;
    QColor buttonDisabled;
    QColor buttonDisabledEdge;
    QColor buttonDisabledText;

    // Menus: the selected item is the accent, as in macOS.
    QColor menu;
    QColor menuBorder;
    QColor menuText;
    QColor menuSelection;
    QColor menuSelectionText;
    QColor menuSeparator;

    QColor tooltip;
    QColor tooltipBorder;
    QColor tooltipText;

    // Scroll bars: a thin handle on no track, as macOS's overlay scrollers. At
    // rest it reads at 3:1 on the panels, wells, fields and dialogs it sits
    // on; under the pointer or held, more.
    QColor scrollHandle;
    QColor scrollHandleHover;

    // Tab bars (Settings): plain text tabs, the chosen one on a raised chip.
    QColor tabText;
    QColor tabSelected;
    QColor tabSelectedBorder;
    QColor tabSelectedText;

    QColor sliderGroove;
    QColor sliderGrooveBorder;
    QColor sliderHandle;
    QColor sliderHandleBorder;

    // The Inspector's controls. No colours of their own: aliases of the
    // palette's. A fill means a value can be changed (plain text cannot).
    QColor inspectorField;   // Fields, pop-ups, wells and segmented tracks: `field`.
    QColor switchOn;         // A switch that is on: the neutral ink of the timeline's toggles, not the accent...
    QColor switchOnKnob;     // ...and its knob.
    QColor switchOff;        // A switch that is off, as the launcher's...
    QColor switchOffStroke;  // ...its edge...
    QColor knob;             // ...and its knob.
    QColor inspectorDetent;  // The tick at a slider's default.
    // A slider knob's edge, read against the groove and panel round it: away
    // from its default the more visible one (ink2), at it the quieter one
    // (track2).
    QColor sliderHandleChanged;
    QColor sliderHandleAtDefault;

    QColor statusBar;
    QColor statusText;
    QColor statusDivider; // Between status bar items.

    // Status. `*Surface` and `*Border` are a banner's fill and edge; `*Text`
    // reads on the surface at 4.5:1. The plain colour reads on `panel` and
    // `statusBar`.
    QColor error;
    QColor errorSurface;
    QColor errorBorder;
    QColor errorText;
    QColor warning;
    QColor warningSurface;
    QColor warningBorder;
    QColor warningText;  // The banner's body text...
    QColor warningTitle; // ...its bold first sentence...
    QColor warningGlyph; // ...and the triangle at its start: the only orange in it.
    QColor success;

    // Agent work (the AI bar, proposals, Ask Kerf, the agent's messages):
    // the accent family. It marks, and never fills a button other than the
    // default action.
    QColor assistant;        // Thin marks: proposal outlines, the AI bar's stripe, the Ask prompt's ring.
    QColor assistantSurface; // A bar or note's fill.
    QColor assistantBorder;  // A bar or note's edge, and its buttons' outlines.
    QColor assistantText;    // Text on `assistantSurface`, and the agent's name in the conversation.
    QColor assistantHover;   // A suggestion chip (an `assistantSurface` pill) under the pointer.

    // Conversation rows in the chat: they have no boxes, so their fills and
    // edges are the chat's own surface.
    QColor messageUser;
    QColor messageUserBorder;
    QColor messageAgent;
    QColor messageAgentBorder;
    QColor messageText;     // Message body text.
    QColor messageUserRole; // "You" over the user's messages, 4.5:1 on `messageUser`.

    // The chat panel laid out beside the picture, as a video site's comments
    // sit beside its player: a page lighter than the stage, rows with an
    // avatar disc, tonal chips, and an underlined composer.
    QColor chatSurface;
    QColor chatText;
    QColor chatSecondary;         // Times, counts, the placeholder: 4.5:1 on `chatSurface`.
    QColor chatChip;              // A suggestion chip: a tonal fill, no outline...
    QColor chatChipHover;         // ...under the pointer...
    QColor chatChipText;          // ...and its label.
    QColor chatAvatarUser;        // The user's avatar disc and its initials.
    QColor chatAvatarUserText;
    QColor chatAvatarKerf;        // Kerf's avatar disc (the accent) and its monogram.
    QColor chatAvatarKerfText;
    QColor chatExpander;          // "2 changes" under Kerf's row, in the accent as text.
    QColor chatComposerRule;      // The composer's underline, and in focus.
    QColor chatComposerRuleFocus;
    QColor chatScrollHandle;      // The list's scroll bar, shown only while the pointer is over the panel...
    QColor chatScrollHandleHover; // ...and under the pointer.
    // The round buttons between the picture and the chat.
    QColor railButton;
    QColor railButtonHover;
    QColor railButtonActive;      // The chat's toggle while it is open: tonal, not filled...
    QColor railGlyph;
    QColor railGlyphActive;       // ...its symbol in Kerf's ink.
    QColor railLabel;             // The count or label under a button.
    // A tonal pill (Reject, a secondary action): a fill and no outline.
    QColor pillTonal;
    QColor pillTonalText;

    // Icons drawn in code: toolbar and transport symbols, pop-up arrows.
    QColor glyph;
    QColor glyphDisabled;
    QColor toolSelected; // Behind a chosen tool or mode: a tonal fill, no outline (its symbol is the accent).

    // Timeline.
    QColor timelineBackground;      // Below the last track, so spare height never reads as a track.
    QColor ruler;                   // Ruler gradient, top...
    QColor rulerBottom;             // ...and bottom.
    QColor rulerTick;               // Major ticks.
    QColor rulerTickHalf;           // The half-way minor tick.
    QColor rulerTickMinor;          // Other minor ticks.
    QColor rulerText;               // Ruler timecodes.
    QColor rulerCorner;             // The box over the track headers showing the playhead time.
    QColor trackHeader;             // A track's header.
    QColor trackHeaderEdge;         // The 1px highlight along a header's top.
    QColor trackHeaderColumn;       // The header column behind and below the headers.
    QColor trackHeaderText;         // The track name.
    QColor trackHeaderTextDisabled; // The name of a disabled track.
    QColor trackHeaderDetail;       // The clip count under the name.
    QColor trackResizeGrip;         // The bar shown while a header's lower edge is dragged.
    QColor trackBadgeText;    // The V1/A1 number: plain secondary text, no fill.
    QColor trackToggle;       // Output, mute and lock: bare symbols, faint when off...
    QColor trackToggleOn;     // ...and a neutral ink when on, not the accent...
    QColor trackToggleHover;  // ...on a round wash only under the pointer.
    QColor trackLane;         // A track's lane.
    QColor laneGrid;          // Faint lines down the lanes at the ruler's major ticks, which show
                              // through a sound clip's translucent body.
    QColor trackLaneAlt;      // Every other lane, where alternation is wanted.
    QColor trackLaneLocked;   // The hatch over a locked lane.
    QColor trackDivider;      // The line under each lane and header.
    QColor trackBankDivider;  // The 2px line between the picture and sound banks.
    QColor headerPreview;     // The header's preview slot.
    QColor headerPreviewBorder;
    QColor addLane;           // The add-track row.
    QColor addLaneButton;
    QColor addLaneButtonBorder;
    QColor addLaneText;

    // Clips, by kind (see clipKind()).
    ClipStyle clipVideo;
    ClipStyle clipAudio;
    ClipStyle clipMusic;
    ClipStyle clipGenerator;
    ClipStyle clipCaption;
    QColor clipSelectedRing;      // The selected clip's 2px inner ring...
    QColor clipSelectedRingOuter; // ...and the 1px line outside it, so it reads on every clip hue
                                  // and lane: ink over the well, not a second loud hue.
    QColor clipTrim;         // The trim bracket on the edge under the pointer or being trimmed.
    QColor clipHighlight;    // The 1px highlight inside a clip's top edge.
    QColor clipFxBadge;      // The FX badge on an effected clip's header...
    QColor clipFxBadgeText;  // ...and its text.
    QColor filmstripSeam;    // Between filmstrip tiles.
    QColor waveCenter;       // The centre line of an audio clip's body.
    QColor waveformPeak;     // A waveform's peaks, and its RMS body over them: white on the clip's
    QColor waveformBody;     // card: the design's literal greys (mockup-spec 7.4).
    QColor playhead;         // The playhead: the accent, deepened where it would not read on the ruler
                             // and on its halo clearly more than `playheadIdle` does.
    QColor playheadHalo;     // The solid 1px line either side of the playhead's line, skimmed or not, so
                             // the line reads the same over pictures, clips and bare lanes.
    QColor playheadIdle;     // The playhead while the skimmer leads: a grey that reads at 3:1 on the
                             // ruler (its head) and on its halo (its line), and less than `playhead`.
    QColor skimmer;          // The skimmer line.
    QColor skimmerText;      // The corner time while skimming.
    QColor scrubTrack;       // A scrub bar's track (Source monitor).
    QColor inOutRange;       // The marked in-to-out range on a scrub bar or ruler.
    QColor snapLine;         // The line shown at the edit point a drag snaps to.
    QColor readout;          // The drag readout box...
    QColor readoutBorder;
    QColor readoutText;
    QColor dragShadow;       // Under the clip being dragged.

    // Marker colours, by the document's names; marker() looks one up. Each
    // reads at 3:1 on the whole ruler, top to foot.
    QColor markerRed;
    QColor markerOrange;
    QColor markerYellow;
    QColor markerGreen; // Also a marker without a colour.
    QColor markerCyan;
    QColor markerBlue;
    QColor markerPurple;
    QColor markerPink;
    QColor markerWhite; // A mid grey on the light ruler, where white would vanish.

    // A pending proposal drawn on the timeline: clips it adds, clips it
    // changes, and clips it removes.
    QColor proposalAdd;
    QColor proposalAddFill;
    QColor proposalChange;
    QColor proposalChangeFill;
    QColor proposalRemove;
    QColor proposalRemoveHatch;
    // The solid rule under an added or changed clip's dashes, so they read on
    // any picture, its own blues included.
    QColor proposalUnderlay;

    // Viewers (Program and Source monitors, the region picker): the stage is
    // the surround, the picture's canvas black.
    QColor viewerSurround;      // The stage around the picture card.
    QColor viewerCanvas;        // The canvas behind a picture that does not fill it, and when empty.
    QColor viewerText;          // The empty viewer's title...
    QColor viewerTextDim;       // ...and its hint; the Source monitor's message.
    QColor viewerCaption;       // The caption box over the picture...
    QColor viewerCaptionBorder; // ...its edge (the box itself: no line around it)...
    QColor viewerCaptionText;
    // The phone a portrait picture is shown in: a deep graphite in both
    // appearances, its 1px lighter edge, and its soft shadow on the stage
    // (the laptop's too).
    QColor deviceFrame;
    QColor deviceEdge;
    QColor deviceShadow;
    QColor deviceScreen;        // The screen's black above and below the picture (the laptop's too).
    QColor deviceInk;           // The status bar's time, signal and battery.
    // The laptop a wide picture is shown in, in the graphite ramp (Q-DEVICE
    // draws its shape)...
    QColor laptopShell;         // ...its lid's rim and its base's front...
    QColor laptopShellShade;    // ...the front's foot and the thumb-cut...
    QColor laptopDeck;          // ...the base's lit top face...
    QColor laptopHighlight;     // ...and the line along its top.
    QColor laptopBezel;         // The lid's black glass round the screen, and the notch.
    QColor laptopCamera;        // The camera in the notch.
    QColor laptopHinge;         // The hinge under the lid, and the base's feet.
    QColor safeZoneFill;        // Where a vertical-video app's interface usually covers the picture...
    QColor safeZoneLine;        // ...its outline...
    QColor safeZoneText;        // ...and its label...
    QColor safeZoneLabel;       // ...on a small dark chip.
    // The player's controls on the picture: white glyphs and time on a
    // gradient to black, the played part of the bar in the accent.
    QColor playerInk;           // Glyphs and the time, white at 90%.
    QColor playerInkDisabled;
    QColor playerShade;         // The gradient's darkest, under the controls.
    QColor playerTopShade;      // The top-left cluster's shade on a portrait card.
    QColor playerHover;         // A round wash under the pointer.
    QColor playerTrack;         // The bar's unplayed part...
    QColor playerPlayed;        // ...its played part (the accent)...
    QColor playerTipFill;       // ...and the time over the bar under the pointer.
    // Masks and regions over the picture: white over a dark rule; only the
    // handles are the accent.
    QColor maskOutline;         // A selected mask's outline...
    QColor maskOutlineIdle;     // ...one that is not selected...
    QColor maskShadow;          // ...and the dark rule under both.
    QColor maskFill;            // A region's wash.
    QColor maskHandleFill;      // A handle's fill (the accent)...
    QColor maskHandleBorder;    // ...and its edge.
    QColor maskTag;             // A mask's name tag...
    QColor maskTagSelected;     // ...the selected mask's...
    QColor maskText;            // ...and the name on either.

    // The project bin, the Effects panel, and media badges.
    QColor thumbnail;           // The letterbox bars behind a picture that does not fill its tile.
    QColor tileEmpty;           // A picture tile with no picture yet (or offline)...
    QColor tileEmptyBorder;
    QColor tileEmptyText;       // ...and its film or waveform symbol.
    QColor audioTile;           // A sound-only media tile...
    QColor audioTileGlyph;      // ...and its waveform.
    QColor cardPlaceholder;     // An effect card before its preview arrives...
    QColor cardPlaceholderText; // ...and the effect symbol on it.
    QColor badgeError;          // Offline...
    QColor badgeErrorText;
    QColor badgeInfo;           // Not downloaded...
    QColor badgeInfoText;
    QColor badgeNeutral;        // External.
    QColor badgeNeutralText;

    // The hardware finish, now flat on the graphite ramp until W2 replaces
    // its widgets: every face one fill (the "top" and "bottom" of a gradient
    // are the same), every highlight, lip, drop and inner shadow clear, lamps
    // in the status colours. editor_common's paint*() primitives draw every
    // part from these.
    QColor faceplateTop;       // The toolbar's plate, a gradient from its top...
    QColor faceplateBottom;    // ...to its foot...
    QColor faceplateHighlight; // ...the light caught along its top edge...
    QColor faceplateEdge;      // ...and the line under it.
    QColor keyTop;             // A raised key's face, a gradient from its top...
    QColor keyBottom;          // ...to its foot...
    QColor keyRing;            // ...the 1px ring round it...
    QColor keyDrop;            // ...the 1px drop under it...
    QColor keyHighlight;       // ...and the light along the inside of its top edge.
    QColor keyLatched;         // A key held in (latched): pressed below the plate...
    QColor keyLatchedShadow;   // ...in the shadow its rim throws across its top.
    QColor keyLed;             // The bar lit at the top of a key that is on (the accent).
    QColor bankWell;           // The well a bank of keys sits in...
    QColor bankShadow;         // ...the shadow inside its top edge...
    QColor bankLip;            // ...and the light along the lip below it.
    QColor wellFill;           // A recessed field (a number, a pop-up)...
    QColor wellShadow;         // ...the shadow inside its top edge (and an unlit lamp's)...
    QColor wellLip;            // ...and the light along the lip below it.
    QColor modulePlate;        // A framed module's plate...
    QColor moduleEdge;         // ...its 1px edge...
    QColor moduleHighlight;    // ...and the light inside its top edge.
    QColor engravedInk;        // An engraved label, 4.5:1 on `modulePlate` and the faceplate...
    QColor engravedHighlight;  // ...and the 1px catch of light (in dark, of shadow) below its strokes.
    QColor lampOff;            // A jewel lamp that is dark...
    QColor lampChanged;        // ...lit warn: a value moved from its default...
    QColor lampKerf;           // ...lit in Kerf's ink: Kerf's work, a proposal (the accent)...
    QColor lampOk;             // ...lit green: all is well.
    QColor glassTop;           // The display glass (a dark readout), a gradient from its top...
    QColor glassBottom;        // ...to its foot...
    QColor glassShadow;        // ...the shadow inside its top edge...
    QColor glassLip;           // ...and the lip of light below it.
    QColor glassText;          // The readout's figures, 4.5:1 on the glass...
    QColor glassLabel;         // ...and the small labels beside them, also 4.5:1.
    QColor knobRing;           // A bakelite knob's skirt...
    QColor knobFaceLight;      // ...its domed face, lit at the top left...
    QColor knobFaceDark;       // ...falling to its shaded edge...
    QColor knobTick;           // ...the scale's ticks round it...
    QColor knobPointer;        // ...and the ink line it points with.
    QColor vuBezelTop;         // An analogue VU meter's dark bezel, a gradient from its top...
    QColor vuBezelBottom;      // ...to its foot...
    QColor vuFace;             // ...its face (the well)...
    QColor vuInk;              // ...the scale and "VU" printed on it, 4.5:1 on the face...
    QColor vuRedZone;          // ...the red at the loud end of the scale...
    QColor vuNeedle;           // ...and the black needle and its pivot.

    // The style of a clip of `kind`.
    const ClipStyle &clip(ClipKind kind) const;
    // The marker colour the document calls `name` (green when unknown or empty).
    QColor marker(const QString &name) const;
};

// The editor's one theme, built from design_tokens.h.
EditorTheme makeEditorTheme();
// Deprecated, removed in W3: the argument is ignored.
EditorTheme makeEditorTheme(bool dark);

// Deprecated, removed in W3: always Dark.
ColorScheme editorColorScheme();
// Deprecated, removed in W3: every scheme, Light included, draws the dark
// theme, so this only applies it if it was never applied.
void setEditorColorScheme(ColorScheme scheme);
// The theme in force.
const EditorTheme &editorTheme();
// Set the application's palette and style sheet from the theme the first
// time (and again after a font or screen density change), repaint every
// widget, and emit EditorAppearance::changed(). Later calls with nothing
// changed do nothing.
void applyEditorAppearance();

// Tells widgets that cache colours or pixmaps to rebuild them. The theme no
// longer changes, so this fires once as it is first applied, then only when
// the application font or a screen's density changes. Deprecated as a theme
// signal, removed in W3; widgets may keep it for DPI and font changes.
class EditorAppearance final : public QObject {
    Q_OBJECT
public:
    static EditorAppearance *instance();

signals:
    void changed();

private:
    EditorAppearance() = default;
};

// The application palette for `theme`: the roles Fusion paints controls the
// style sheet leaves to it with (check boxes, radio buttons, plain text
// edits), and the window and accent, so native parts draw on the ramp too.
QPalette editorPalette(const EditorTheme &theme);

} // namespace kerf
