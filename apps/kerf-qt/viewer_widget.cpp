#include "viewer_widget.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QCursor>
#include <QFontMetricsF>
#include <QKeyEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPainterPathStroker>
#include <QPolygonF>

#include <algorithm>
#include <array>
#include <cmath>
#include <initializer_list>

namespace kerf {

namespace {

constexpr double kHandleSize = 7.0;
// How near the pointer must be to a handle, or a path, to take it.
constexpr double kHandleReach = 6.0;

QColor maskColor(bool selected) { return selected ? editorTheme().maskOutline : editorTheme().maskOutlineIdle; }

// An outline over the picture: a dark rule, then a white line on it, which
// reads on any picture; `dashed` for a path only.
void strokeOverPicture(QPainter &p, const QPainterPath &path, bool selected, bool dashed) {
    const EditorTheme &theme = editorTheme();
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(theme.maskShadow, 3.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    p.drawPath(path);
    QPen pen(maskColor(selected), 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin);
    if (dashed) pen.setStyle(Qt::DashLine);
    p.setPen(pen);
    p.drawPath(path);
}

// A mask's name on a tag just above `anchor` (the top left of its outline).
void paintMaskTag(QPainter &p, const QPointF &anchor, const QString &name, bool selected) {
    const EditorTheme &theme = editorTheme();
    QFont font = p.font();
    font.setPixelSize(11);
    font.setWeight(QFont::DemiBold);
    const QFontMetricsF metrics(font);
    const QSizeF size(metrics.horizontalAdvance(name) + 10.0, metrics.height() + 4.0);
    // Clear of the top-left handle, by 4 points.
    const QRectF tag(QPointF(anchor.x() - kHandleSize / 2.0, anchor.y() - kHandleSize / 2.0 - 4.0 - size.height()), size);
    p.setPen(Qt::NoPen);
    p.setBrush(selected ? theme.maskTagSelected : theme.maskTag);
    p.drawRoundedRect(tag, 4, 4);
    p.setFont(font);
    p.setPen(theme.maskText);
    p.drawText(tag, Qt::AlignCenter, name);
}

// Fractions of the picture are kept to four places, under half a pixel of
// a 4K picture, so the document stays readable.
double fraction(double value) { return std::round(value * 10000.0) / 10000.0; }

QJsonArray fractions(std::initializer_list<double> values) {
    QJsonArray out;
    for (double value : values) out.append(fraction(value));
    return out;
}

QPointF pointOf(const QJsonValue &value) {
    const QJsonArray point = value.toArray();
    return {point.at(0).toDouble(), point.at(1).toDouble()};
}

QJsonArray pointsOf(const QVector<QPointF> &points) {
    QJsonArray out;
    for (const QPointF &point : points) out.append(fractions({point.x(), point.y()}));
    return out;
}

std::array<double, 4> fourOf(const QJsonObject &mask, const QString &key) {
    const QJsonArray values = mask.value(key).toArray();
    std::array<double, 4> out{};
    for (int index = 0; index < 4 && index < values.size(); ++index) out[index] = values.at(index).toDouble();
    return out;
}

// The outline of a mask as an operation carries it.
QJsonObject outlineOf(const QJsonObject &mask) {
    for (const QString &key : {QStringLiteral("rect"), QStringLiteral("ellipse")})
        if (mask.contains(key)) return QJsonObject{{key, mask.value(key)}};
    return QJsonObject{{QStringLiteral("points"), mask.value(QStringLiteral("points"))},
                       {QStringLiteral("closed"), mask.value(QStringLiteral("closed")).toBool(true)}};
}

QJsonObject withOutline(QJsonObject mask, const QJsonObject &outline) {
    for (const QString &key : {QStringLiteral("rect"), QStringLiteral("ellipse"), QStringLiteral("points"), QStringLiteral("closed")})
        mask.remove(key);
    for (auto it = outline.begin(); it != outline.end(); ++it) mask.insert(it.key(), it.value());
    return mask;
}

// A rectangle's corners clockwise from its top left, an ellipse's left,
// right, top, and bottom, or a path's points.
QVector<QPointF> handlesOf(const QJsonObject &mask) {
    if (mask.contains(QStringLiteral("rect"))) {
        const auto [l, t, r, b] = fourOf(mask, QStringLiteral("rect"));
        return {{l, t}, {r, t}, {r, b}, {l, b}};
    }
    if (mask.contains(QStringLiteral("ellipse"))) {
        const auto [x, y, rx, ry] = fourOf(mask, QStringLiteral("ellipse"));
        return {{x - rx, y}, {x + rx, y}, {x, y - ry}, {x, y + ry}};
    }
    QVector<QPointF> points;
    for (const QJsonValue &point : mask.value(QStringLiteral("points")).toArray()) points.append(pointOf(point));
    return points;
}

// `mask` with its handle `index` dragged to `to`: a rectangle keeps the
// opposite corner, an ellipse its center.
QJsonObject withHandle(const QJsonObject &mask, int index, const QPointF &to) {
    if (mask.contains(QStringLiteral("rect"))) {
        const QRectF rect = QRectF(to, handlesOf(mask).value((index + 2) % 4)).normalized();
        return withOutline(mask, {{QStringLiteral("rect"), fractions({rect.left(), rect.top(), rect.right(), rect.bottom()})}});
    }
    if (mask.contains(QStringLiteral("ellipse"))) {
        auto [x, y, rx, ry] = fourOf(mask, QStringLiteral("ellipse"));
        if (index < 2) rx = std::abs(to.x() - x);
        else ry = std::abs(to.y() - y);
        return withOutline(mask, {{QStringLiteral("ellipse"), fractions({x, y, rx, ry})}});
    }
    QVector<QPointF> points = handlesOf(mask);
    if (index >= 0 && index < points.size()) points[index] = to;
    QJsonObject outline = outlineOf(mask);
    outline.insert(QStringLiteral("points"), pointsOf(points));
    return withOutline(mask, outline);
}

QJsonObject movedBy(const QJsonObject &mask, const QPointF &by) {
    if (mask.contains(QStringLiteral("rect"))) {
        const auto [l, t, r, b] = fourOf(mask, QStringLiteral("rect"));
        return withOutline(mask, {{QStringLiteral("rect"), fractions({l + by.x(), t + by.y(), r + by.x(), b + by.y()})}});
    }
    if (mask.contains(QStringLiteral("ellipse"))) {
        const auto [x, y, rx, ry] = fourOf(mask, QStringLiteral("ellipse"));
        return withOutline(mask, {{QStringLiteral("ellipse"), fractions({x + by.x(), y + by.y(), rx, ry})}});
    }
    QVector<QPointF> points = handlesOf(mask);
    for (QPointF &point : points) point += by;
    QJsonObject outline = outlineOf(mask);
    outline.insert(QStringLiteral("points"), pointsOf(points));
    return withOutline(mask, outline);
}

} // namespace

ViewerWidget::ViewerWidget(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("viewer"));
    setMinimumSize(320, 180);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMouseTracking(true);
}

void ViewerWidget::setFrame(const QImage &frame, double time) {
    m_frame = frame;
    m_time = time;
    m_scaled = QImage();
    update();
}

void ViewerWidget::clearFrame() {
    m_frame = QImage();
    m_scaled = QImage();
    update();
}

void ViewerWidget::setLiveScrub(bool live) {
    if (m_liveScrub == live) return;
    m_liveScrub = live;
    m_scaled = QImage();
    update();
}

void ViewerWidget::setCaption(const QString &caption) {
    m_caption = caption;
    update();
}

void ViewerWidget::setProjectInfo(double width, double height) {
    m_width = width;
    m_height = height;
    update();
}

QRectF ViewerWidget::videoRect() const {
    const QRectF target = QRectF(rect()).adjusted(8, 8, -8, -8);
    double aspect = 16.0 / 9.0;
    if (m_width > 0 && m_height > 0) aspect = m_width / m_height;
    else if (!m_frame.isNull()) aspect = static_cast<double>(m_frame.width()) / m_frame.height();
    QSizeF canvasSize(target.width(), target.width() / aspect);
    if (canvasSize.height() > target.height())
        canvasSize = QSizeF(target.height() * aspect, target.height());
    QRectF canvas(target.center() - QPointF(canvasSize.width() / 2, canvasSize.height() / 2), canvasSize);
    if (canvasSize.width() < target.width() - 1 && neighbourOnTrailingEdge()) canvas.moveRight(target.right());
    return canvas;
}

void ViewerWidget::setTrailingNeighbour(QWidget *neighbour) {
    m_trailingNeighbour = neighbour;
    update();
}

bool ViewerWidget::neighbourOnTrailingEdge() const {
    const QWidget *neighbour = m_trailingNeighbour.data();
    if (!neighbour || !neighbour->isVisible() || !isVisible() || neighbour->window() != window()) return false;
    const QRect beside(mapFromGlobal(neighbour->mapToGlobal(QPoint())), neighbour->size());
    // Past the edge by no more than the viewer's frame and a splitter.
    constexpr int kGap = 16;
    const int middle = height() / 2;
    return beside.left() >= width() - 1 && beside.left() <= width() + kGap && beside.top() <= middle && beside.bottom() >= middle;
}

QRectF ViewerWidget::captionRect() const {
    const QRectF canvas = videoRect();
    const double horizontalMargin = std::max(12.0, canvas.width() * 0.08);
    const double bottomMargin = std::max(12.0, canvas.height() * 0.08);
    const double captionHeight = std::clamp(canvas.height() * 0.12, 34.0, 64.0);
    return QRectF(canvas.left() + horizontalMargin,
                  canvas.bottom() - bottomMargin - captionHeight,
                  std::max(1.0, canvas.width() - horizontalMargin * 2.0), captionHeight);
}

void ViewerWidget::setMasks(const QString &clip, const QJsonArray &masks, const QTransform &placement) {
    // A drag in progress keeps its outline until it is let go.
    if (m_dragHandle != -1 && clip == m_maskClip) {
        m_placement = placement;
        return;
    }
    if (clip == m_maskClip && masks == m_masks && placement == m_placement) return;
    if (clip != m_maskClip) {
        cancelDrawing();
        m_selectedMask.clear();
    }
    m_maskClip = clip;
    m_masks = masks;
    m_placement = placement;
    if (clip.isEmpty()) setMaskTool(MaskTool::None);
    // The selection outlives a snapshot that lacks it, as one read before
    // a new mask was added does.
    update();
}

void ViewerWidget::setMaskTool(MaskTool tool) {
    if (m_maskClip.isEmpty()) tool = MaskTool::None;
    if (tool == m_maskTool) return;
    cancelDrawing();
    m_maskTool = tool;
    if (tool != MaskTool::None) setFocus(Qt::OtherFocusReason);
    updateCursor(mapFromGlobal(QCursor::pos()));
    update();
    emit maskToolChanged(tool);
}

void ViewerWidget::selectMask(const QString &name) {
    const QString selected = maskIndex(name) < 0 ? QString() : name;
    if (selected == m_selectedMask) return;
    m_selectedMask = selected;
    update();
}

bool ViewerWidget::editsMasks() const {
    return !m_maskClip.isEmpty() && (m_maskTool != MaskTool::None || !m_masks.isEmpty());
}

QTransform ViewerWidget::pictureToWidget() const {
    const QRectF canvas = videoRect();
    const double canvasWidth = m_width > 0 ? m_width : m_frame.isNull() ? canvas.width() : m_frame.width();
    const double scale = canvas.width() / std::max(1.0, canvasWidth);
    QTransform view;
    view.translate(canvas.left(), canvas.top());
    view.scale(scale, scale);
    return m_placement * view;
}

QPointF ViewerWidget::picturePoint(const QPointF &widget) const {
    bool invertible = false;
    const QTransform toPicture = pictureToWidget().inverted(&invertible);
    return invertible ? toPicture.map(widget) : QPointF();
}

int ViewerWidget::maskIndex(const QString &name) const {
    if (name.isEmpty()) return -1;
    for (int index = 0; index < m_masks.size(); ++index)
        if (m_masks.at(index).toObject().value(QStringLiteral("name")).toString() == name) return index;
    return -1;
}

int ViewerWidget::handleAt(const QPointF &widget) const {
    const int index = maskIndex(m_selectedMask);
    if (index < 0) return -1;
    const QTransform toWidget = pictureToWidget();
    const QVector<QPointF> handles = handlesOf(m_masks.at(index).toObject());
    for (int handle = handles.size() - 1; handle >= 0; --handle) {
        const QPointF delta = toWidget.map(handles.at(handle)) - widget;
        if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) return handle;
    }
    return -1;
}

QString ViewerWidget::maskAt(const QPointF &widget) const {
    const QTransform toWidget = pictureToWidget();
    QPainterPathStroker stroker;
    stroker.setWidth(kHandleReach * 2.0);
    const auto hit = [&](int index) {
        const QJsonObject mask = m_masks.at(index).toObject();
        const QPainterPath path = toWidget.map(maskPath(mask));
        const bool closed = !mask.contains(QStringLiteral("points")) || mask.value(QStringLiteral("closed")).toBool(true);
        return (closed && path.contains(widget)) || stroker.createStroke(path).contains(widget);
    };
    const int selected = maskIndex(m_selectedMask);
    if (selected >= 0 && hit(selected)) return m_selectedMask;
    for (int index = m_masks.size() - 1; index >= 0; --index)
        if (hit(index)) return m_masks.at(index).toObject().value(QStringLiteral("name")).toString();
    return {};
}

void ViewerWidget::updateCursor(const QPointF &widget) {
    if (m_maskTool != MaskTool::None) setCursor(Qt::CrossCursor);
    else if (!editsMasks()) unsetCursor();
    else if (m_dragHandle == kMoveMask) setCursor(Qt::ClosedHandCursor);
    else if (m_dragHandle >= 0 || handleAt(widget) >= 0) setCursor(Qt::PointingHandCursor);
    else if (!maskAt(widget).isEmpty()) setCursor(Qt::OpenHandCursor);
    else unsetCursor();
}

void ViewerWidget::addMask(QJsonObject mask) {
    QString name;
    for (int n = 1; name.isEmpty() || maskIndex(name) >= 0; ++n) name = QStringLiteral("k%1").arg(n);
    QJsonObject op = outlineOf(mask);
    op.insert(QStringLiteral("op"), QStringLiteral("mask-add"));
    op.insert(QStringLiteral("clip"), m_maskClip);
    op.insert(QStringLiteral("name"), name);
    // Shown at once; the next snapshot replaces it.
    mask.insert(QStringLiteral("name"), name);
    mask.insert(QStringLiteral("mode"), QStringLiteral("add"));
    mask.insert(QStringLiteral("inverted"), false);
    m_masks.append(mask);
    m_selectedMask = name;
    setMaskTool(MaskTool::None);
    update();
    emit maskSelected(name);
    emit editRequested(QJsonArray{op});
}

void ViewerWidget::finishPen(bool closed) {
    if (m_penPoints.size() < (closed ? 3 : 2)) return;
    const QJsonArray points = pointsOf(m_penPoints);
    m_penPoints.clear();
    addMask(QJsonObject{{QStringLiteral("points"), points}, {QStringLiteral("closed"), closed}});
}

void ViewerWidget::cancelDrawing() {
    if (!m_drawing && m_penPoints.isEmpty()) return;
    m_drawing = false;
    m_penPoints.clear();
    update();
}

void ViewerWidget::removeSelectedMask() {
    const int index = maskIndex(m_selectedMask);
    if (index < 0) return;
    const QString name = m_selectedMask;
    m_masks.removeAt(index);
    m_selectedMask.clear();
    update();
    emit maskSelected(QString());
    emit editRequested(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("mask-remove")},
                                              {QStringLiteral("clip"), m_maskClip},
                                              {QStringLiteral("name"), name}}});
}

bool ViewerWidget::event(QEvent *event) {
    // Keys the masks take ahead of the editor's shortcuts.
    if (event->type() == QEvent::ShortcutOverride) {
        const int key = static_cast<QKeyEvent *>(event)->key();
        const bool drawing = m_drawing || !m_penPoints.isEmpty();
        const bool takes = (key == Qt::Key_Escape && (drawing || m_maskTool != MaskTool::None)) ||
                           ((key == Qt::Key_Return || key == Qt::Key_Enter) && m_penPoints.size() >= 2) ||
                           ((key == Qt::Key_Delete || key == Qt::Key_Backspace) &&
                            (drawing || maskIndex(m_selectedMask) >= 0));
        if (takes) {
            event->accept();
            return true;
        }
    }
    return QWidget::event(event);
}

void ViewerWidget::keyPressEvent(QKeyEvent *event) {
    switch (event->key()) {
    case Qt::Key_Escape:
        if (m_drawing || !m_penPoints.isEmpty()) cancelDrawing();
        else setMaskTool(MaskTool::None);
        return;
    case Qt::Key_Return:
    case Qt::Key_Enter:
        // An open path, as Stroke draws along.
        finishPen(false);
        return;
    case Qt::Key_Delete:
    case Qt::Key_Backspace:
        if (!m_penPoints.isEmpty()) {
            m_penPoints.removeLast();
            update();
        } else if (!m_drawing) {
            removeSelectedMask();
        }
        return;
    default: QWidget::keyPressEvent(event);
    }
}

void ViewerWidget::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return;
    const QPointF position = event->position();
    if (!editsMasks()) {
        emit seekRequested(fractionAt(position.x()));
        return;
    }
    setFocus(Qt::MouseFocusReason);
    const QPointF at = picturePoint(position);
    if (m_maskTool == MaskTool::Pen) {
        // A click on the first point closes the outline.
        if (m_penPoints.size() >= 3) {
            const QPointF delta = pictureToWidget().map(m_penPoints.first()) - position;
            if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) {
                finishPen(true);
                return;
            }
        }
        m_penPoints.append(at);
        m_pointer = position;
        update();
        return;
    }
    if (m_maskTool != MaskTool::None) {
        m_drawing = true;
        m_drawStart = m_drawEnd = at;
        update();
        return;
    }
    const int handle = handleAt(position);
    const QString name = handle >= 0 ? m_selectedMask : maskAt(position);
    if (name != m_selectedMask) {
        m_selectedMask = name;
        update();
        emit maskSelected(name);
    }
    if (name.isEmpty()) return;
    m_dragHandle = handle >= 0 ? handle : kMoveMask;
    m_dragFrom = at;
    m_dragOriginal = m_masks.at(maskIndex(name)).toObject();
    updateCursor(position);
}

void ViewerWidget::mouseMoveEvent(QMouseEvent *event) {
    const QPointF position = event->position();
    if (m_drawing) {
        m_drawEnd = picturePoint(position);
        update();
        return;
    }
    if (m_dragHandle != -1) {
        const QPointF at = picturePoint(position);
        const int index = maskIndex(m_dragOriginal.value(QStringLiteral("name")).toString());
        if (index >= 0)
            m_masks[index] = m_dragHandle == kMoveMask ? movedBy(m_dragOriginal, at - m_dragFrom)
                                                       : withHandle(m_dragOriginal, m_dragHandle, at);
        update();
        return;
    }
    if (editsMasks()) {
        // The picture holds still while masks are edited.
        if (m_maskTool == MaskTool::Pen && !m_penPoints.isEmpty()) {
            m_pointer = position;
            update();
        }
        updateCursor(position);
        return;
    }
    const double fraction = fractionAt(position.x());
    if (m_skimming && std::abs(fraction - m_skimFraction) < 0.0005) return;
    m_skimming = true;
    m_skimFraction = fraction;
    emit scrubbed(fraction);
}

void ViewerWidget::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return;
    if (m_drawing) {
        m_drawing = false;
        m_drawEnd = picturePoint(event->position());
        const QTransform toWidget = pictureToWidget();
        const QRectF drawn = QRectF(toWidget.map(m_drawStart), toWidget.map(m_drawEnd)).normalized();
        update();
        // A click, not a drag, draws nothing.
        if (drawn.width() < 4.0 && drawn.height() < 4.0) return;
        const QRectF box = QRectF(m_drawStart, m_drawEnd).normalized();
        if (m_maskTool == MaskTool::Ellipse)
            addMask(QJsonObject{{QStringLiteral("ellipse"), fractions({box.center().x(), box.center().y(),
                                                                       box.width() / 2.0, box.height() / 2.0})}});
        else
            addMask(QJsonObject{{QStringLiteral("rect"), fractions({box.left(), box.top(), box.right(), box.bottom()})}});
        return;
    }
    if (m_dragHandle == -1) return;
    m_dragHandle = -1;
    const QString name = m_dragOriginal.value(QStringLiteral("name")).toString();
    const QJsonObject original = m_dragOriginal;
    m_dragOriginal = {};
    updateCursor(event->position());
    const int index = maskIndex(name);
    if (index < 0) return;
    const QJsonObject outline = outlineOf(m_masks.at(index).toObject());
    if (outline == outlineOf(original)) return;
    QJsonObject op = outline;
    op.insert(QStringLiteral("op"), QStringLiteral("mask-set"));
    op.insert(QStringLiteral("clip"), m_maskClip);
    op.insert(QStringLiteral("name"), name);
    emit editRequested(QJsonArray{op});
}

void ViewerWidget::leaveEvent(QEvent *) {
    if (!m_skimming) return;
    m_skimming = false;
    emit scrubEnded();
}

void ViewerWidget::paintMasks(QPainter &p) {
    if (m_maskClip.isEmpty()) return;
    const QTransform toWidget = pictureToWidget();
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setBrush(Qt::NoBrush);
    // Outlines are white over a dark rule, never the accent, so they read
    // on a picture in the accent's own hue; a path only (mode none) is dashed.
    const auto outline = [&p](const QPainterPath &path, bool selected, bool pathOnly) {
        strokeOverPicture(p, path, selected, pathOnly);
    };
    // Handles are the accent with a white edge; the pen's first point turns
    // white when a click there would close the outline.
    const auto handle = [&p](const QPointF &center, bool closes) {
        const EditorTheme &theme = editorTheme();
        p.setPen(QPen(closes ? theme.maskHandleFill : theme.maskHandleBorder, 1.0));
        p.setBrush(closes ? theme.maskHandleBorder : theme.maskHandleFill);
        p.drawRect(QRectF(center - QPointF(kHandleSize / 2.0, kHandleSize / 2.0), QSizeF(kHandleSize, kHandleSize)));
        p.setBrush(Qt::NoBrush);
    };
    for (const QJsonValue &value : m_masks) {
        const QJsonObject mask = value.toObject();
        const QString name = mask.value(QStringLiteral("name")).toString();
        const bool selected = name == m_selectedMask;
        const QPainterPath path = toWidget.map(maskPath(mask));
        outline(path, selected, mask.value(QStringLiteral("mode")).toString() == QStringLiteral("none"));
        // The name on a tag over the outline's top left; the selected
        // mask's tag is the accent.
        if (!name.isEmpty()) paintMaskTag(p, path.boundingRect().topLeft(), name, selected);
        if (!selected) continue;
        for (const QPointF &point : handlesOf(mask)) handle(toWidget.map(point), false);
    }
    if (m_drawing) {
        const QRectF box = QRectF(m_drawStart, m_drawEnd).normalized();
        QPainterPath path;
        if (m_maskTool == MaskTool::Ellipse) path.addEllipse(box);
        else path.addRect(box);
        outline(toWidget.map(path), true, false);
    }
    if (!m_penPoints.isEmpty()) {
        QPainterPath path(toWidget.map(m_penPoints.first()));
        for (int index = 1; index < m_penPoints.size(); ++index) path.lineTo(toWidget.map(m_penPoints.at(index)));
        outline(path, true, false);
        QPainterPath rubber(toWidget.map(m_penPoints.last()));
        rubber.lineTo(m_pointer);
        strokeOverPicture(p, rubber, false, true);
        const QPointF first = toWidget.map(m_penPoints.first());
        const QPointF reach = first - m_pointer;
        // The first point fills when a click there would close the outline.
        const bool closes = m_penPoints.size() >= 3 && std::abs(reach.x()) <= kHandleReach && std::abs(reach.y()) <= kHandleReach;
        for (int index = 0; index < m_penPoints.size(); ++index) handle(toWidget.map(m_penPoints.at(index)), index == 0 && closes);
    }
    p.restore();
}

void ViewerWidget::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.fillRect(rect(), theme.viewerSurround);
    const QRectF canvas = videoRect();
    QPainterPath canvasPath;
    canvasPath.addRoundedRect(canvas.adjusted(0.5, 0.5, -0.5, -0.5), 2, 2);
    p.setPen(QPen(theme.viewerCanvas, 1));
    p.setBrush(theme.viewerCanvas);
    p.drawPath(canvasPath);
    if (!m_frame.isNull()) {
        p.save();
        p.setClipPath(canvasPath);
        const qreal dpr = m_liveScrub ? 1.0 : std::max(1.0, devicePixelRatioF());
        const QSize target(std::max(1, static_cast<int>(std::round(canvas.width() * dpr))),
                           std::max(1, static_cast<int>(std::round(canvas.height() * dpr))));
        if (m_scaled.isNull() || m_scaled.size() != target || m_scaledFast != m_liveScrub) {
            m_scaled = m_frame.scaled(target, Qt::KeepAspectRatio,
                                      m_liveScrub ? Qt::FastTransformation : Qt::SmoothTransformation);
            m_scaled.setDevicePixelRatio(dpr);
            m_scaledFast = m_liveScrub;
        }
        const QPointF topLeft(canvas.center().x() - m_scaled.width() / dpr / 2.0,
                              canvas.center().y() - m_scaled.height() / dpr / 2.0);
        p.drawImage(topLeft, m_scaled);
        p.restore();
    } else {
        const QPointF center = canvas.center() - QPointF(0, 22);
        p.setPen(Qt::NoPen);
        p.setBrush(theme.viewerGlyphFill);
        p.drawEllipse(center, 20, 20);
        p.setPen(theme.viewerGlyphRing);
        p.setBrush(Qt::NoBrush);
        p.drawEllipse(center, 20, 20);
        p.setPen(Qt::NoPen);
        p.setBrush(theme.viewerGlyph);
        p.drawPolygon(QPolygonF{center + QPointF(-5, -8), center + QPointF(9, 0), center + QPointF(-5, 8)});
        p.setPen(theme.viewerText);
        p.drawText(QRectF(canvas.left(), center.y() + 32, canvas.width(), 20), Qt::AlignCenter,
                   QStringLiteral("Your viewer"));
        p.setPen(theme.viewerTextDim);
        p.drawText(QRectF(canvas.left(), center.y() + 52, canvas.width(), 20), Qt::AlignCenter,
                   QStringLiteral("The playhead frame shows here."));
    }
    if (!m_caption.isEmpty()) {
        const QRectF caption = captionRect();
        p.setPen(Qt::NoPen);
        p.setBrush(theme.viewerCaption);
        p.drawRoundedRect(caption, 4, 4);
        p.setPen(theme.viewerCaptionBorder);
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(caption, 4, 4);
        p.setPen(theme.viewerCaptionText);
        p.drawText(caption.adjusted(10, 4, -10, -4), Qt::AlignCenter | Qt::TextWordWrap, m_caption);
    }
    paintMasks(p);
}

double ViewerWidget::fractionAt(int x) const {
    return std::clamp(static_cast<double>(x) / std::max(1, width()), 0.0, 1.0);
}

RegionPicker::RegionPicker(const QImage &frame, const QRectF &region, QWidget *parent)
    : QWidget(parent), m_frame(frame), m_region(region) {
    setMinimumSize(480, 270);
    setMouseTracking(true);
}

void RegionPicker::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.fillRect(rect(), theme.viewerSurround);
    const QRectF image = imageRect();
    painter.drawImage(image, m_frame);
    if (m_region.isEmpty()) return;
    const QRectF selected(image.x() + m_region.x() * image.width(),
                          image.y() + m_region.y() * image.height(),
                          m_region.width() * image.width(), m_region.height() * image.height());
    // White over a dark rule, as the masks' outlines, so it reads on any
    // picture.
    painter.setRenderHint(QPainter::Antialiasing);
    painter.fillRect(selected, theme.maskFill);
    QPainterPath frame;
    frame.addRect(selected);
    strokeOverPicture(painter, frame, true, false);
}

void RegionPicker::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !imageRect().contains(event->position())) return;
    m_start = normalizedPoint(event->position());
    m_dragging = true;
    m_region = QRectF();
    update();
}

void RegionPicker::mouseMoveEvent(QMouseEvent *event) {
    if (!m_dragging) return;
    const QPointF end = normalizedPoint(event->position());
    m_region = QRectF(m_start, end).normalized();
    update();
}

void RegionPicker::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !m_dragging) return;
    m_dragging = false;
    const QPointF end = normalizedPoint(event->position());
    m_region = QRectF(m_start, end).normalized();
    if (m_region.width() < 0.01 || m_region.height() < 0.01) m_region = QRectF();
    update();
}

QRectF RegionPicker::imageRect() const {
    const QSizeF fitted = m_frame.size().scaled(size(), Qt::KeepAspectRatio);
    return QRectF((width() - fitted.width()) / 2.0, (height() - fitted.height()) / 2.0,
                  fitted.width(), fitted.height());
}

QPointF RegionPicker::normalizedPoint(const QPointF &point) const {
    const QRectF image = imageRect();
    return QPointF(std::clamp((point.x() - image.x()) / image.width(), 0.0, 1.0),
                   std::clamp((point.y() - image.y()) / image.height(), 0.0, 1.0));
}

} // namespace kerf
