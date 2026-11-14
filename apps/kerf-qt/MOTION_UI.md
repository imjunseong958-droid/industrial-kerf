# Motion UI

How the editor moves, and what to do when a feature needs to move too. Read this
before adding or changing an animation. The approved reference is the
[motion mock](https://claude.ai/artifact/K9K3zqcSZBbS4YnToYUzu1).

Values live in `tokens::motion` (`design_tokens.h`). This file names them rather than
repeating their numbers, so changing a value does not make it wrong.

## Principles

1. **Things become the next thing.** Nothing opens like a tab. What is on screen turns
   into what comes next: a tile becomes the picture, the picture becomes cards.
2. **One room.** The conversation and the stage share a surface with no line between
   them. `StageShell::paintEvent` paints the room and the picture's shadow; the
   conversation and the picture's region are clear over it.
3. **The picture floats over the conversation.** Motion that crosses regions is drawn
   above them, in `StageMotion`.
4. **Motion never decides anything.** State changes at once and motion follows it, so
   a stopped animation leaves nothing wrong.
5. **Reduce Motion means simply there.** Every scene has to work with no motion.
   Tests run this way (`KERF_REDUCE_MOTION=1`).
6. **Never wait on a blank.** If a render is coming, something stands in for it.
7. **Nothing moves the picture's colours.** The room around the picture stays neutral,
   whatever it shows: colour around a picture changes how its colours look, and
   this is where they are judged. Motion around it uses no colour taken from it.

## Scenes

| Scene | When | What moves | Code | Test |
|---|---|---|---|---|
| Start window | It opens | Heading, box and ideas rise in turn | `HomeWindow::enter` | `aRiseLeavesTheLayoutAlone` |
| Sidebar | It opens or closes | It slides in from the window's edge; the toolbar's leading stretch, its top, follows its edge and the title moves with it | `StageShell::transition`, `ToolbarWiring::layoutLeading`, `FlatToolBar::setSidebarEdge` | `toolbarHoldsOnlyTheSidebarTheTitleExportAndThePanel` (the end state) |
| Export arrives | The program gets its first clip | Export fades in where it is, in the toolbar | `ToolbarWiring::followCut` | `toolbarHoldsOnlyTheSidebarTheTitleExportAndThePanel` (the end state) |
| Account menu | The account at the sidebar's foot is pressed | The menu fades in and rises over the account, which stays held | `ToolbarWiring::popupAppMenu`, `ui::popupMenu` (`Side::Above`) | none |
| Settings | ⌘, or the account's menu | The page has the room at once; its body comes up into place, and a page chosen fades in over the last | `MainWindow::showSettings`, `StageShell::setCovered` | `theAccountMenuReachesEverythingElse` (the end state) |
| Hand-over | A request is sent | Start window fades over the editor; its box goes to the composer | `MainWindow::takeOverFrom`, `main.cpp` | none |
| Footage | Kerf starts with footage and no cut | Named tiles pop in, in the conversation | `MainWindow::showSheet`, `FootageSheet` | `footageTilesKeepTheirPlacesAsTheyGoOn` |
| Picture arrives | A project opens with clips on its timeline | First tile grows into the picture as the conversation gives way; the timeline comes in out of the room | `MainWindow::followContentWithPanes`, `MainWindow::growStage`, `StageShell::transition`, `StageMotion::followPicture` | `regionsMoveToANewArrangementWhenMotionIsOn`, `aMoveCutShortStillEnds`, `motionLeavesNothingBehind`, `aStandInGoesWithThePicture` |
| Cut at work | Kerf makes a first cut | Its card is in Kerf's row of the turn: light runs round the picture, which sits a little back and crossfades to each clip as it is placed, with Cutting on it and a line of light under it. Done, the ring lights once and goes out, the picture lightens, Done shows and fades, then the play mark and the length | `ChatPanel::setBuildingProposal`, `CardPicture::setWorking`, `CardPicture::setFinished`, `quietenCutPreview` | `aCutAtWorkIsItsCardInTheTurn` |
| Cut card | A first cut Kerf made is pressed | The picture grows out of the card as the conversation gives way and the timeline comes in; the card says it is on the stage. Closed, the picture goes back into the card. Moving over the card runs through its clips | `CardPicture`, `MainWindow::openCutFromCard`, `MainWindow::growStage`, `MainWindow::cardPictureHome`, `StageShell::setPictureHome` | `aCutKerfMadeOpensFromItsCard` |
| Cut builds | Each new clip in a proposal, with the stage open | Its tile flies to its lane, ringed; the picture becomes that clip | `MainWindow::flyChange`, `coverPicture` | `flightsWaitForMotionAndEndOnTheirOwn`, `aStandInLetGoEarlyGoesOnFromWhereItIs`, `aStandInHoldingStillDrawsNoFrames` |
| First cut shows | A proposal on an empty program | Timeline shows it, read-only; the picture takes its format and renders it | `MainWindow::showProposedCut` | `aFirstCutShowsOnTheTimelineUntilItIsTheProgram`, `aProposedCutBringsItsOwnFormat` |
| Kerf at work | During a turn | Kerf cursor on the timeline; working chip on the picture | `TimelineWidget::setAgentCursor`, `WorkChip`, `quietenCutPreview` | none |
| Watch changes | "Watch what changed" | Stage reaches over the dimmed conversation; before wipes to and fro over after; changed clips lift | `MainWindow::watchChanges`, `followReelCompare`, `ViewerWidget::setCompareFrame`, `TimelineWidget::setFocusClips` | `watchingLeavesARoomOverThePicture`, `theStripSitsOnAnUprightPicture`, `theBeforeStillComesAndGoes`, `theLastDimIsTheOneThatStays`, `focusedClipsRiseOnce` |
| Going home | Kerf has rendered | The picture shrinks into the folder; the conversation has the room | `MainWindow::goHome` | `theVeilDoesNotOutliveAMoveCutShort` |
| Manual export | An export you started finishes | A frame flies to the folder; the panel stays | `MainWindow::flyExportHome` | none |

"none" means it has only been checked in a real window. Motion can only be judged
there, or from screenshots taken while it runs.

## What to reuse

Look here before writing a new animation.

| You want | Use |
|---|---|
| Regions to move to a new arrangement | `StageShell::setPanes`, with an origin for the picture to grow from or return to; `StageShell::setPictureHome` for where it returns to when no origin is given |
| The stage to grow out of something, with a stand-in until its frame | `MainWindow::growStage` |
| Something to travel across regions | `StageMotion::fly` with a `Flight` |
| Something to hold a place until the real thing comes | `Flight::stays`, ended by `releaseStaying` |
| A stand-in for the picture | `Flight::fit`, so it shows footage as the render will |
| Something to fade in where it is | `moves::fadeIn` |
| Something to rise into place, in a layout or not | `moves::rise` |
| A page that takes the room (Settings) | `StageShell::setCover`, shown with `setCovered`; raise its body, not the page (see **What a frame costs**) |
| Something in the toolbar to keep to the sidebar's edge | `ToolbarWiring::sidebarEdge`, which follows the sidebar while it moves |
| A region to fall back | `moves::dim` |
| The timeline to fall back or come in | `StageShell::veilTimeline` (`moves::dim` does not work on it) |
| A duration, stagger or distance | A name in `tokens::motion`, added there if none fits |
| The curve | `tokens::easing()`; `landOvershoot` and `popOvershoot` for what lands or pops |

## Adding or changing a scene

1. Decide what on screen becomes the new thing. If nothing does, ask whether it should
   exist as a separate thing at all.
2. Reuse what is above. Put new values in `tokens::motion`, never in the code.
3. Decide what shows while anything slow is happening.
4. Check it in **16:9 and 9:16**, and from an empty project as well as one with a cut.
5. Check it with Reduce Motion on, and test the end state without motion.
6. Keep a frame's work small: see **What a frame costs**.
7. Update the **Scenes** table here in the same change.

## What a frame costs

The editor paints on the CPU, and everything clear over what changes is drawn again
with it. So:

- **Mark only what changed**, not the widget: `StageMotion::tick` marks where a flight
  was and is, `StageShell::updateRoom` where the shadow falls, `ViewerWidget::updateLine`
  what a line crossed. `motionLeavesNothingBehind` fails if too little is marked.
- **Nothing holding still draws frames.** A stand-in that has landed stops the ticker,
  however long its render takes.
- **Prepare an image once, when it starts moving** (`readyToMove`): no bigger than it
  is shown, in `RGB32` or premultiplied `ARGB32`. Never scale one in `paintEvent` per
  frame; the picture on the move draws what it has (`ViewerWidget::setStageMoving`).
- **A long animation changes what can be seen, not every tick.** The working chip draws
  `workPulseSteps` a pulse.
- **One motion per property.** A new one takes over from the last, from where it has
  got to (`moves::dim`). Two at once fight over each frame.
- **Every start has its end**, also when cut short: `transitionFinished` always follows
  `transitionStarted`.
- Avoid a new `QGraphicsEffect` on a large region: it draws all of it again each frame.
- **Frames that keep coming are drawn as they come.** While playing, each frame is
  sampled straight onto the screen, and the last is drawn at its best once they stop
  (`ViewerWidget::setFrame`, `tokens::motion::frameStream`). A still watched for long,
  as the before side of the wipe, is made at its size once.
- **Draw an icon once.** `icons::paint` keeps each icon it has filled, at its size,
  colour and place within a pixel, and copies it from then on.
- **Away from the picture's corners, keep to its rect.** Rounding the clip costs a pass
  over the whole picture, so what is drawn again in its middle is kept inside its rect.

To measure, from the repository root:

```sh
KERF_MOTION_COSTS=1 QT_SCALE_FACTOR=2 target/qt/kerf-qt-tests -platform offscreen whatEachSceneCosts
```

This reads a frame's paint and the process's CPU, on the offscreen platform, so it
compares one build with another and leaves out putting the frame on a display.
`whatAConversationCosts`, run the same way, times the window's own parts as a
conversation grows: the conversation made again, a comment arriving, a snapshot taken
in, and each region drawn whole.

## Known limits

- `QGraphicsOpacityEffect` does not dim `TimelineWidget`; use the veil.
- The before side of the wipe is a still, as two streams cannot play at once. A cut
  that is all new has nothing to compare, so it has no wipe.
- `Going home` follows the turn that called `render`, not that render finishing.
- Stand-ins are posters, so they are soft until the render arrives.
- Screenshots do not show whether motion is smooth, and `whatEachSceneCosts` shows what
  it costs, not how it looks. Only a real window shows that.
- While the regions move, the conversation's list keeps the width it is going to
  (`ChatPanel::holdWidth`): its text wraps once, at the start, not in each frame.
