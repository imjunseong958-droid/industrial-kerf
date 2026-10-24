# effectcraft-segment

Trained models for Roto Brush, behind one swappable interface.

- **`MaskModel`**: what Roto Brush asks of a model: given a frame and prompts (foreground /
  background points, a box, a prior mask), the foreground probability of every pixel. Roto Brush
  (`effectcraft-track`) only talks to this trait, so models can be added or swapped without
  touching it. The classical graph-cut segmenter stays built in as the fallback.
- **`MODELS`**: the registry. Every entry must be open source under a licence compatible with
  EffectCraft's MIT OR Apache-2.0 (`ALLOWED_LICENCES`), with its task, authors, homepage, licence
  URL, the official weights URL, size and SHA-256. `load` refuses anything else.
- **Weights are never bundled.** They are downloaded on demand (or installed from a file) by the
  engine (`roto.model.*`, Settings ▸ Roto Brush), verified
  against the registry's SHA-256 (`sha256`, dependency-free) and kept with a notice naming their
  authors, licence and source (`notice`).
- **`pt`**: reads PyTorch checkpoints (`torch.save` zip archives and the pickle subset state dicts
  use), so official weights load exactly as published.
- **`mobilesam`**: MobileSAM in plain Rust: the TinyViT-5M image encoder and Segment Anything's
  prompt encoder and two-way-transformer mask decoder.

All of it runs on `nn` kernels (rayon, and ndarray's safe `general_mat_mul`, which picks AVX/FMA
kernels at run time). No ML framework, no unsafe code.

## Models

| id | Task | Model | Authors | Licence | File | Size |
|---|---|---|---|---|---|---|
| `mobilesam` | Roto Brush | [MobileSAM](https://github.com/ChaoningZhang/MobileSAM): Segment Anything with a 5M-parameter TinyViT encoder | Chaoning Zhang et al., building on Meta AI's Segment Anything and Microsoft's TinyViT | Apache-2.0 | `mobile_sam.pt` from the official repository | 40.7 MB |

How they were checked:

- **MobileSAM**: the implementation follows the published architectures (MobileSAM, TinyViT and
  Segment Anything papers and their Apache-2.0 reference code). Its output was checked against an
  independent implementation (Hugging Face candle's, MIT/Apache-2.0) on the same weights:
  embeddings within 0.3% RMS (exact vs. tanh GELU) and 99.9% of mask pixels equal.

Tests that need the weights run when `EFFECTCRAFT_MOBILESAM` points at the file, and are
skipped otherwise.

## Adding a model

1. Pick a model whose code **and** weights are open source under an allowed licence. Check the
   weights' own terms: many models with MIT code ship weights trained on research-only data.
2. Add a `ModelInfo` to `MODELS` (its task and authors; pin the official file's SHA-256 and size),
   and credit it in the root `ATTRIBUTION.md`.
3. Implement `MaskModel` for it and add its loader to `load`.
4. Test it against a reference implementation; Settings, the `roto.model.*` commands and the
   tools pick it up from the registry.
