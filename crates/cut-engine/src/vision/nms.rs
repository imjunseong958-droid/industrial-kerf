//! Class-aware non-maximum suppression.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NmsBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub confidence: f32,
    pub class_id: usize,
}

#[inline]
pub fn iou(a: &NmsBox, b: &NmsBox) -> f32 {
    let inter_x1 = a.x1.max(b.x1);
    let inter_y1 = a.y1.max(b.y1);
    let inter_x2 = a.x2.min(b.x2);
    let inter_y2 = a.y2.min(b.y2);
    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter = inter_w * inter_h;
    if inter <= 0.0 {
        return 0.0;
    }
    let area_a = (a.x2 - a.x1).max(0.0) * (a.y2 - a.y1).max(0.0);
    let area_b = (b.x2 - b.x1).max(0.0) * (b.y2 - b.y1).max(0.0);
    let union = area_a + area_b - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Class-aware greedy NMS. Returns indices into `boxes` in descending confidence order.
pub fn nms(boxes: &[NmsBox], iou_threshold: f32) -> Vec<usize> {
    if boxes.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by(|&a, &b| {
        boxes[b]
            .confidence
            .partial_cmp(&boxes[a].confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut keep = Vec::with_capacity(order.len());
    let mut suppressed = vec![false; boxes.len()];
    for &index in &order {
        if suppressed[index] {
            continue;
        }
        keep.push(index);
        let current = boxes[index];
        for (other_index, other_suppressed) in suppressed.iter_mut().enumerate() {
            if other_index == index || *other_suppressed {
                continue;
            }
            let other = boxes[other_index];
            if other.class_id == current.class_id && iou(&current, &other) > iou_threshold {
                *other_suppressed = true;
            }
        }
    }
    keep
}

/// Class-aware NMS returning the surviving boxes.
pub fn nms_boxes(boxes: Vec<NmsBox>, iou_threshold: f32) -> Vec<NmsBox> {
    let keep = nms(&boxes, iou_threshold);
    keep.into_iter().map(|index| boxes[index]).collect()
}
