# Motion UI

How the editor moves, and what to do when a feature needs to move too. Read this
before adding or changing an animation.

Values live in `theme::motion` (`src/theme.rs`). This file names them rather than
repeating their numbers, so changing a value does not make it wrong.

## Principles

1. **Things become the next thing.** Nothing opens like a tab. What is on screen turns
   into what comes next: the conversation gives way as the picture comes in.
2. **One room.** The conversation and the stage share a surface with no line between
   them (`theme::surround`); only the sidebar and the panes the picture sits among
   are a step off it.
3. **Motion never decides anything.** State changes at once and motion follows it, so
   a stopped animation leaves nothing wrong. A region's place is a `Tween` whose
   *target* is the state; what is drawn is where the tween has got to.
4. **Reduce Motion means simply there.** Every scene has to work with no motion.
   `KERF_REDUCE_MOTION=1`, or Settings ▸ Appearance, sets `motion::reduced()`;
   `motion::duration` and `motion::animation` then take no time, and what repeats
   (lights, stripes) holds still.
5. **Never wait on a blank.** If a picture is coming, something stands in for it
   (the stage says "Connecting to kerfd…" or the project's name on the screen).
6. **Nothing moves the picture's colours.** The room around the picture stays neutral,
   whatever it shows. Motion around it uses no colour taken from it.

## Scenes

| Scene | When | What moves | Code | Test |
|---|---|---|---|---|
| Sidebar | It opens or closes (its button, the toolbar's when it is away, `view.sidebar`) | It slides in from the window's edge over `PANEL`, the conversation's left edge against its own; the toolbar's title moves with it. Dragged by its edge, it is where the pointer is, with no motion, and a line shows on the edge | `Workspace::set_sidebar`, `Workspace::on_drag`, `Workspace::render` | `prefs::choices_round_trip_through_the_file` (the width kept) |
| Panel | The panel button, `view.picture`/`view.timeline`, or a cut arriving | The picture and the timeline come in from the window's right edge over `PANEL`, their left edge against the conversation's right as it narrows, and go out the same way. One of the two alone grows to the panel's height as the other folds away | `Workspace::set_panel`, `Workspace::toggle_pane`, `Workspace::panel` | none |
| Picture arrives | A project opens with clips on its timeline, or a proposal brings its own | The panel comes in over `STAGE`, the conversation giving way to its rail | `Workspace::session_changed` | none |
| Media band | Media opens or closes (`window.project`, the toolbar's grid) | The band comes down out from under the picture over `PANEL` as the timeline's top moves down for it; closing, it goes back up under it. With the panel closed, the panel comes in with the band in it | `Workspace::set_media`, `Workspace::panel` | none |
| Media tiles | Media shows in the band | Each tile fades in where it is over `POP` | `MediaBand::tile` | none |
| Making a shot | A media is being made from its steps | Amber stripes (`activity::gpu_stripe`, `STRIPE_SPACE` apart) move across its tile once each `STRIPE_CYCLE`, with "Making take 2 · 0:41"; drawn again only while something is made; still under Reduce Motion. Its clips on the timeline, and the takes card in its graph, have them too | `MediaBand::tile`, `media::paint_stripes`, `TimelineView`, `ShotGraph` | `state::a_making_is_shown_until_it_is_made_or_stopped` (the state) |
| Sidebar activity | A conversation is Working | A band of light passes over its title once each `TITLE_SHIMMER`, taking `TITLE_CROSS` of it, then rests; under Reduce Motion the title is simply in ink. Done while you looked elsewhere: a dot at the row's end, there at once | `SidebarView::row`, `motion::shimmer_at` | `motion::the_light_over_a_title_rests_after_crossing`, `state::a_turn_that_ends_in_a_conversation_not_looked_at_leaves_a_dot` |
| Archived unfolds | Archived · N is pressed | The archived conversations fade in where they are, one after another (`APPEAR`, `UNFOLD_STAGGER` apart); folding is at once | `SidebarView::row` | none |
| Row menu | A conversation's ⋯ is pressed | Its menu fades in over `FADE`; a press outside closes it at once | `SidebarView::row_menu` | none |
| Settings | ⌘, / Ctrl+, the sidebar's gear | Over the window: the window dims at once (`SCRIM`) and the panel is in its middle; its body comes up `PAGE_RISE` into place over `PAGE`, and a page chosen fades in over `APPEAR`. Esc or its close gives the window back at once | `Workspace::show_settings`, `Workspace::modal`, `SettingsView` | none |
| Command palette | ⌘⇧P / Ctrl+Shift+P | Over the window as Settings is: the window dims at once and the palette rises into its middle over `PAGE`; the chosen row moves with the arrows at once. Esc, or running a command, gives the window back at once | `Workspace::open_modal`, `CommandPalette` | `shortcuts::the_palette_finds_a_command_by_any_words_of_it` (what it finds) |
| Finish | The toolbar's Finish, or Run QC | Over the window as Settings is: the window dims at once and the panel rises into its middle over `PAGE`; a section chosen fades in over `APPEAR`. While a check, a measurement or a render runs its bar fills in amber (`activity::gpu`), and the light round the picture says so too | `FinishView` | none |
| Sync chip | The toolbar's branch chip is pressed; a sync runs | Its menu fades in under it over `FADE` and closes at once on a press outside or Esc; a `…` item asks for its words in place. While a sync runs, a spinner turns once each `WORK_PULSE` beside what it is doing, still under Reduce Motion | `Collab` | none |
| Inspector | A clip is chosen | The inspector stands beside the picture at once, and the picture's room narrows for it; a value dragged moves its slider with the pointer, and the picture changes only when kerfd's new snapshot comes | `Workspace::panel`, `InspectorView` | none |
| Add Effect | The inspector's Add Effect… | The list fades in over the inspector over `FADE`; Esc, its close or choosing one takes it away at once | `InspectorView::adding_list` | none |
| Make cards | Kerf asks for a making in the conversation | Its card fades in where it is, rising `APPEAR_RISE` over `APPEAR`, as a message does; then it follows the making (clock, the step's line, Stop) and says when the take is made, at once | `Makings::card` | `state::a_making_is_shown_until_it_is_made_or_stopped` (the state) |
| A shot's graph | Open Graph on a media, or the tile's menu | Over the window as Settings is: the window dims at once and the panel rises into its middle over `PAGE`. Nothing in the graph moves by itself: a card goes where it is dragged, the canvas where it is panned; its head's menus fade in over `FADE` | `ShotGraph` | none |
| Media tile menu | A tile's ⋯ or a right-click on it | Its menu fades in over `FADE`; a press outside closes it at once | `MediaBand::menu` | none |
| Switch | A switch is pressed | Its knob slides to the other end and its track takes or loses the accent, over `SEGMENT` | `ui::switch` | none |
| Export | The toolbar's Export | Its popover fades in under the button over `FADE`; Esc or a press outside closes it at once | `Workspace::toggle_export`, `ExportView` | none |
| Messages arrive | You send, or the bridge keeps a turn | Your request and Kerf's answer fade in where they are, rising `APPEAR_RISE` over `APPEAR`; the list keeps to the latest unless you scrolled away | `ChatView::turn`, `ChatView::render` | `state::a_turn_shows_in_its_conversation_and_its_error_is_said_once` (the end state) |
| Working | During a turn | Under your request, what Kerf says it is doing with a band of light passing over it once each `WORK_SHIMMER` (still under Reduce Motion, its clock then going on once each `CLOCK_FRAME`), the time so far and its last steps on a hairline. Done, it folds to "Worked for …" at once | `ChatView::working` | `state::a_working_turn_keeps_its_steps_and_streams_its_answer` |
| Stage at work | Kerf works, a render runs, or a model makes a shot | A light runs round just outside the picture's edge (`EDGE_GAP` out, `EDGE_WIDTH` wide, a tail of `EDGE_TAIL`), once each `EDGE_TURN`, blue for Kerf and amber for the computer, amber winning; the rest of the ring at `EDGE_REST`. The chip at the picture's top left says what runs. Both fade in, and out when it ends, over `EDGE_FADE`; under Reduce Motion the ring is still | `StageView::render`, `paint_edge_light` | `state::a_render_job_is_followed_from_queued_to_done` (what the chip says) |
| Timeline drag | A clip or its edge is dragged | The clip is where the pointer is, with no motion, snapping within a few points to clip edges, the playhead and markers (a line shows where); its old place is a dashed outline. Released, it stays where it was dropped until kerfd's answer redraws the cut, and goes back at once if kerfd refuses the edit | `TimelineView` | none |
| Timeline follows | The playhead leaves the view while playing or jumping | The view pages to it at once; nothing slides | `TimelineView` | none |
| Kerf at work | During a turn | Kerf's cursor on the timeline at the time it looks at, the clips it names ringed in its blue | `TimelineView` | `state::a_working_turn_keeps_its_steps_and_streams_its_answer` (the state) |
| Notices | An error, or a render finished | The note fades in over `APPEAR` at the conversation's foot, and goes after a while | `Workspace::notices`, `EditorState::expire_notices` | `state::a_turn_shows_in_its_conversation_and_its_error_is_said_once` |
| Empty conversation | Nothing has been said yet | The question and its suggestions rise `PAGE_RISE` into place over `RISE` | `ChatView::empty` | none |

"none" means it has only been checked in a real window. Motion can only be judged
there, or from screenshots taken while it runs (see `.claude/skills/run-kerf-editor`).

## What to reuse

Look here before writing a new animation.

| You want | Use |
|---|---|
| A region to move to a new place or size | A `motion::Tween` held by the view: `go(target, scene)` when the state changes, `sample(window)` in `render` (it asks for the next frame while it runs) |
| Something to fade or rise in where it is | GPUI's `with_animation(id, motion::animation(scene), …)` with `opacity` and a margin; the id must stay the same for as long as the thing is the same thing, or it starts again every frame |
| Something to repeat while something works | `motion::phase(since, period)`, asking `window.request_animation_frame()` only while it shows |
| A light passing over words | `motion::shimmer_at` with a gradient over the words (`SidebarView::row`, `ChatView::working`) |
| What one goes into | `Workspace::modal`: the window dims at once and the body rises |
| A duration, stagger or distance | A name in `theme::motion`, added there if none fits |
| The curve | `motion::ease`, cubic-bezier(.22, 1, .36, 1) |

## Adding or changing a scene

1. Decide what on screen becomes the new thing. If nothing does, ask whether it should
   exist as a separate thing at all.
2. Reuse what is above. Put new values in `theme::motion`, never in the code.
3. Decide what shows while anything slow is happening.
4. Check it in **16:9 and 9:16**, and from an empty project as well as one with a cut.
5. Check it with Reduce Motion on, and test the end state without motion.
6. Keep a frame's work small: see **What a frame costs**.
7. Update the **Scenes** table here in the same change.

## What a frame costs

GPUI draws every frame on the GPU from what the views lay out, so the cost is in
laying out, not in pixels:

- **Nothing holding still draws frames.** Ask for the next frame only while something
  moves: a `Tween` running, a light round the picture, stripes over what is made.
- **One motion per property.** A new `go` takes over from where the last got to.
- **Draw what moves with `canvas`, not with elements**, when there are many pieces:
  the light round the picture and the stripes are painted paths, not a div each.
- **A picture is uploaded once.** Each frame playback draws becomes one `RenderImage`,
  and the one before it is dropped from the atlas (`Session::show_frame`), so playing
  does not grow memory.
