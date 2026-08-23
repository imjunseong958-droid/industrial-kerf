/**
 * @cut/media — ffmpeg/ffprobe wrappers (probe + render compiler live here).
 * Placeholder: interfaces only for now.
 */

export interface ProbeInfo {
  duration?: number;
  width?: number;
  height?: number;
  fps?: number;
  audioCodec?: string;
  videoCodec?: string;
}
