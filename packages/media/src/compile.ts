/**
 * Filtergraph compiler — pure logic, no ffprobe/ffmpeg needed.
 *
 * Builds an ffmpeg argv (and a pretty string) that renders a Timeline:
 *  - one input per source (`-i path`)
 *  - per clip: trim + setpts (PTS-based), optional setpts/atempo for speed,
 *    optional volume filter
 *  - concat filter across the clips of each track
 *  - tracks are concatenated too when there is more than one
 */
import type { Clip, Source, SubtitlesOverlay, Timeline } from "@cut/timeline";

/**
 * Escape a file path for use inside an ffmpeg filter argument (macOS-safe):
 * backslashes and colons are special inside filter values, and the path is
 * wrapped in single quotes which also need escaping.
 */
export function escapeFilterPath(p: string): string {
  return p
    .replace(/\\/g, "\\\\")
    .replace(/:/g, "\\:")
    .replace(/'/g, "\\'");
}

/** Convert '#rrggbb' to ASS '&HAABBGGRR'; pass other formats through. */
function toAssColor(c: string): string {
  const m = /^#([0-9a-fA-F]{6})$/.exec(c);
  if (!m) return c;
  const r = c.slice(1, 3)!;
  const g = c.slice(3, 5)!;
  const b = c.slice(5, 7)!;
  return `&H00${b}${g}${r}`.toUpperCase();
}

const ALIGNMENT: Record<NonNullable<SubtitlesOverlay["style"]>["position"] & string, number> = {
  bottom: 2,
  center: 5,
  top: 8,
};

/** Build the `subtitles=` filter string for one overlay entry. */
export function subtitlesFilter(o: SubtitlesOverlay): string {
  const styleParts: string[] = [];
  if (o.style?.fontSize !== undefined) styleParts.push(`FontSize=${o.style.fontSize}`);
  if (o.style?.fontColor !== undefined) styleParts.push(`PrimaryColour=${toAssColor(o.style.fontColor)}`);
  if (o.style?.outlineColor !== undefined) styleParts.push(`OutlineColour=${toAssColor(o.style.outlineColor)}`);
  if (o.style?.position !== undefined) {
    styleParts.push(`Alignment=${ALIGNMENT[o.style.position]}`);
    if (o.style.marginV !== undefined) styleParts.push(`MarginV=${o.style.marginV}`);
  }
  const forceStyle = styleParts.length ? `:force_style='${styleParts.join(",")}'` : "";
  return `subtitles=filename='${escapeFilterPath(o.path)}'${forceStyle}`;
}

export interface CompiledFilterGraph {
  /** Full ffmpeg argv array (spawn-safe, no shell). */
  argv: string[];
  /** Human-readable rendering of the same command. */
  command: string;
  /** The filter_complex string itself. */
  filterComplex: string;
}

function fmt(n: number): string {
  // Trim floating point noise but keep enough precision for timecodes.
  return String(Number(n.toFixed(6)));
}

interface ClipChain {
  /** output label of this clip's video stream (or undefined if no video) */
  v?: string;
  /** output label of this clip's audio stream (or undefined if no audio) */
  a?: string;
}

function buildClipChain(
  clip: Clip,
  inputIdx: number,
  hasAudio: boolean,
  parts: string[],
): ClipChain {
  const dur = clip.out - clip.in;
  const vIn = `${inputIdx}:v`;
  const aIn = `${inputIdx}:a`;
  const chain: ClipChain = {};

  // --- video ---
  let vLabel = `v${inputIdx}`;
  const vFilters: string[] = [
    `trim=start=${fmt(clip.in)}:end=${fmt(clip.out)}`,
    "setpts=PTS-STARTPTS",
  ];
  if (clip.speed !== undefined && clip.speed !== 1) {
    vFilters.push(`setpts=${fmt(1 / clip.speed)}*PTS`);
  }
  parts.push(`[${vIn}]${vFilters.join(",")}[${vLabel}]`);
  chain.v = vLabel;

  // --- audio ---
  if (hasAudio) {
    const aFilters: string[] = [
      `atrim=start=${fmt(clip.in)}:end=${fmt(clip.out)}`,
      "asetpts=PTS-STARTPTS",
    ];
    if (clip.speed !== undefined && clip.speed !== 1) {
      // atempo only supports [0.5, 100]; for extreme speeds we'd chain —
      // keep it simple and clamp-free for now.
      aFilters.push(`atempo=${fmt(clip.speed)}`);
    }
    if (clip.volume !== undefined && clip.volume !== 1) {
      aFilters.push(`volume=${fmt(clip.volume)}`);
    }
    const aLabel = `a${inputIdx}`;
    parts.push(`[${aIn}]${aFilters.join(",")}[${aLabel}]`);
    chain.a = aLabel;
  }
  return chain;
}

function buildConcatSection(
  chains: ClipChain[],
  hasAudio: boolean,
  parts: string[],
  tag: string,
): { v: string; a?: string } {
  const vout = `${tag}v`;
  if (hasAudio && chains.every((c) => c.a !== undefined)) {
    // concat expects interleaved [v0][a0][v1][a1]... when a>0
    const ins = chains.map((c) => `[${c.v!}][${c.a!}]`).join("");
    const aout = `${tag}a`;
    parts.push(`${ins}concat=n=${chains.length}:v=1:a=1[${vout}][${aout}]`);
    return { v: vout, a: aout };
  }
  const vins = chains.map((c) => `[${c.v!}]`).join("");
  parts.push(`${vins}concat=n=${chains.length}:v=1:a=0[${vout}]`);
  return { v: vout };
}

/** Total output duration of a timeline in seconds. */
export function timelineDuration(timeline: Timeline): number {
  let total = 0;
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      const d = clip.out - clip.in;
      total += clip.speed !== undefined ? d / clip.speed : d;
    }
  }
  return total;
}

export function compileFilterGraph(
  timeline: Timeline,
  opts: { videoPostFilters?: string[] } = {},
): CompiledFilterGraph {
  const srcById = new Map<string, Source>();
  for (const s of timeline.sources) srcById.set(s.id, s);

  const parts: string[] = [];
  const inputs: string[] = [];
  const inputIndexOfSource = new Map<string, number>();

  // One -i per unique source actually used.
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (!inputIndexOfSource.has(clip.src)) {
        const src = srcById.get(clip.src);
        if (!src) throw new Error(`Unknown source id: ${clip.src}`);
        inputIndexOfSource.set(clip.src, inputs.length / 2);
        inputs.push("-i", src.path);
      }
    }
  }

  const trackOutputs: { v: string; a?: string }[] = [];
  timeline.tracks.forEach((track, ti) => {
    // No ffprobe pass here (pure logic); assume every source has audio.
    // A missing audio stream would surface as an ffmpeg error at render time.
    const assumeAudio = true;

    const chains = track.clips.map((clip) =>
      buildClipChain(clip, inputIndexOfSource.get(clip.src)!, assumeAudio, parts),
    );

    if (chains.length === 0) throw new Error(`Track ${track.id} has no clips`);

    const out =
      chains.length === 1 ? chains[0]! : buildConcatSection(chains, true, parts, `t${ti}_`);
    trackOutputs.push({
      v: out.v!,
      ...(out.a !== undefined ? { a: out.a } : {}),
    });
  });

  // Concat across tracks when more than one.
  let finalV: string;
  let finalA: string | undefined;
  if (trackOutputs.length === 1) {
    finalV = trackOutputs[0]!.v;
    finalA = trackOutputs[0]!.a;
  } else {
    const out = buildConcatSection(
      trackOutputs.map((o) => ({
        v: o.v,
        ...(o.a !== undefined ? { a: o.a } : {}),
      })),
      true,
      parts,
      "tracks_",
    );
    finalV = out.v;
    finalA = out.a;
  }

  // Optional post-filters (e.g. draft scale/fps) applied inside the graph.
  if (opts.videoPostFilters?.length) {
    parts.push(`[${finalV}]${opts.videoPostFilters.join(",")}[${finalV}out]`);
    finalV = `${finalV}out`;
  }

  // Burn in subtitle overlays, chained after the final video output.
  const subOverlays = timeline.overlays.filter(
    (o): o is SubtitlesOverlay => (o as { type?: string }).type === "subtitles",
  );
  for (const o of subOverlays) {
    const outLabel = `${finalV}subs`;
    parts.push(`[${finalV}]${subtitlesFilter(o)}[${outLabel}]`);
    finalV = outLabel;
  }

  const filterComplex = parts.join(";\n");
  const fpsArgs = ["-r", String(timeline.fps)];
  const argv = [
    "ffmpeg",
    "-y",
    ...inputs,
    "-filter_complex",
    filterComplex.replace(/\n/g, ""),
    "-map",
    `[${finalV}]`,
    ...(finalA ? ["-map", `[${finalA}]`] : []),
    ...fpsArgs,
    // caller appends codec/output-specific args via render()
  ];

  const command = ["ffmpeg", ...argv.slice(1)].join(" ");
  return { argv, command, filterComplex };
}
