# Custom cam/chat/content training layout

Copy [`dataset.template.yaml`](./dataset.template.yaml), replace `path`, and
lay out YOLO detection data like this:

```text
dataset/
  images/train/...
  images/val/...
  labels/train/*.txt
  labels/val/*.txt
```

Each label row is `class_id center_x center_y width height`, normalized to
`0..1`. Only class `0: cam`, class `1: chat`, and class `2: content` are
accepted. The training script validates every discovered train/validation label
before it imports Ultralytics.

## Validate or train

```sh
python3 packages/vision/python/train_yolo26s.py \
  --data /path/to/dataset.yaml \
  --validate-only

python3 packages/vision/python/train_yolo26s.py \
  --data /path/to/dataset.yaml \
  --base yolo26s.pt \
  --output /models/cut-yolo26s-cam-chat-content.pt \
  --epochs 100 \
  --imgsz 640 \
  --export-format onnx
```

The default bare `yolo26s.pt` is the official Ultralytics base and may be
downloaded by Ultralytics. These starter files never install packages or
download data. For a no-download run, provide an existing local base and
`--offline`:

```sh
python3 packages/vision/python/train_yolo26s.py \
  --data /path/to/dataset.yaml \
  --base /models/yolo26s-base.pt \
  --offline \
  --output /models/cut-yolo26s-cam-chat-content.pt
```

After training, stdout includes `weightsPath` and a config snippet ready for
`@cut/vision`, with the required three-target label map. Install `ultralytics`
and any accelerator/runtime dependencies separately in the Python environment;
this repository does not auto-install them.

## Data and licensing

Use images and annotations only when you have permission to process and train
on them. Confirm source-image, annotation, and any pretrained-base licenses
permit derivative model training and redistribution. Face imagery can contain
personal data: obtain appropriate consent, minimize retention, protect the
dataset/checkpoints, and follow the laws and policies applicable to your use.
Do not commit datasets, checkpoints, exported artifacts, or credentials to this
repository.
