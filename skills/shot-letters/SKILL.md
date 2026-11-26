---
name: shot-letters
description: Keep what has to be exact in a shot a model films, a sign's letters, a logo, a count, by having Kerf draw it over the take.
---

# What Kerf draws in a shot a model films

- A model draws letters wrong. What has to be exact, a sign's letters, a
  logo, a count, is Kerf's to draw: put it in the shot's scene as any clip,
  in depth where it stands there.
- Say `filmed=true` on every other clip of the scene that draws: the
  backdrop, the floor, each solid. A camera, a light, a null, and a clip on
  a hidden track that a light or an effect reads say nothing.
- Write the guide's step with `only=filmed`: the model is handed the scene
  without what Kerf draws, and draws nothing in its place. What the step
  after it is handed with the guide (`moves=`) is read as that guide shows
  it, and so is a take held to its guide.
- Where the shot is cut in, lay a clip of the scene (`seq=<scene>`) on the
  track over the shot's clip, from the same frame and for as long, placed
  and scaled as the shot's clip is. There the scene shows only what no model
  films, through its own camera and lights, cut by whatever a model films
  before it. In its own sequence the scene shows every clip as ever.
- A transition between a clip a model films and one Kerf draws shows the
  take where it would show the filmed clip, moved, wiped, blurred, or lit as
  the transition has it. One that goes by how bright the filmed clip is (a
  burn or a luma fade read from it, a ray, a glow, a morph cut), or tints or
  splits its colors, is a plain dissolve there: pick another if its look
  matters.
- The cut is where the guide's solid ends, so a take that strays from its
  guide shows at it: hold the take to the guide (`look` with `follows`)
  before laying the scene over it.
- Changing what Kerf draws, a word of the sign, asks for no new take: the
  guide is as it was.
