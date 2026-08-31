import { describe, expect, it } from "vitest";
import {
  detectEncoderCapabilities,
  parseEncoderCapabilities,
  selectVideoEncoder,
} from "../src/encoder.js";

describe("hardware encoder selection", () => {
  it("parses known encoders and keeps software output as the default", () => {
    const capabilities = parseEncoderCapabilities([
      " V....D h264_videotoolbox VideoToolbox H.264 Encoder",
      " V....D hevc_qsv Intel Quick Sync Video",
    ].join("\n"));
    expect(selectVideoEncoder(capabilities)).toMatchObject({ encoder: "libx264", hardware: false });
    expect(selectVideoEncoder(capabilities, { preferHardware: true })).toMatchObject({
      encoder: "h264_videotoolbox",
      hardware: true,
      family: "videotoolbox",
    });
    expect(selectVideoEncoder(capabilities, { preferHardware: true, codec: "hevc" })).toMatchObject({
      encoder: "hevc_qsv",
      hardware: true,
      family: "qsv",
    });
  });

  it("supports an injected detector without spawning ffmpeg", async () => {
    const capabilities = await detectEncoderCapabilities({
      ffmpegBin: "fake-ffmpeg",
      runner: async (_bin, args) => {
        expect(args).toEqual(["-hide_banner", "-encoders"]);
        return " V....D h264_nvenc NVIDIA NVENC H.264 encoder";
      },
    });
    expect(capabilities.encoders.find((entry) => entry.encoder === "h264_nvenc")?.available).toBe(true);
  });
});
