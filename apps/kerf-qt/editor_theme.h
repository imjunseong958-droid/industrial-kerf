#pragma once

#include <QColor>
#include <QJsonObject>
#include <QObject>
#include <QPalette>
#include <QString>

namespace kerf {

// The editor is dark only: one graphite ramp and one blue accent, from
// design_tokens.h.

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
    // The design's tokens themselves (design_tokens.h 0.2), so the style
    // sheet and painted widgets can name the ramp directly.
    QColor card;        // Comments panel, Source viewer, menus, tooltips, clips.
    QColor on;          // Selected or on fill, avatars, the sliding segment.
    QColor hover2;      // Hover over a `raised` surface.
    QColor ink;         // Primary text, slider knob.
    QColor ink2;        // Secondary text, icon default.
    QColor ink3;        // Tertiary text, meta: 4.66:1 on chrome, under 4.5:1 on card and raised.
    QColor track;       // Slider and progress track, minor ticks.
    QColor track2;      // Slider fill, major ticks, played bar.
    QColor line;        // Hairlines (white 8%).
    QColor line2;       // Control outlines (white 14%).
    QColor lineh;       // Hover outline (white 22%).
    QColor accentHover; // The primary button under the pointer.
    QColor accentInk;   // Timecode links, accepted text, "Ask Kerf" actions.
    QColor pos;         // Pass, diff additions, connected.

    // Accent: Kerf's own (focus, the default button, the playhead, checked
    // tools, progress). It does not follow the system accent.
    QColor accent;
    QColor accentPressed;     // An accent button held down.
    QColor accentText;        // The accent as text or a thin line, 4.5:1 on every surface it is drawn on:
                              // `panel` (the transports' timecodes), and `rulerCorner` (the timeline's
                              // corner time).
    QColor accentTint;        // A quiet accent wash on `panel`: a checked tool, an info badge, the chosen
                              // mask tool and the selected mask. `accentText` reads on it at 3:1.
    QColor accentTintPressed; // `accentTint` darkened while held, as a bezel darkens; `accentText`
                              // still reads on it at 3:1.
    QColor onAccent;          // Text and glyphs on `accent`, 4.5:1: white.
    QColor onAccentSecondary; // Secondary text on `selection`: `selectionText` held back only as far as
                              // it still reads at 4.5:1.

    // Surfaces, from the back of the window forwards.
    QColor window;           // Behind everything: the main window.
    QColor chrome;           // Toolbar, menu bar, and tool palette.
    QColor panel;            // A panel's body: the timeline, the sheets.
    QColor panelTitle;       // A dock's title strip, and its inactive tabs.
    QColor panelTitleActive; // The current tab in a title strip.
    QColor raised;           // Cards inside a panel or dialog.
    QColor group;            // A grouped box's fill (Settings, the proposal), as macOS groups rows.
    QColor groupBorder;      // ...and its edge.
    QColor well;             // The deepest content area: lists, the project bin, preset list.
    QColor dialog;           // Dialog and settings window background.

    // Lines.
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

    // Fields: line edits, text edits, spin boxes, pop-ups' lists.
    QColor field;
    QColor fieldBorder;
    QColor fieldDisabled;
    QColor focusRing; // The border of the field being typed in (the accent).
    QColor focusHalo; // The soft ring outside a button focused from the keyboard (the accent at 35%).

    // An unchecked check box or radio button: no fill of its own and the
    // design's outline (white at 16%, laid on chrome).
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
    QColor sliderHandle;

    // The Inspector's controls. No colours of their own: aliases of the
    // palette's. A fill means a value can be changed (plain text cannot).
    QColor inspectorField;   // Fields, pop-ups, wells and segmented tracks: `field`.
    QColor switchOff;        // A switch that is off, as the launcher's...
    QColor switchOffStroke;  // ...its edge...
    QColor knob;             // ...and its knob.


    // Status. `*Surface` and `*Border` are a banner's fill and edge; `*Text`
    // reads on the surface at 4.5:1. The plain colour reads on `panel`.
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
    QColor assistantText;    // Text on `assistantSurface`, and the agent's name in the conversation.
    QColor assistantHover;   // A suggestion chip (an `assistantSurface` pill) under the pointer.

    // Conversation rows in the chat: they have no boxes, so their fills and
    // edges are the chat's own surface.
    QColor messageUser;
    QColor messageAgent;
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
    QColor chatComposerRuleFocus;
    QColor chatScrollHandle;      // The list's scroll bar, shown only while the pointer is over the panel...
    QColor chatScrollHandleHover; // ...and under the pointer.
    // The round buttons between the picture and the chat.
    // A tonal pill (Reject, a secondary action): a fill and no outline.
    QColor pillTonal;
    QColor pillTonalText;

    // Icons drawn in code: toolbar and transport symbols, pop-up arrows.
    QColor glyph;
    QColor glyphDisabled;
    QColor toolSelected; // Behind a chosen tool or mode: a tonal fill, no outline (its symbol is the accent).

    // Timeline: the ruler, which the playhead and markers read on.
    QColor ruler;                   // Ruler gradient, top...
    QColor rulerBottom;             // ...and bottom.
    QColor rulerCorner;             // The box over the track headers showing the playhead time.

    // Clips, by kind (see clipKind()).
    ClipStyle clipVideo;
    ClipStyle clipAudio;
    ClipStyle clipMusic;
    ClipStyle clipGenerator;
    ClipStyle clipCaption;
    QColor playhead;         // The playhead: the accent, deepened where it would not read on the ruler
                             // and on its halo clearly more than `playheadIdle` does.
    QColor playheadHalo;     // The solid 1px line either side of the playhead's line, skimmed or not, so
                             // the line reads the same over pictures, clips and bare lanes.
    QColor playheadIdle;     // The playhead while the skimmer leads: a grey that reads at 3:1 on the
                             // ruler (its head) and on its halo (its line), and less than `playhead`.
    QColor inOutRange;       // The marked in-to-out range on a scrub bar or ruler.

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

    // A pending proposal's changed clip (the timeline's own drawing is
    // design_tokens.h's).
    QColor proposalChange;

    // Viewers (the picture, the Source monitor, the region picker): the picture's
    // canvas black.
    QColor viewerSurround;      // Around the region picker's frame.
    QColor viewerCanvas;        // The canvas behind a picture that does not fill it, and when empty.
    QColor viewerText;          // The empty viewer's title...
    QColor viewerTextDim;       // ...and its hint; the Source monitor's message.
    QColor viewerCaption;       // The caption box over the picture...
    QColor viewerCaptionBorder; // ...its edge (the box itself: no line around it)...
    // The Source viewer's black above and below its picture.
    QColor deviceScreen;
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
    QColor tileEmpty;           // A picture tile with no picture yet (or offline)...
    QColor tileEmptyText;       // ...and its film or waveform symbol.
    QColor cardPlaceholder;     // An effect card before its preview arrives...
    QColor cardPlaceholderText; // ...and the effect symbol on it.
    QColor badgeError;          // Offline...
    QColor badgeErrorText;
    QColor badgeInfo;           // Not downloaded...
    QColor badgeInfoText;

    // The style of a clip of `kind`.
    const ClipStyle &clip(ClipKind kind) const;
    // The marker colour the document calls `name` (green when unknown or empty).
    QColor marker(const QString &name) const;
};

// The editor's one theme, built from design_tokens.h.
EditorTheme makeEditorTheme();

// The theme in force.
const EditorTheme &editorTheme();
// Set the application's palette and style sheet from the theme the first
// time (and again after a font or screen density change), repaint every
// widget, and emit EditorAppearance::changed(). Later calls with nothing
// changed do nothing.
void applyEditorAppearance();

// Tells widgets that cache colours or pixmaps to rebuild them. The theme
// never changes, so this fires once as it is first applied, then only when
// the application font or a screen's density changes.
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
