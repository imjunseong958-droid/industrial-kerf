#pragma once

// The editor's icon set, drawn in code (plan decision 4: no QtSvg).
//
// Every icon of the review-thread design (mockup-spec 16) is kept as its SVG
// markup, copied verbatim, in a table keyed by name: g-169 ... g-cdr, plus
// "playhead-head" and "checkbox-check". The retired g-isl and g-punch and the
// device preview's glyphs are left out. A small SVG
// reader turns that markup into QPainterPaths once, on first use.
//
// The design's icon convention (mockup svg.i): fill none, stroke currentColor,
// stroke width 1.5 in the icon's viewBox, round caps and joins. An icon drawn
// at 14 px uses stroke 1.7143 instead, so it still renders 1.5 px wide
// (svg.i.s). Markup that names its own stroke width or colour keeps it (the
// 24-unit gear at 2.25, the playhead head, the checkbox check).

#include "design_tokens.h"

#include <QColor>
#include <QIcon>
#include <QPainterPath>
#include <QPixmap>
#include <QPointF>
#include <QRectF>
#include <QSizeF>
#include <QString>
#include <QStringList>
#include <QStringView>

class QPainter;

namespace kerf::icons {

// The colour an icon takes in each state of the control drawing it
// (mockup-spec 0.5: ink2 at rest, ink on hover and when on, dis when disabled).
struct IconStates {
    QColor normal = tokens::color(tokens::ink2);
    QColor hover = tokens::color(tokens::ink);
    QColor on = tokens::color(tokens::ink);
    QColor disabled = tokens::color(tokens::dis);
};

// Every icon name in the table, in the order of mockup-spec 16.
QStringList names();
bool has(const QString &name);

// The icon's viewBox size: 16 x 16 for most, 24 x 24 for g-settings, 9 x 12
// for the playhead head. Empty for an unknown name.
QSizeF viewBox(const QString &name);

// The icon's geometry, every shape of it in document order, in viewBox units.
// For a stroked shape this is the centre line; strokedPath() is the area the
// stroke covers. Empty for an unknown name.
QPainterPath path(const QString &name);

// Everything the icon paints at `px` (fills, plus the stroke outlines at the
// width they are drawn at), in viewBox units.
QPainterPath strokedPath(const QString &name, qreal px = 16);

// The stroke width, in viewBox units, of a shape that does not name its own
// when the icon is drawn at `px`: 1.5, or 1.7143 at 14 px.
qreal defaultStrokeWidth(qreal px);

// The square (or, for a non-square viewBox, the box of that aspect whose
// longer side is `px`) an icon drawn at `px` takes, centred in `rect`.
QRectF iconBox(const QString &name, const QRectF &rect, qreal px = 16);

// Draws the icon at `px` (the longer side of its box), centred in `rect`, with
// currentColor = `color`. Fixed colours in the markup are kept. An unknown
// name draws nothing.
void paint(QPainter &painter, const QString &name, const QRectF &rect, const QColor &color, qreal px = 16);

// A pixmap of the icon at `px`, `dpr` device pixels per point, transparent
// around it.
QPixmap pixmap(const QString &name, const QColor &color, qreal px = 16, qreal dpr = 1);

// A QIcon with the icon in each state colour, at 1x and 2x: Normal/Off is
// states.normal, Active/Off (hover) states.hover, On and Selected states.on,
// Disabled states.disabled.
QIcon icon(const QString &name, const IconStates &states = {}, qreal px = 16);

// Problems the reader met while building the table (empty when every icon
// parsed). Each entry names the icon.
QStringList problems();

// --------------------------------------------------------------- the SVG reader
// The parts of SVG the table uses, public so the tests can reach them.
namespace svg {

// Parses SVG path data (M/m L/l H/h V/v C/c S/s Q/q T/t A/a Z/z, with implicit
// repeats and compact numbers such as ".5.5" or "1-1"). Returns the path read
// up to the first error; `error`, when given, is set to a description of it,
// or cleared.
QPainterPath parsePathData(QStringView data, QString *error = nullptr);

// Appends an SVG elliptical arc from `from` (the path's current point) to
// `to`, converted from endpoint to centre form (SVG 1.1 F.6.5) and drawn as
// cubic Béziers of at most 90 degrees each. Radii too small for the chord are
// scaled up, zero radii give a line, and equal endpoints add nothing.
void arcTo(QPainterPath &path, QPointF from, qreal rx, qreal ry, qreal xAxisRotationDegrees, bool largeArc,
           bool sweep, QPointF to);

// An SVG <rect>: starts at (x + rx, y) and runs clockwise, as SVG draws it,
// so a dash pattern starts where the browser starts it. ry <= 0 takes rx.
QPainterPath rectPath(qreal x, qreal y, qreal width, qreal height, qreal rx = 0, qreal ry = 0);

// An SVG <ellipse> or <circle>: starts at (cx + rx, cy) and runs clockwise.
QPainterPath ellipsePath(qreal cx, qreal cy, qreal rx, qreal ry);

} // namespace svg

} // namespace kerf::icons
