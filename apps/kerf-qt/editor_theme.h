#pragma once

#include <QColor>
#include <QJsonObject>
#include <QObject>
#include <QPalette>
#include <QString>

namespace kerf {

// Light or dark. System follows the platform's appearance as it changes;
// KERF_COLOR_SCHEME=light or dark picks one for the whole run.
enum class ColorScheme { System, Light, Dark };

// Appearance detection, shared by the launcher and the editor.
//
// Whether the platform is in its dark appearance: Qt's colour scheme where
// the platform reports one (Qt 6.5 and later), else the lightness of the
// system palette's window colour.
bool systemIsDark();
// Kerf's own accent, one per appearance: an aubergine ink on cream and its
// mauve on the warm dark. Kerf does not follow the system accent.
QColor kerfAccent(bool dark);
// KERF_COLOR_SCHEME: Light or Dark when it names one, else System.
ColorScheme environmentColorScheme();
// Whether `scheme` draws dark: Dark, or System on a dark platform.
bool schemeIsDark(ColorScheme scheme);

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
// 4.5:1 in both appearances.
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

// Every colour the editor draws, by role. The application style sheet is
// generated from these, and painted widgets read them at paint time from
// editorTheme(); a widget that caches colours or pixmaps rebuilds them on
// EditorAppearance::changed().
//
// Both share the project launcher's palette, so opening a project keeps the
// same look: antique cream paper in light, warm espresso in dark, never
// pure white or black. Kerf's own purple marks Kerf, focus, the playhead
// and the default action. The viewer's stage follows the appearance; the
// picture card is warm black in both.
struct EditorTheme {
    bool dark = true;

    // Accent: Kerf's own (focus, the default button, the playhead, checked
    // tools, progress). It does not follow the system accent.
    QColor accent;
    QColor accentTop;         // Top of an accent bezel's gradient.
    QColor accentPressed;     // An accent button held down.
    QColor accentText;        // The accent as text or a thin line, 4.5:1 on every surface it is drawn on:
                              // `panel` (the transports' timecodes), `rulerCorner` (the timeline's
                              // corner time), and `statusBar` (the sync chip).
    QColor accentTint;        // A quiet accent wash on `panel`: a checked tool, an info badge, the chosen
                              // mask tool and the selected mask. `accentText` reads on it at 3:1.
    QColor accentTintPressed; // `accentTint` darkened while held, as a bezel darkens; `accentText`
                              // still reads on it at 3:1.
    QColor onAccent;          // Text and glyphs on `accent`, 4.5:1: cream on the aubergine, dark ink on
                              // the light mauve of the dark appearance.
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

    // An unchecked check box or radio button: a fill and an edge that reads
    // at 3:1 on the panels and dialogs it sits on.
    QColor checkFill;
    QColor checkBorder;

    // Push buttons: a fill with a lighter top edge and a darker bottom one,
    // as macOS draws its bezels (the launcher's buttons).
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
    // Kerf's purple is the accent itself. It marks, and never fills a
    // button other than the default action.
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
    QColor trackBadge;              // The V1/A1 badge: a rounded fill, no edge.
    QColor trackBadgeText;
    QColor trackToggle;       // Output, mute, and lock toggles when off...
    QColor trackToggleGlyph;
    QColor trackToggleOn;     // ...and on: a neutral ink plate, not the accent.
    QColor trackToggleOnGlyph;
    QColor trackLane;         // A track's lane.
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
                                  // and lane: cream over ink, not a second loud hue.
    QColor clipTrim;         // The trim bracket on the edge under the pointer or being trimmed.
    QColor clipHighlight;    // The 1px highlight inside a clip's top edge.
    QColor clipFxBadge;      // The FX badge on an effected clip's header...
    QColor clipFxBadgeText;  // ...and its text.
    QColor filmstripSeam;    // Between filmstrip tiles.
    QColor waveCenter;       // The centre line of an audio clip's body.
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
    // any picture, its own purples and pinks included.
    QColor proposalUnderlay;

    // Viewers (Program and Source monitors, the region picker). The stage
    // follows the appearance; the picture card and what is drawn on it are
    // the same warm black in both.
    QColor viewerSurround;      // The stage around the picture card.
    QColor viewerCanvas;        // The canvas behind a picture that does not fill it, and when empty.
    QColor viewerText;          // The empty viewer's title...
    QColor viewerTextDim;       // ...and its hint; the Source monitor's message.
    QColor viewerGlyph;         // The empty viewer's play symbol...
    QColor viewerGlyphRing;     // ...its ring...
    QColor viewerGlyphFill;     // ...and the ring's fill.
    QColor viewerCaption;       // The caption box over the picture...
    QColor viewerCaptionBorder; // ...its edge (the box itself: no line around it)...
    QColor viewerCaptionText;
    // The phone a portrait picture is shown in: a deep graphite in both
    // appearances, its 1px lighter edge, and its soft shadow on the stage.
    QColor deviceFrame;
    QColor deviceEdge;
    QColor deviceShadow;
    QColor deviceScreen;        // The screen's black above and below the picture.
    QColor deviceInk;           // The status bar's time, signal and battery.
    QColor safeZoneFill;        // Where a vertical-video app's interface usually covers the picture...
    QColor safeZoneLine;        // ...its outline...
    QColor safeZoneText;        // ...and its label.
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
    // Masks and regions over the picture: cream over a dark rule, never the
    // accent, whose hue can be the picture's own.
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

    // The style of a clip of `kind`.
    const ClipStyle &clip(ClipKind kind) const;
    // The marker colour the document calls `name` (green when unknown or empty).
    QColor marker(const QString &name) const;
};

// The theme for an appearance.
EditorTheme makeEditorTheme(bool dark);

// The scheme the editor draws in: KERF_COLOR_SCHEME's, else System, until
// setEditorColorScheme() picks another.
ColorScheme editorColorScheme();
// Draw the editor (and launchers made after this) in `scheme`, now.
void setEditorColorScheme(ColorScheme scheme);
// The theme in force. Before the first applyEditorAppearance() it is the
// one the current scheme calls for.
const EditorTheme &editorTheme();
// Compute the theme for the current scheme and, when it
// changed (or was never applied), set the application's palette and style
// sheet, repaint every widget, and emit EditorAppearance::changed().
void applyEditorAppearance();
// Re-apply as the platform's appearance changes: on
// QStyleHints::colorSchemeChanged and on application palette changes.
void followSystemAppearance();

// Tells widgets that cache colours or pixmaps that the theme changed.
// Anything painted from editorTheme() at paint time is repainted already.
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
// edits). It leaves Window to the platform, so systemIsDark()'s palette
// fallback keeps reading the system's.
QPalette editorPalette(const EditorTheme &theme);

} // namespace kerf
