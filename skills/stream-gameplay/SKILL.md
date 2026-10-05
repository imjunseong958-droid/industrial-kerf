---
name: stream-gameplay
description: Reframe gameplay while keeping the game, camera, and chat regions the user asked for.
---

# Stream and gameplay reframing

- Treat the game as the framing anchor unless the user names another
  subject.
- Find the game, camera, and chat regions in observed frames. Frame from
  what you saw; do not assume a layout or invent motion between frames.
- Keep every requested region visible. When one frame cannot hold them all,
  stack or place them as separate layers and say which composition you chose
  and why.
- Never silently crop away chat or the camera when the user asked for them.
- Layouts change mid-stream (menus, scene switches, camera moves). Split
  where the layout changes and frame each part on its own.
- Check the edges of every crop and every cut in the result before handing it
  over.
