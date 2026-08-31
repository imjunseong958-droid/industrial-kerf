# `@cut/interchange`

OpenTimelineIO interchange for cut v2 projects through an external Python
sidecar. The package does not install Python packages, download adapters, or
probe the filesystem during construction. Set `pythonExecutable` to a Python
environment that already has the official `opentimelineio` package installed.

```ts
import { createOpenTimelineIOAdapter } from "@cut/interchange";

const otio = createOpenTimelineIOAdapter({
  pythonExecutable: "/path/to/python3",
  timeoutMs: 120_000,
});

const imported = await otio.import("/tmp/edit.otio");
const exported = await otio.export(imported.project, "/tmp/edit-copy.otio", {
  adapter: "otio_json",
});

console.log(imported.project, imported.report.losses);
console.log(exported.outputPath, exported.report.warnings);
```

The process boundary is injectable (`runner`/`processRunner`) so unit tests can
return deterministic bridge JSON without Python or OTIO installed. The bundled
bridge is `python/otio_bridge.py`; it accepts `import` and `export` commands,
reads export JSON from stdin, and writes one JSON response to stdout.

## Mapping

Import and export preserve the portable subset:

- timeline and media names/paths, including `file:` URL paths;
- video/audio tracks and their explicit ordering;
- clip `timelineStart`, source `in`/`out`, inferred speed, and source duration;
- intentional track gaps and basic track-level transitions;
- timeline, track, and clip markers with labels, ranges, colors, and metadata;
- OTIO adapter selection for `otio_json`, `otioz`, or an installed external
  adapter plugin.

Cut v2 effects, keyframes, transforms, blend modes, title/adjustment tracks,
overlays, and adapter-specific features have no portable OTIO mapping. They
appear in `report.losses` and `report.warnings`. Nothing is silently discarded.
OTIO effects encountered during import stay as descriptive cut effect entries
and are still reported.

Explicit starts after the current OTIO sequence position are represented with
leading/interstitial `Gap` objects. Overlapping clip, gap, or transition starts
cannot be represented by a single OTIO track; the bridge deterministically
appends the item at the current sequence position and reports a
`placement_overlap` loss with the requested and effective starts.

`otio_json` and `otioz` are the only adapters this package identifies as native
OTIO adapters. FCPXML, AAF, and other interchange formats require the matching
external OTIO adapter plugin in the selected Python environment; this package
does not claim those formats are available by default.

The TypeScript adapter validates paths, adapter names, timeout values, project
shape, subprocess exit status, JSON output, timeout, and abort. See the typed
errors exported from `src/errors.ts` for handling failures.
