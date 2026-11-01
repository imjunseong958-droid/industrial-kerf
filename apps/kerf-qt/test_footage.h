#pragma once

#include <QImage>
#include <QSize>

namespace kerf::testing {

// The tests' stand-in footage: three shots of a harbor teaser, drawn in
// code (no media files, no FFmpeg) so screenshots look like footage rather
// than a template. Each is built from layered value noise, so it has
// gradients, texture and film grain, and lays itself out for any shape
// (16:9, 9:16, 1:1) without stretching. The same shot and size always give
// the same pixels.
enum class Shot {
    HarborWide,  // A harbor at dawn: hazy hills, a low sun behind cloud, boats on rippled water.
    FisherClose, // A close-up on a dock: weathered planks and a coil of rope, warm light, soft background.
    DronePass,   // Straight down over the coast: deep water, surf, sand and patchwork fields.
};

QImage footageFrame(Shot shot, const QSize &size);

} // namespace kerf::testing
