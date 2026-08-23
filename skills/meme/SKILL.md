---
name: meme
description: Meme-style edits — speed ramps and zoom punch-ins driven by crop+scale expressions timed to beats.
---

# Meme Edit Recipe

Goal: high-energy meme cuts using speed ramps and zoom punch-ins.

## Speed ramps

Split the timeline at ramp points, then set per-segment speed:

- Build-up: 1.5–2× (fast)
- Punchline/key moment: 0.5× or freeze-frame (slow-mo sells the beat)
- After the punchline: snap back to 1×

Keep ramps aligned to audio peaks — probe the audio waveform and place ramp
boundaries on transients, not on round timestamps.

## Zoom punch-ins

Emphasize moments with quick zooms via ffmpeg crop+scale expressions:

```
# ease-in punch to ~115% over 0.3s starting at t=2.5
scale=iw*1.15:ih*1.15,zoompan=z='min(1+0.15*(on/8),1.15)':d=8:s=1920x1080
```

Or with crop expressions for a re-framed punch-in:

```
crop=w='iw-(iw*0.12*min(max(t-2.5,0)*3.33,1))':h='ih-(ih*0.12*min(max(t-2.5,0)*3.33,1))'
```

Rules of thumb:

- Punch-in duration 0.2–0.4s; longer feels sluggish.
- Stack at most two effects on the same moment (zoom + speed change max).
- Always end the edit back at neutral framing/speed so the loop reads clean.
