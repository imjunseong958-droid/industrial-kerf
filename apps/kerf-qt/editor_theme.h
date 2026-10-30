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
// The accent chosen in System Settings (Qt 6.6 and later), or macOS's blue
// for `dark` or light where the platform has none to give (offscreen test
// runs among them).
QColor systemAccent(bool dark);
// KERF_COLOR_SCHEME: Light or Dark when it names one, else System.
ColorScheme environmentColorScheme();
// Whether `scheme` draws dark: Dark, or System on a dark platform.
bool schemeIsDark(ColorScheme scheme);

// `over` laid on `under` at `amount` (0 to 1), alpha included.
QColor mixColor(const QColor &under, const QColor &over, qreal amount);
// WCAG contrast ratio of two opaque colours (1 to 21).
qreal contrastRatio(const QColor &a, const QColor &b);

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
// Dark keeps the finishing-room look (near-black wells, brightness steps
// instead of extra hues) with the system accent in place of amber; light is
// macOS's light window family. The viewer's surround is the same neutral
// dark grey in both, so pictures are judged against the same field.
struct EditorTheme {
    bool dark = true;

    // Accent: the system accent (focus, the default button, the playhead,
    // checked tools, progress).
    QColor accent;
    QColor accentTop;         // Top of an accent bezel's gradient.
    QColor accentPressed;     // An accent button held down.
    QColor accentText;        // The accent as text or a thin line on `panel`, 4.5:1.
    QColor accentTint;        // A quiet accent wash on `panel`: a checked tool, an info badge.
    QColor onAccent;          // Text and glyphs on `accent`.
    QColor onAccentSecondary; // Secondary text on `accent`.

    // Surfaces, from the back of the window forwards.
    QColor window;           // Behind everything: the main window, the dock container.
    QColor chrome;           // Toolbar, menu bar, and tool palette.
    QColor panel;            // A docked panel's body: monitors' frames, timeline, inspector, transport.
    QColor panelTitle;       // A dock's title strip, and its inactive tabs.
    QColor panelTitleActive; // The current tab in a title strip.
    QColor raised;           // Group boxes and cards inside a panel or dialog.
    QColor well;             // The deepest content area: lists, the project bin, preset list.
    QColor dialog;           // Dialog and settings window background.

    // Lines.
    QColor dockSeparator;      // Splitter handles between docked panels.
    QColor dockSeparatorHover; // A splitter handle under the pointer.
    QColor divider;            // The hairline between bars and panels (toolbar foot, title strip foot).
    QColor borderSubtle;       // Quiet edges: menus, menu separators.
    QColor border;             // Ordinary edges: pills, toolbar separators, group boxes.
    QColor borderStrong;       // Edges that must read: hovered controls, checkbox frames.

    // Text.
    QColor text;          // Body text and labels.
    QColor textStrong;    // Titles, the selected tab, emphasised values.
    QColor textSecondary; // Captions, hints, inactive tabs.
    QColor textTertiary;  // Least important: durations, counts, placeholders.
    QColor textDisabled;  // Disabled controls.

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

    QColor scrollTrack;
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
    QColor warningText;
    QColor success;

    // Agent work (the AI bar, proposals, Ask Kerf, the agent's messages):
    // the system purple, so it never reads as the user's own selection.
    QColor assistant;        // Thin marks: proposal outlines, the Ask prompt's ring.
    QColor assistantStrong;  // A filled button (Accept); `onAssistant` reads on it at 4.5:1.
    QColor onAssistant;
    QColor assistantSurface; // A bar or note's fill.
    QColor assistantBorder;  // A bar or note's edge, and its buttons' outlines.
    QColor assistantText;    // Text on `assistantSurface`, and the agent's name in the conversation.

    // Conversation bubbles in the assistant panel.
    QColor messageUser;
    QColor messageUserBorder;
    QColor messageAgent;
    QColor messageAgentBorder;
    QColor messageText;     // Message body text.
    QColor messageUserRole; // "YOU" over the user's messages.

    // Icons drawn in code: toolbar and transport symbols, pop-up arrows.
    QColor glyph;
    QColor glyphDisabled;

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
    QColor trackBadge;              // The V1/A1 badge.
    QColor trackBadgeBorder;
    QColor trackBadgeText;
    QColor trackToggle;       // Output, mute, and lock toggles when off...
    QColor trackToggleBorder;
    QColor trackToggleGlyph;
    QColor trackToggleOn;     // ...and on (the accent).
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
    QColor clipSelectedRing; // The 2px ring round the selected clip. Not the accent: the accent
                             // is usually blue, the picture clips' own hue.
    QColor clipTrim;         // The trim bracket on the edge under the pointer or being trimmed.
    QColor clipHighlight;    // The 1px highlight inside a clip's top edge.
    QColor clipFxBadge;      // The FX badge on an effected clip's header...
    QColor clipFxBadgeText;  // ...and its text.
    QColor filmstripSeam;    // Between filmstrip tiles.
    QColor waveCenter;       // The centre line of an audio clip's body.
    QColor playhead;         // The playhead and its corner time (the accent).
    QColor playheadHalo;     // The 3px halo that keeps the playhead readable over pictures.
    QColor playheadIdle;     // The playhead while the skimmer leads.
    QColor skimmer;          // The skimmer line.
    QColor skimmerText;      // The corner time while skimming.
    QColor scrubTrack;       // A scrub bar's track (Source monitor).
    QColor inOutRange;       // The marked in-to-out range on a scrub bar or ruler.
    QColor snapLine;         // The line shown at the edit point a drag snaps to.
    QColor readout;          // The drag readout box...
    QColor readoutBorder;
    QColor readoutText;

    // Marker colours, by the document's names; marker() looks one up.
    QColor markerRed;
    QColor markerOrange;
    QColor markerYellow;
    QColor markerGreen; // Also a marker without a colour.
    QColor markerCyan;
    QColor markerBlue;
    QColor markerPurple;
    QColor markerPink;
    QColor markerWhite; // A light grey on the light ruler, where white would vanish.

    // A pending proposal drawn on the timeline: clips it adds, clips it
    // changes, and clips it removes.
    QColor proposalAdd;
    QColor proposalAddFill;
    QColor proposalChange;
    QColor proposalChangeFill;
    QColor proposalRemove;
    QColor proposalRemoveHatch;

    // Viewers (Program and Source monitors, the region picker). The same in
    // both appearances: pictures are judged against a neutral dark field.
    QColor viewerSurround;      // Around the picture.
    QColor viewerCanvas;        // The canvas behind a picture that does not fill it, and when empty.
    QColor viewerText;          // The empty viewer's title...
    QColor viewerTextDim;       // ...and its hint; the Source monitor's message.
    QColor viewerGlyph;         // The empty viewer's play symbol...
    QColor viewerGlyphRing;     // ...its ring...
    QColor viewerGlyphFill;     // ...and the ring's fill.
    QColor viewerCaption;       // The caption box over the picture...
    QColor viewerCaptionBorder;
    QColor viewerCaptionText;
    QColor maskOutline;         // Mask outlines and handles over the picture...
    QColor maskOutlineIdle;     // ...on a mask that is not selected...
    QColor maskText;            // ...a mask's name (white: the accent's hue can be the picture's)...
    QColor maskFill;            // ...a region's wash...
    QColor maskHandleFill;      // ...the inside of a handle that is not filled...
    QColor maskShadow;          // ...and the shadow that keeps outlines readable.

    // The project bin, the Effects panel, and media badges.
    QColor thumbnail;           // Behind a letterboxed thumbnail.
    QColor thumbnailText;       // "MEDIA" or "AUDIO" on a tile with no picture.
    QColor audioTile;           // A sound-only media tile...
    QColor audioTileGlyph;      // ...and its note.
    QColor cardPlaceholder;     // An effect card before its preview arrives...
    QColor cardPlaceholderText;
    QColor badgeError;          // OFFLINE...
    QColor badgeErrorText;
    QColor badgeInfo;           // NOT DOWNLOADED...
    QColor badgeInfoText;
    QColor badgeNeutral;        // EXTERNAL.
    QColor badgeNeutralText;

    // The style of a clip of `kind`.
    const ClipStyle &clip(ClipKind kind) const;
    // The marker colour the document calls `name` (green when unknown or empty).
    QColor marker(const QString &name) const;
};

// The theme for an appearance and accent.
EditorTheme makeEditorTheme(bool dark, const QColor &accent);

// The scheme the editor draws in: KERF_COLOR_SCHEME's, else System, until
// setEditorColorScheme() picks another.
ColorScheme editorColorScheme();
// Draw the editor (and launchers made after this) in `scheme`, now.
void setEditorColorScheme(ColorScheme scheme);
// The theme in force. Before the first applyEditorAppearance() it is the
// one the current scheme and accent call for.
const EditorTheme &editorTheme();
// Compute the theme for the current scheme and system accent and, when it
// changed (or was never applied), set the application's palette and style
// sheet, repaint every widget, and emit EditorAppearance::changed().
void applyEditorAppearance();
// Re-apply as the platform's appearance or accent changes: on
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

// A white tick for a checked check box on the accent, as an image file the
// style sheet can name (written once per run); empty if it cannot be written.
QString checkMarkImagePath();

// The application palette for `theme`: the roles Fusion paints controls the
// style sheet leaves to it with (check boxes, radio buttons, plain text
// edits). It leaves Window and Accent to the platform, so systemIsDark()'s
// palette fallback and systemAccent() keep reading the system's.
QPalette editorPalette(const EditorTheme &theme);

} // namespace kerf
