# Attribution

Every non-code asset in the EffectCraft crates Kerf carries, with its author, source and licence,
taken from EffectCraft's own index. Each file listed here also has a `<file>.attribution` sidecar
with full details.

## Asset files

| Asset | Author | Source | Licence |
|---|---|---|---|
| `assets/fonts/Inter-Regular.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Medium.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-SemiBold.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Bold.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/Inter-Italic.ttf` | The Inter Project Authors | https://github.com/rsms/inter | OFL-1.1 (`assets/fonts/OFL-Inter.txt`) |
| `assets/fonts/NotoSerif-Regular.ttf` | The Noto Project Authors | https://github.com/notofonts/latin-greek-cyrillic | OFL-1.1 (`assets/fonts/OFL-NotoSerif.txt`) |
| `assets/fonts/JetBrainsMono-Regular.ttf` | The JetBrains Mono Project Authors | https://github.com/JetBrains/JetBrainsMono | OFL-1.1 (`assets/fonts/OFL-JetBrainsMono.txt`) |
| `crates/model/tests/fixtures/quad.gltf` | EffectCraft contributors | Original work: glTF 2.0 test fixture (textured quad, node hierarchy, translation animation; data URIs) | MIT OR Apache-2.0 |
| `crates/model/tests/fixtures/quad.glb` | EffectCraft contributors | Original work: binary glTF test fixture (the same quad with its texture in the BIN chunk) | MIT OR Apache-2.0 |
| `crates/model/tests/fixtures/cube.obj` | EffectCraft contributors | Original work: Wavefront OBJ test fixture (unit cube, two materials) | MIT OR Apache-2.0 |
| `crates/model/tests/fixtures/cube.mtl` | EffectCraft contributors | Original work: MTL test fixture (materials of cube.obj) | MIT OR Apache-2.0 |

## Trained models (not bundled)

The repository contains no model weights, and Kerf does not download them. `crates/segment` can
read these models with its own Rust implementation and is not affiliated with or endorsed by their
authors.

| Model | Authors | Source | Licence |
|---|---|---|---|
| MobileSAM (`mobile_sam.pt`) | Chaoning Zhang et al., building on Meta AI's Segment Anything and Microsoft's TinyViT | https://github.com/ChaoningZhang/MobileSAM | Apache-2.0 |
| MediaPipe Face Landmarker (`face_landmarker.task`: BlazeFace short-range detector, Face Mesh V2, blendshapes) | Google LLC | https://developers.google.com/edge/mediapipe/solutions/vision/face_landmarker (model cards: https://storage.googleapis.com/mediapipe-assets/MediaPipe%20BlazeFace%20Model%20Card%20(Short%20Range).pdf, https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Face%20Mesh%20V2.pdf) | Apache-2.0 |

## Formats and parameters from other open-source projects

Written in Rust for EffectCraft; no code is copied. They follow published specifications and
configuration from these Apache-2.0 projects:

- `crates/segment/src/tflite.rs` reads TensorFlow Lite files using the field layout of TensorFlow
  Lite's `schema.fbs` (The TensorFlow Authors, https://github.com/tensorflow/tensorflow,
  Apache-2.0).
- `crates/segment/src/mediapipe.rs` follows MediaPipe's published face landmarker graph: the SSD
  anchor layout, detection decoding, crop sizes and the face-mesh point indices it uses (Google
  LLC, https://github.com/google-ai-edge/mediapipe, Apache-2.0).
- `crates/segment/src/mobilesam.rs` follows the MobileSAM, TinyViT and Segment Anything
  architectures and their Apache-2.0 reference code.

