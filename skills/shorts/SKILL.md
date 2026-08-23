---
name: shorts
description: Recipe for cutting 9:16 vertical shorts — crop/scale filters, hook in the first 3 seconds, remove silence first.
---

# Vertical Shorts Recipe

Goal: turn long-form footage into a punchy 9:16 short (15–60s).

## Order of operations

1. **Remove silence first.** Run silence detection (`probe {action:"silence"}`),
   then use `edit` to drop silent spans before doing anything else. Tight audio
   makes every later timing decision honest.
2. **Pick the hook.** The first 3 seconds decide retention. Choose the span with
   the strongest claim, question, or visual moment and move it (or cut to it)
   so it opens the short. Never start with dead air or a slow intro.
3. **Crop to 9:16.** Render with an ffmpeg filter chain like:

   ```
   crop=ih*9/16:ih:(iw-ih*9/16)/2:0,scale=1080:1920
   ```

   Center-crop by default; shift the crop x-offset toward the speaker's side
   when the subject is off-center.
4. **Trim to length.** Target ≤ 60s. Cut mid-sentence pauses aggressively —
   jump cuts are the house style, not a flaw.

## Checklist

- [ ] Silence removed
- [ ] Hook lands within the first 3s
- [ ] 9:16 crop + scale to 1080x1920
- [ ] Subtitles burned in (bottom third, large font) — most shorts are watched muted
