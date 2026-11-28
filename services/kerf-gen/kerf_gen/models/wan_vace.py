"""Wan 2.1 VACE 1.3B, through diffusers' WanVACEPipeline: a shot that follows a guide Kerf draws
(its depth, its ids, its picture), from a first frame or with pictures to look like, as a prompt
says. What it takes is said here and nowhere else.

Its weights are the single files Wan is published as, in `KERF_GEN_WAN_VACE` (by default
`/workspace/models/wan-vace-1.3b`): `wan2.1_vace_1.3B_fp16.safetensors`,
`umt5_xxl_fp16.safetensors` and `wan_2.1_vae.safetensors`, with diffusers' configs and the
tokenizer beside them in `diffusers/`, from `Wan-AI/Wan2.1-VACE-1.3B-diffusers`.

A line can set `steps` (30), `guidance` (5), `strength`, how closely the guide is followed (1),
and `negative`, what the picture should not have.
"""
import os
import random
import subprocess
import sys
import threading

# MIOpen searches every way to run a convolution the first time it meets one, which takes
# minutes for the VAE's; FAST takes the first that works.
os.environ.setdefault("MIOPEN_FIND_MODE", "FAST")

from . import Stopped  # noqa: E402

NAME = "wan-vace"
WIDTH, HEIGHT = 832, 480
RATE = 16
FRAMES = 81
SECONDS = (FRAMES - 1) / RATE
STEPS = 30
GUIDANCE = 5.0
NEGATIVE = (
    "bright colors, overexposed, static, blurred details, subtitles, style, artwork, painting, "
    "picture, still, overall gray, worst quality, low quality, JPEG artifacts, ugly, deformed, "
    "extra fingers, poorly drawn hands, poorly drawn face, mutated, disfigured, misshapen limbs, "
    "fused fingers, still picture, cluttered background, three legs, many people in the "
    "background, walking backwards"
)

SPEC = {
    "about": "Wan 2.1 VACE 1.3B: a shot that follows its guide, from a first frame or like its references",
    "size": [WIDTH, HEIGHT],
    "rate": RATE,
    "seconds": SECONDS,
    "reads": ["guide", "first", "ref", "regions"],
    "more": {"steps": STEPS, "guidance": GUIDANCE, "strength": 1.0, "negative": ""},
}

STILLS = (".png", ".jpg", ".jpeg", ".webp")

_pipe = None
_loading = threading.Lock()


def weights():
    return os.environ.get("KERF_GEN_WAN_VACE", "/workspace/models/wan-vace-1.3b")


def ffmpeg():
    return os.environ.get("KERF_GEN_FFMPEG", "ffmpeg")


def prepare():
    """Load the pipeline onto the GPU, once: a server does it before it listens."""
    global _pipe
    with _loading:
        if _pipe is not None:
            return _pipe
        import torch
        from diffusers import (
            AutoencoderKLWan,
            UniPCMultistepScheduler,
            WanVACEPipeline,
            WanVACETransformer3DModel,
        )
        from safetensors.torch import load_file
        from transformers import AutoTokenizer, UMT5EncoderModel

        folder = weights()
        configs = os.path.join(folder, "diffusers")
        print(f"loading {NAME} from {folder}", file=sys.stderr, flush=True)
        transformer = WanVACETransformer3DModel.from_single_file(
            os.path.join(folder, "wan2.1_vace_1.3B_fp16.safetensors"),
            config=configs,
            subfolder="transformer",
            torch_dtype=torch.bfloat16,
        )
        vae = AutoencoderKLWan.from_single_file(
            os.path.join(folder, "wan_2.1_vae.safetensors"),
            config=configs,
            subfolder="vae",
            torch_dtype=torch.float32,
        )
        text_encoder = UMT5EncoderModel.from_pretrained(
            None,
            config=UMT5EncoderModel.config_class.from_pretrained(os.path.join(configs, "text_encoder")),
            state_dict=load_file(os.path.join(folder, "umt5_xxl_fp16.safetensors")),
            torch_dtype=torch.bfloat16,
        )
        pipe = WanVACEPipeline(
            tokenizer=AutoTokenizer.from_pretrained(os.path.join(configs, "tokenizer")),
            text_encoder=text_encoder,
            vae=vae,
            scheduler=UniPCMultistepScheduler.from_pretrained(os.path.join(configs, "scheduler")),
            transformer=transformer,
        )
        _pipe = pipe.to("cuda")
        print(f"{NAME} is loaded", file=sys.stderr, flush=True)
        return _pipe


def frames_for(seconds):
    """How many frames a shot of `seconds` is: Wan makes 4k + 1, up to 81."""
    if not seconds:
        return FRAMES
    wanted = round(float(seconds) * RATE / 4) * 4 + 1
    return max(5, min(FRAMES, wanted))


def size_of(request):
    """The size a request asks, each side a multiple of 16, else the model's own."""
    size = request.get("size")
    if not size:
        return WIDTH, HEIGHT
    width, height = (max(16, int(side) // 16 * 16) for side in size)
    return width, height


def prompt_of(request):
    """The prompt, with what clips of the guide say they are after it."""
    said = [request.get("prompt") or ""]
    said += [region["prompt"] for region in request.get("regions") or [] if region.get("prompt")]
    return ". ".join(part.strip().rstrip(".") for part in said if part.strip())


def fit(image, width, height):
    """`image` filling `width` x `height`, what is over cut off evenly."""
    from PIL import Image

    image = image.convert("RGB")
    scale = max(width / image.width, height / image.height)
    resized = image.resize(
        (max(width, round(image.width * scale)), max(height, round(image.height * scale))),
        Image.LANCZOS,
    )
    left = (resized.width - width) // 2
    top = (resized.height - height) // 2
    return resized.crop((left, top, left + width, top + height))


def read_frames(path, width, height, count):
    """The first `count` frames of the video at `path`, filling `width` x `height` at the
    model's rate; the last is held where it is shorter."""
    import numpy as np
    from PIL import Image

    picture = (
        f"scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},fps={RATE}"
    )
    read = subprocess.run(
        [ffmpeg(), "-hide_banner", "-loglevel", "error", "-i", path, "-vf", picture,
         "-frames:v", str(count), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
        capture_output=True,
    )
    if read.returncode != 0:
        raise RuntimeError(f"the guide does not read: {read.stderr.decode(errors='replace').strip()}")
    frames = np.frombuffer(read.stdout, dtype=np.uint8).reshape(-1, height, width, 3)
    if len(frames) == 0:
        raise RuntimeError("the guide has no frames")
    images = [Image.fromarray(frame) for frame in frames]
    while len(images) < count:
        images.append(images[-1])
    return images


def number(more, name, default):
    try:
        return float(more.get(name, default))
    except (TypeError, ValueError):
        raise RuntimeError(f"`{name}={more.get(name)}` is not a number")


def generate(request, out, report, stopped):
    """Make what `request` asks, every file in it a path here, at `out`."""
    import numpy as np
    import torch
    from PIL import Image

    pipe = prepare()
    more = request.get("more") or {}
    still = os.path.splitext(out)[1].lower() in STILLS
    width, height = size_of(request)
    count = 1 if still else frames_for(request.get("seconds"))
    steps = max(1, int(number(more, "steps", STEPS)))
    guide = request.get("guide")
    report(0.0, "reading what it is handed")
    if guide:
        video = read_frames(guide, width, height, count)
    else:
        # Grey, the colour VACE reads as no guide.
        video = [Image.new("RGB", (width, height), (127, 127, 127)) for _ in range(count)]
    # White where it makes the picture; black where it keeps what it is handed.
    mask = [Image.new("L", (width, height), 255) for _ in range(count)]
    if request.get("first"):
        video[0] = fit(Image.open(request["first"]), width, height)
        mask[0] = Image.new("L", (width, height), 0)
    references = [fit(Image.open(path), width, height) for path in request.get("references") or []]
    seed = request.get("seed")
    if seed is None:
        seed = random.randrange(2**32)

    def step_ended(pipeline, step, timestep, tensors):
        if stopped():
            raise Stopped()
        report((step + 1) / steps, f"step {step + 1} of {steps}")
        return tensors

    made = pipe(
        video=video,
        mask=mask,
        reference_images=references or None,
        prompt=prompt_of(request),
        negative_prompt=more.get("negative") or NEGATIVE,
        height=height,
        width=width,
        num_frames=count,
        num_inference_steps=steps,
        guidance_scale=number(more, "guidance", GUIDANCE),
        conditioning_scale=number(more, "strength", 1.0),
        generator=torch.Generator("cuda").manual_seed(int(seed) % 2**63),
        callback_on_step_end=step_ended,
        output_type="np",
    ).frames[0]
    if stopped():
        raise Stopped()
    report(1.0, "writing what it made")
    frames = (np.clip(made, 0.0, 1.0) * 255).round().astype(np.uint8)
    command = [ffmpeg(), "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo",
               "-pix_fmt", "rgb24", "-s", f"{width}x{height}", "-r", str(RATE), "-i", "-"]
    if still:
        command += ["-frames:v", "1"]
    else:
        command += ["-c:v", "libx264", "-preset", "medium", "-crf", "16", "-pix_fmt", "yuv420p"]
    command.append(out)
    written = subprocess.run(command, input=frames.tobytes(), capture_output=True)
    if written.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {written.stderr.decode(errors='replace').strip()}")
