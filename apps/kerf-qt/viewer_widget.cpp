#include "viewer_widget.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "stage_shell.h"
#include "ui/components.h"

#include <QCursor>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPainterPathStroker>
#include <QPolygonF>
#include <QVariantAnimation>

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

// A mask's name on a tag just above `anchor` (the top left of its outline),
// kept inside `bounds` (the picture): 8 points under its top edge and 4 in
// from its sides, never on a black bar.
void paintMaskTag(QPainter &p, const QPointF &anchor, const QString &name, bool selected, const QRectF &bounds) {
    const EditorTheme &theme = editorTheme();
    QFont font = p.font();
    font.setPixelSize(11);
    font.setWeight(QFont::DemiBold);
    const QFontMetricsF metrics(font);
    const QSizeF size(metrics.horizontalAdvance(name) + 10.0, metrics.height() + 4.0);
    // Clear of the top-left handle, by 4 points.
    QRectF tag(QPointF(anchor.x() - kHandleSize / 2.0, anchor.y() - kHandleSize / 2.0 - 4.0 - size.height()), size);
    if (bounds.isValid()) {
        tag.moveLeft(std::clamp(tag.left(), bounds.left() + 4.0, std::max(bounds.left() + 4.0, bounds.right() - 4.0 - size.width())));
        tag.moveTop(std::clamp(tag.top(), bounds.top() + 8.0, std::max(bounds.top() + 8.0, bounds.bottom() - 4.0 - size.height())));
    }
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

namespace {

// -1 follows the platform and reduced motion, 0 never moves, 1 always does.
int g_stageMotion = -1;

// The broadcast safe zones (mockup-spec 3.3): action safe, a 1 pt line just
// inside a rect 3.5% in from every edge, and title safe, 5% in, shown by 16 pt
// corner ticks and a 12 pt cross at the centre.
constexpr double kActionSafeInset = 0.035;
constexpr double kTitleSafeInset = 0.05;
constexpr double kSafeTick = 16.0;
constexpr double kSafeCross = 12.0;
constexpr QRgb kSafeTickColor = qRgba(255, 255, 255, tokens::alpha(0.45));

void paintSafeZones(QPainter &p, const QRectF &picture) {
    if (picture.width() < 2.0 * kSafeTick || picture.height() < 2.0 * kSafeTick) return;
    const double w = picture.width();
    const double h = picture.height();
    p.save();
    p.setRenderHint(QPainter::Antialiasing, false);
    p.setPen(Qt::NoPen);
    const QRectF a = picture.adjusted(kActionSafeInset * w, kActionSafeInset * h, -kActionSafeInset * w, -kActionSafeInset * h);
    p.setBrush(tokens::color(tokens::lineh));
    p.drawRect(QRectF(a.left(), a.top(), a.width(), 1.0));
    p.drawRect(QRectF(a.left(), a.bottom() - 1.0, a.width(), 1.0));
    p.drawRect(QRectF(a.left(), a.top() + 1.0, 1.0, a.height() - 2.0));
    p.drawRect(QRectF(a.right() - 1.0, a.top() + 1.0, 1.0, a.height() - 2.0));
    const QRectF t = picture.adjusted(kTitleSafeInset * w, kTitleSafeInset * h, -kTitleSafeInset * w, -kTitleSafeInset * h);
    p.setBrush(tokens::color(kSafeTickColor));
    const double k = kSafeTick;
    p.drawRect(QRectF(t.left(), t.top(), k, 1.0));
    p.drawRect(QRectF(t.left(), t.top() + 1.0, 1.0, k - 1.0));
    p.drawRect(QRectF(t.right() - k, t.top(), k, 1.0));
    p.drawRect(QRectF(t.right() - 1.0, t.top() + 1.0, 1.0, k - 1.0));
    p.drawRect(QRectF(t.left(), t.bottom() - 1.0, k, 1.0));
    p.drawRect(QRectF(t.left(), t.bottom() - k, 1.0, k - 1.0));
    p.drawRect(QRectF(t.right() - k, t.bottom() - 1.0, k, 1.0));
    p.drawRect(QRectF(t.right() - 1.0, t.bottom() - k, 1.0, k - 1.0));
    const QRectF c(picture.center() - QPointF(kSafeCross / 2.0, kSafeCross / 2.0), QSizeF(kSafeCross, kSafeCross));
    p.drawRect(QRectF(c.left(), c.top() + kSafeCross / 2.0, kSafeCross, 1.0));
    p.drawRect(QRectF(c.left() + kSafeCross / 2.0, c.top(), 1.0, kSafeCross));
    p.restore();
}

// The overlap box (mockup-spec 3.3) and the strokes of a drawing.
constexpr QRgb kOverlapFill = qRgba(236, 93, 97, tokens::alpha(0.32));
constexpr QRgb kStrokeUnder = qRgba(0, 0, 0, tokens::alpha(0.4));
constexpr double kStrokeAlpha = 0.9;
constexpr int kAvatar = 16;

// Fractions of the frame are kept to four places, as the masks' are.
double fourPlaces(double value) { return std::round(value * 10000.0) / 10000.0; }

// `text` broken at spaces into lines no wider than `width`, a word wider
// than that on a line of its own.
QStringList wrapped(const QString &text, const QFontMetricsF &metrics, double width) {
    QStringList lines;
    QString line;
    for (const QString &word : text.split(QLatin1Char(' '), Qt::SkipEmptyParts)) {
        const QString candidate = line.isEmpty() ? word : line + QLatin1Char(' ') + word;
        if (!line.isEmpty() && metrics.horizontalAdvance(candidate) > width) {
            lines << line;
            line = word;
        } else {
            line = candidate;
        }
    }
    if (!line.isEmpty()) lines << line;
    return lines;
}

QColor strokeColor(const QJsonObject &stroke) {
    QColor color(stroke.value(QStringLiteral("color")).toString());
    if (!color.isValid()) color = tokens::color(tokens::ink);
    color.setAlphaF(kStrokeAlpha);
    return color;
}

} // namespace

ViewerWidget::ViewerWidget(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("viewer"));
    setMinimumSize(tokens::layout::pictureColumnLeast, 180);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMouseTracking(true);
    setAttribute(Qt::WA_OpaquePaintEvent);
    m_overlapFade = new QVariantAnimation(this);
    m_overlapFade->setDuration(tokens::motion::fade);
    m_overlapFade->setEasingCurve(tokens::easing());
    connect(m_overlapFade, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_overlapLevel = value.toDouble();
        update();
    });
}

void ViewerWidget::setMotionOverride(int state) { g_stageMotion = state < 0 ? -1 : (state ? 1 : 0); }

bool ViewerWidget::motionAllowed() const {
    if (g_stageMotion >= 0) return g_stageMotion == 1;
    // Only where something shows the motion: on screen, and not asked to keep still.
    return isVisible() && QGuiApplication::platformName() != QLatin1String("offscreen") && ui::motionDuration(tokens::motion::fade) > 0;
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
    if (caption == m_caption) return;
    m_caption = caption;
    update();
}

void ViewerWidget::setProjectInfo(double width, double height) {
    m_width = width;
    m_height = height;
    m_scaled = QImage();
    placeOverlays();
    update();
}

double ViewerWidget::aspect() const {
    if (m_width > 0 && m_height > 0) return m_width / m_height;
    if (!m_frame.isNull()) return static_cast<double>(m_frame.width()) / m_frame.height();
    return 16.0 / 9.0;
}

bool ViewerWidget::verticalSequence() const { return !stage::isWide(aspect()); }

QRectF ViewerWidget::videoRect() const { return QRectF(stage::fitted(rect(), aspect())); }

QRectF ViewerWidget::pictureRect() const {
    const QRectF card = videoRect();
    const double frameWidth = m_width > 0 ? m_width : m_frame.width();
    const double frameHeight = m_height > 0 ? m_height : m_frame.height();
    if (m_zoom <= 0.0 || frameWidth <= 0 || frameHeight <= 0) return card;
    // One frame pixel a device pixel at 100%.
    const qreal ratio = std::max(1.0, devicePixelRatioF());
    const QSizeF size(frameWidth * m_zoom / ratio, frameHeight * m_zoom / ratio);
    return QRectF(card.center() - QPointF(size.width() / 2.0, size.height() / 2.0), size);
}

void ViewerWidget::setOverlays(QWidget *hud, QWidget *chip) {
    if (m_hud) m_hud->removeEventFilter(this);
    m_hud = hud;
    m_chip = chip;
    if (hud) {
        // It shows and hides itself (only while an agent or a render runs).
        const bool shown = !hud->isHidden() && hud->parentWidget();
        hud->setParent(this);
        hud->setVisible(shown);
        hud->installEventFilter(this);
    }
    if (chip) {
        const bool shown = !chip->isHidden() && chip->parentWidget();
        chip->setParent(this);
        chip->setVisible(shown);
    }
    placeOverlays();
    update();
}

bool ViewerWidget::eventFilter(QObject *watched, QEvent *event) {
    // Its shadow is the picture's to paint.
    if (watched == m_hud && (event->type() == QEvent::ShowToParent || event->type() == QEvent::HideToParent)) {
        placeOverlays();
        update();
    }
    return QWidget::eventFilter(watched, event);
}

QRect ViewerWidget::hudRect() const {
    const QRectF picture = videoRect();
    const double pw = picture.width();
    const double ph = picture.height();
    const double tx = std::round(pw * kTitleSafeInset);
    const double ty = std::round(ph * kTitleSafeInset);
    const double hw = std::max(0.0, std::min(double(kHudWidth), pw - 2.0 * tx - 16.0));
    return QRectF(picture.left() + pw - tx - 8.0 - hw, picture.top() + ty + 8.0, hw, kHudHeight).toRect();
}

void ViewerWidget::placeOverlays() {
    if (m_hud) {
        m_hud->setGeometry(hudRect());
        m_hud->raise();
    }
    if (m_chip) {
        const QRect picture = videoRect().toRect();
        m_chip->adjustSize();
        m_chip->move(picture.left() + kChipInset, picture.top() + kChipInset);
        m_chip->raise();
    }
}

void ViewerWidget::setSafeZones(bool shown) {
    if (shown == m_safeZones) return;
    m_safeZones = shown;
    update();
}

void ViewerWidget::setAnnotationsShown(bool shown) {
    if (shown == m_annotationsShown) return;
    m_annotationsShown = shown;
    update();
}

void ViewerWidget::setAnnotations(const QList<Annotation> &annotations) {
    m_annotations = annotations;
    update();
}

void ViewerWidget::setOverlap(const QRectF &rect, const QString &tag) {
    const bool shown = !rect.isEmpty();
    if (shown) {
        m_overlapRect = rect;
        m_overlapTag = tag;
    }
    if (shown == m_overlapShown) {
        update();
        return;
    }
    m_overlapShown = shown;
    m_overlapFade->stop();
    const double end = shown ? 1.0 : 0.0;
    if (!motionAllowed()) {
        m_overlapLevel = end;
        update();
        return;
    }
    m_overlapFade->setStartValue(m_overlapLevel);
    m_overlapFade->setEndValue(end);
    m_overlapFade->start();
}

QRectF ViewerWidget::overlapBox() const {
    if (m_overlapRect.isEmpty()) return {};
    const QRectF picture = pictureRect();
    return QRectF(picture.left() + m_overlapRect.x() * picture.width(), picture.top() + m_overlapRect.y() * picture.height(),
                  m_overlapRect.width() * picture.width(), m_overlapRect.height() * picture.height());
}

QRectF ViewerWidget::overlapTagRect() const {
    const QRectF box = overlapBox();
    if (box.isEmpty() || m_overlapTag.isEmpty()) return {};
    const QFontMetricsF metrics(tokens::font(tokens::monoSmall));
    const QSizeF size(ui::snapWidth(metrics, m_overlapTag) + 8.0, 16.0);
    // Right-aligned to the caption's right, 4 over it; without a caption,
    // to the box's.
    const QRectF caption = m_caption.isEmpty() ? QRectF() : captionRect();
    const QRectF anchor = caption.isEmpty() ? box : caption;
    return QRectF(QPointF(anchor.right() - size.width(), anchor.top() - 4.0 - size.height()), size);
}

void ViewerWidget::setZoom(double zoom) {
    zoom = std::max(0.0, zoom);
    if (zoom == m_zoom) return;
    m_zoom = zoom;
    m_scaled = QImage();
    update();
}

void ViewerWidget::beginDrawing() {
    cancelDrawing();
    setMaskTool(MaskTool::None);
    m_capturing = true;
    m_strokes.clear();
    m_stroke.clear();
    setCursor(Qt::CrossCursor);
    setFocus(Qt::OtherFocusReason);
    update();
}

QJsonArray ViewerWidget::capturedStrokes() const {
    QJsonArray out;
    for (const QVector<QPointF> &stroke : m_strokes) {
        QJsonArray points;
        for (const QPointF &point : stroke) points.append(QJsonArray{fourPlaces(point.x()), fourPlaces(point.y())});
        out.append(QJsonObject{{QStringLiteral("tool"), QStringLiteral("pen")},
                               {QStringLiteral("points"), points},
                               {QStringLiteral("color"), tokens::color(tokens::ink).name()}});
    }
    return out;
}

void ViewerWidget::finishDrawing() {
    if (!m_capturing) return;
    if (m_stroke.size() >= 2) m_strokes.append(m_stroke);
    const QJsonArray strokes = capturedStrokes();
    m_capturing = false;
    m_strokes.clear();
    m_stroke.clear();
    unsetCursor();
    update();
    // Nothing drawn is no drawing.
    if (strokes.isEmpty()) emit drawingCancelled();
    else emit drawingFinished(strokes);
}

void ViewerWidget::cancelDrawingCapture() {
    if (!m_capturing) return;
    m_capturing = false;
    m_strokes.clear();
    m_stroke.clear();
    unsetCursor();
    update();
    emit drawingCancelled();
}

QPointF ViewerWidget::frameFraction(const QPointF &widget) const {
    const QRectF picture = pictureRect();
    return QPointF(std::clamp((widget.x() - picture.left()) / std::max(1.0, picture.width()), 0.0, 1.0),
                   std::clamp((widget.y() - picture.top()) / std::max(1.0, picture.height()), 0.0, 1.0));
}

QPointF ViewerWidget::fromFrame(const QPointF &fraction) const {
    const QRectF picture = pictureRect();
    return QPointF(picture.left() + fraction.x() * picture.width(), picture.top() + fraction.y() * picture.height());
}

void ViewerWidget::resizeEvent(QResizeEvent *) {
    m_scaled = QImage();
    placeOverlays();
}

void ViewerWidget::showEvent(QShowEvent *) { placeOverlays(); }

QRectF ViewerWidget::captionBox(QString *text) const {
    const QRectF picture = pictureRect();
    const double w = picture.width();
    const double h = picture.height();
    const double tx = std::round(w * kTitleSafeInset);
    const double ty = std::round(h * kTitleSafeInset);
    const QFontMetricsF metrics(tokens::font(tokens::caption));
    constexpr double padX = 8.0, padY = 2.0, line = 20.0;
    QString shown = m_caption.simplified();
    double textWidth = 0.0;
    int lines = 1;
    if (!verticalSequence()) {
        // One line on a wide picture, cut at title safe.
        const double most = std::max(1.0, w - 2.0 * tx - 2.0 * padX);
        shown = metrics.elidedText(shown, Qt::ElideRight, most);
        textWidth = std::min(most, std::ceil(metrics.horizontalAdvance(shown)));
    } else {
        // Wrapped and balanced on a vertical one: the narrowest width that keeps
        // the line count the widest allows.
        const double most = std::max(1.0, w - 2.0 * tx - 16.0 - 2.0 * padX);
        const QStringList widest = wrapped(shown, metrics, most);
        lines = std::max<int>(1, int(widest.size()));
        double lo = 1.0, hi = most;
        for (int step = 0; step < 24 && hi - lo > 0.5; ++step) {
            const double mid = (lo + hi) / 2.0;
            if (wrapped(shown, metrics, mid).size() <= lines) hi = mid;
            else lo = mid;
        }
        double widestLine = 0.0;
        const QStringList balanced = wrapped(shown, metrics, hi);
        for (const QString &part : balanced) widestLine = std::max(widestLine, metrics.horizontalAdvance(part));
        textWidth = std::min(most, std::ceil(widestLine));
        lines = std::max<int>(1, int(balanced.size()));
        shown = balanced.join(QLatin1Char('\n'));
    }
    if (text) *text = shown;
    const QSizeF size(textWidth + 2.0 * padX, lines * line + 2.0 * padY);
    return QRectF(QPointF(picture.left() + std::round((w - size.width()) / 2.0), picture.top() + h - ty - size.height()), size);
}

QRectF ViewerWidget::captionRect() const { return m_caption.isEmpty() ? QRectF() : captionBox(nullptr); }


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
    const QRectF canvas = pictureRect();
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
    // Keys the masks and a drawing take ahead of the editor's shortcuts.
    if (event->type() == QEvent::ShortcutOverride) {
        const int key = static_cast<QKeyEvent *>(event)->key();
        const bool drawing = m_drawing || !m_penPoints.isEmpty();
        const bool capture = m_capturing && (key == Qt::Key_Escape || key == Qt::Key_Return || key == Qt::Key_Enter);
        const bool takes = capture || (key == Qt::Key_Escape && (drawing || m_maskTool != MaskTool::None)) ||
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
    if (m_capturing) {
        switch (event->key()) {
        case Qt::Key_Escape: cancelDrawingCapture(); return;
        case Qt::Key_Return:
        case Qt::Key_Enter: finishDrawing(); return;
        default: QWidget::keyPressEvent(event); return;
        }
    }
    switch (event->key()) {
    case Qt::Key_Escape:
        if (m_drawing || !m_penPoints.isEmpty()) cancelDrawing();
        else if (m_maskTool != MaskTool::None) setMaskTool(MaskTool::None);
        // Nothing of its own to cancel: on to the window (a sheet, a tray).
        else event->ignore();
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
    if (m_capturing) {
        // A stroke starts on the picture.
        if (pictureRect().contains(position)) {
            m_stroke = {frameFraction(position)};
            update();
        }
        return;
    }
    if (!editsMasks()) {
        // A click on the picture plays or pauses, as on a video site.
        // On the black bars, a click does nothing.
        if (videoRect().contains(position) && m_playEnabled) emit playRequested();
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
    if (m_capturing) {
        // The stroke follows the pointer, a point each point it moves.
        if ((event->buttons() & Qt::LeftButton) && !m_stroke.isEmpty()) {
            const QPointF at = frameFraction(position);
            if ((fromFrame(at) - fromFrame(m_stroke.constLast())).manhattanLength() >= 1.0) {
                m_stroke.append(at);
                update();
            }
        }
        return;
    }
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
    // Skimming is over the picture only; off it, the picture holds the
    // playhead's frame again.
    if (!videoRect().contains(position)) {
        if (m_skimming) {
            m_skimming = false;
            emit scrubEnded();
        }
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
    if (m_capturing) {
        if (!m_stroke.isEmpty()) {
            // A click leaves a dot: the same point twice.
            if (m_stroke.size() == 1) m_stroke.append(m_stroke.constFirst());
            m_strokes.append(m_stroke);
            m_stroke.clear();
            update();
        }
        return;
    }
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
    const QRectF picture = pictureRect();
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
        // Outlines are white over a dark rule, never the accent, so they
        // read on a picture in the accent's own hue; a path only (mode none)
        // is dashed.
        strokeOverPicture(p, path, selected, mask.value(QStringLiteral("mode")).toString() == QStringLiteral("none"));
        // The name on a tag over the outline's top left; the selected
        // mask's tag is the accent.
        if (!name.isEmpty()) paintMaskTag(p, path.boundingRect().topLeft(), name, selected, picture);
        if (!selected) continue;
        for (const QPointF &point : handlesOf(mask)) handle(toWidget.map(point), false);
    }
    if (m_drawing) {
        const QRectF box = QRectF(m_drawStart, m_drawEnd).normalized();
        QPainterPath path;
        if (m_maskTool == MaskTool::Ellipse) path.addEllipse(box);
        else path.addRect(box);
        strokeOverPicture(p, toWidget.map(path), true, false);
    }
    if (!m_penPoints.isEmpty()) {
        QPainterPath path(toWidget.map(m_penPoints.first()));
        for (int index = 1; index < m_penPoints.size(); ++index) path.lineTo(toWidget.map(m_penPoints.at(index)));
        strokeOverPicture(p, path, true, false);
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

void ViewerWidget::paintCaption(QPainter &p, bool flagged) {
    if (m_caption.isEmpty()) return;
    QString text;
    const QRectF box = captionBox(&text);
    p.save();
    p.setPen(Qt::NoPen);
    p.setBrush(tokens::color(tokens::overlay::captionBackground));
    p.drawRect(box);
    p.setFont(tokens::font(tokens::caption));
    p.setPen(tokens::color(tokens::overlay::captionText));
    const QStringList lines = text.split(QLatin1Char('\n'));
    for (int index = 0; index < lines.size(); ++index)
        p.drawText(QRectF(box.left(), box.top() + 2.0 + 20.0 * index, box.width(), 20.0), Qt::AlignCenter, lines.at(index));
    if (flagged && m_overlapLevel > 0.0) {
        // The QC note's outline, 2 outside the caption.
        QColor neg = tokens::color(tokens::neg);
        neg.setAlphaF(m_overlapLevel);
        p.setPen(QPen(neg, 1.0));
        p.setBrush(Qt::NoBrush);
        p.drawRect(box.adjusted(-2.5, -2.5, 2.5, 2.5));
    }
    p.restore();
}

void ViewerWidget::paintOverlap(QPainter &p) {
    if (m_overlapLevel <= 0.0) return;
    const QRectF box = overlapBox();
    if (box.isEmpty()) return;
    p.save();
    p.setOpacity(p.opacity() * m_overlapLevel);
    p.setPen(Qt::NoPen);
    p.setBrush(tokens::color(kOverlapFill));
    p.drawRect(box);
    ui::paintInsetLine(p, box, 0, tokens::color(tokens::neg));
    const QRectF tag = overlapTagRect();
    if (!tag.isEmpty()) {
        p.setPen(Qt::NoPen);
        p.setBrush(tokens::color(tokens::chrome));
        p.drawRoundedRect(tag, tokens::radius::small, tokens::radius::small);
        p.setFont(tokens::font(tokens::monoSmall));
        p.setPen(tokens::color(tokens::neg));
        p.drawText(tag, Qt::AlignCenter, m_overlapTag);
    }
    p.restore();
}

void ViewerWidget::paintStrokes(QPainter &p, const QJsonArray &drawing, const QString &initials) {
    QPointF start;
    bool hasStart = false;
    for (const QJsonValue &value : drawing) {
        const QJsonObject stroke = value.toObject();
        QVector<QPointF> points;
        for (const QJsonValue &point : stroke.value(QStringLiteral("points")).toArray()) points.append(fromFrame(pointOf(point)));
        if (points.isEmpty()) continue;
        const QString tool = stroke.value(QStringLiteral("tool")).toString();
        QPainterPath path;
        if ((tool == QLatin1String("rect") || tool == QLatin1String("ellipse")) && points.size() >= 2) {
            const QRectF box = QRectF(points.at(0), points.at(1)).normalized();
            if (tool == QLatin1String("rect")) path.addRect(box);
            else path.addEllipse(box);
        } else if (tool == QLatin1String("arrow") && points.size() >= 2) {
            const QPointF from = points.at(0), to = points.at(1);
            path.moveTo(from);
            path.lineTo(to);
            // A head of 10 at 28 degrees either side.
            const double angle = std::atan2(to.y() - from.y(), to.x() - from.x());
            for (const double side : {-0.4887, 0.4887}) {
                path.moveTo(to);
                path.lineTo(to - QPointF(std::cos(angle + side), std::sin(angle + side)) * 10.0);
            }
        } else {
            path.moveTo(points.first());
            for (int index = 1; index < points.size(); ++index) path.lineTo(points.at(index));
        }
        p.setBrush(Qt::NoBrush);
        p.setPen(QPen(tokens::color(kStrokeUnder), 4.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p.drawPath(path);
        p.setPen(QPen(strokeColor(stroke), 2.0, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p.drawPath(path);
        if (!hasStart) {
            start = points.first();
            hasStart = true;
        }
    }
    if (!hasStart || initials.isEmpty()) return;
    // Who drew it, at the start of the first stroke.
    const QRectF avatar(start - QPointF(kAvatar / 2.0, kAvatar / 2.0), QSizeF(kAvatar, kAvatar));
    p.setPen(QPen(QColor(Qt::black), 1.0));
    p.setBrush(tokens::color(tokens::on));
    p.drawEllipse(avatar.adjusted(-0.5, -0.5, 0.5, 0.5));
    p.setFont(tokens::font(tokens::metaStrong));
    p.setPen(tokens::color(tokens::ink));
    p.drawText(avatar, Qt::AlignCenter, initials);
}

void ViewerWidget::paintAnnotations(QPainter &p) {
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    if (m_annotationsShown)
        for (const Annotation &annotation : std::as_const(m_annotations)) paintStrokes(p, annotation.drawing, annotation.initials);
    if (m_capturing) {
        // The drawing in hand, as it will be kept.
        QJsonArray strokes = capturedStrokes();
        if (m_stroke.size() >= 1) {
            QJsonArray points;
            for (const QPointF &point : m_stroke) points.append(QJsonArray{point.x(), point.y()});
            if (m_stroke.size() == 1) points.append(points.first());
            strokes.append(QJsonObject{{QStringLiteral("tool"), QStringLiteral("pen")}, {QStringLiteral("points"), points}});
        }
        paintStrokes(p, strokes, QString());
    }
    p.restore();
}

void ViewerWidget::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    // Black bars, where the region is not of the picture's shape.
    p.fillRect(rect(), Qt::black);
    const QRectF card = videoRect();
    const QRectF picture = pictureRect();
    p.fillRect(card, theme.viewerCanvas);
    QPainterPath viewport;
    viewport.addRect(card);
    if (!m_frame.isNull()) {
        p.save();
        p.setClipPath(viewport);
        const qreal dpr = m_liveScrub ? 1.0 : std::max(1.0, devicePixelRatioF());
        const QSize target(std::max(1, static_cast<int>(std::round(picture.width() * dpr))),
                           std::max(1, static_cast<int>(std::round(picture.height() * dpr))));
        if (m_scaled.isNull() || m_scaled.size() != target || m_scaledFast != m_liveScrub) {
            m_scaled = m_frame.scaled(target, Qt::KeepAspectRatio,
                                      m_liveScrub ? Qt::FastTransformation : Qt::SmoothTransformation);
            m_scaled.setDevicePixelRatio(dpr);
            m_scaledFast = m_liveScrub;
        }
        const QPointF topLeft(picture.center().x() - m_scaled.width() / dpr / 2.0,
                              picture.center().y() - m_scaled.height() / dpr / 2.0);
        p.drawImage(topLeft, m_scaled);
        p.restore();
    } else {
        // Two quiet lines, wrapped inside the picture.
        p.save();
        p.setClipRect(card);
        const QRectF area = card.adjusted(18, 0, -18, 0);
        const QFont title = tokens::font(tokens::controlStrong);
        const QFont detail = tokens::font(tokens::meta);
        const QString titleText = QStringLiteral("Your viewer");
        const QString detailText = QStringLiteral("The playhead frame shows here.");
        const int flags = Qt::AlignHCenter | Qt::AlignTop | Qt::TextWordWrap;
        const QRectF titleBox = QFontMetricsF(title).boundingRect(area, flags, titleText);
        const QRectF detailBox = QFontMetricsF(detail).boundingRect(area, flags, detailText);
        const qreal gap = 4;
        const qreal top = card.center().y() - (titleBox.height() + gap + detailBox.height()) / 2.0;
        p.setFont(title);
        p.setPen(theme.viewerText);
        p.drawText(QRectF(area.left(), top, area.width(), titleBox.height()), flags, titleText);
        p.setFont(detail);
        p.setPen(theme.viewerTextDim);
        p.drawText(QRectF(area.left(), top + titleBox.height() + gap, area.width(), detailBox.height()), flags, detailText);
        p.restore();
    }
    // Over the picture, inside its rect: the caption, the QC overlap box,
    // safe zones and the comments' drawings; the masks' outlines on top.
    p.save();
    p.setClipPath(viewport);
    paintCaption(p, m_overlapShown || m_overlapLevel > 0.0);
    paintOverlap(p);
    if (m_safeZones) paintSafeZones(p, picture);
    paintAnnotations(p);
    p.restore();
    paintMasks(p);
    // The agent HUD's --pop shadow (0 8 24 rgba(0,0,0,.45)), under the card
    // it paints itself: the HUD is a child, so it lands over this.
    if (m_hud && m_hud->isVisible()) ui::paintShadow(p, QRectF(m_hud->geometry()), tokens::radius::card);
}

QRectF ViewerWidget::hudShadowRect() const {
    if (!m_hud || !m_hud->isVisible()) return {};
    return QRectF(m_hud->geometry()).marginsAdded(QMarginsF(ui::shadowMargins()));
}


double ViewerWidget::fractionAt(double x) const {
    // Across the picture, as the player's bar maps it, so the frame skimmed
    // is the one the bar's tip names.
    const QRectF card = videoRect();
    return std::clamp((x - card.left()) / std::max(1.0, card.width()), 0.0, 1.0);
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
