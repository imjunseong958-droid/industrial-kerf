#include "viewer_widget.h"

#include <QTimer>

#include "components.h"
#include "design_tokens.h"
#include "stage_shell.h"
#include <QVariantAnimation>
#include "editor_common.h"
#include "editor_theme.h"
#include "motion_path.h"
#include "stage_shell.h"
#include "ui/components.h"

#include <QCursor>
#include <QFontMetricsF>
#include <QGuiApplication>
#include <QKeyEvent>
#include <QLineF>
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
constexpr double kPathDragStart = 3.0;
// `path=` holds four places, so a point that comes back may be this far from where it was.
constexpr double kPathSamePoint = 0.0002;

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
    if (m_wipeArmed && !frame.isNull()) {
        m_wipeArmed = false;
        const int length = ui::motionDuration(tokens::motion::stage);
        if (length > 0 && !m_wipeFrom.isNull()) {
            if (!m_wipeRun) {
                m_wipeRun = new QVariantAnimation(this);
                m_wipeRun->setStartValue(0.0);
                m_wipeRun->setEndValue(1.0);
                m_wipeRun->setEasingCurve(tokens::easing());
                connect(m_wipeRun, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
                    const qreal from = m_wipe;
                    m_wipe = value.toReal();
                    updateLine(from, m_wipe, 2);
                });
                connect(m_wipeRun, &QAbstractAnimation::finished, this, [this] {
                    m_wipe = -1.0;
                    m_wipeFrom = QImage();
                    update();
                });
            }
            // A new one starts over, rather than going on from where the last had got.
            m_wipeRun->stop();
            m_wipe = 0.0;
            m_wipeRun->setDuration(length);
            m_wipeRun->start();
        }
    }
    const QRect shownBefore = shownRect();
    m_frame = frame;
    m_time = time;
    m_scaled = QImage();
    const bool following = m_frameClock.isValid() && m_frameClock.elapsed() < tokens::motion::frameStream;
    m_frameClock.restart();
    m_streaming = following && !m_frame.isNull();
    // Drawn as it comes, a picture is opaque: in the format the screen is drawn in,
    // once, rather than in the paint.
    m_straight = m_streaming ? m_frame.convertToFormat(QImage::Format_RGB32) : QImage();
    if (m_streaming) {
        if (!m_settle) {
            m_settle = new QTimer(this);
            m_settle->setSingleShot(true);
            m_settle->setInterval(tokens::motion::frameSettle);
            connect(m_settle, &QTimer::timeout, this, [this] {
                // They have stopped: the last is drawn at its best.
                m_streaming = false;
                m_straight = QImage();
                update();
            });
        }
        m_settle->start();
    }
    // A frame of the same shape changes only the picture: what is around it, and
    // what is under that, is not drawn again.
    const QRect shown = shownRect();
    if (shown == shownBefore && !shown.isEmpty()) update(shown);
    else update();
}

QRect ViewerWidget::shownRect() const {
    return videoRect().united(pictureRect()).toAlignedRect().adjusted(-1, -1, 1, 1).intersected(rect());
}

void ViewerWidget::clearFrame() {
    m_frame = QImage();
    m_scaled = QImage();
    m_straight = QImage();
    m_streaming = false;
    update();
}

void ViewerWidget::updateLine(qreal from, qreal to, int reach) {
    // A line crossing the picture changes only what it crosses.
    const QRectF card = videoRect();
    const qreal left = card.left() + card.width() * std::min(from, to);
    const qreal right = card.left() + card.width() * std::max(from, to);
    update(QRectF(left - reach, card.top(), right - left + 2 * reach, card.height()).toAlignedRect());
}

void ViewerWidget::setCompareFrame(const QImage &before) {
    // In the format it draws fastest in, as it is drawn for as long as it is watched.
    m_compareFrame = before.isNull() || before.format() == QImage::Format_RGB32 ? before : before.convertToFormat(QImage::Format_RGB32);
    m_compareScaled = QImage();
    if (before.isNull()) {
        if (m_compareSweep) m_compareSweep->stop();
        update();
        return;
    }
    if (!m_compareSweep && !ui::reducedMotion()) {
        m_compareSweep = new QVariantAnimation(this);
        m_compareSweep->setDuration(tokens::motion::compareSweep);
        m_compareSweep->setLoopCount(-1);
        m_compareSweep->setStartValue(tokens::motion::compareFrom);
        m_compareSweep->setKeyValueAt(0.5, tokens::motion::compareTo);
        m_compareSweep->setEndValue(tokens::motion::compareFrom);
        m_compareSweep->setEasingCurve(QEasingCurve::Linear);
        connect(m_compareSweep, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
            const qreal from = m_compareAt;
            m_compareAt = value.toReal();
            // The line has a soft glow either side of it.
            updateLine(from, m_compareAt, 10);
        });
    }
    if (m_compareSweep && m_compareSweep->state() != QAbstractAnimation::Running) m_compareSweep->start();
    update();
}

void ViewerWidget::setFloating(bool floating) {
    m_floating = floating;
    update();
}

void ViewerWidget::setWorkChip(QWidget *chip) {
    m_workChip = chip;
    if (chip) chip->setParent(this);
    placeOverlays();
}

void ViewerWidget::wipeToNextFrame() {
    if (m_frame.isNull()) return;
    m_wipeFrom = m_frame;
    m_wipeArmed = true;
}

void ViewerWidget::setLiveScrub(bool live) {
    if (m_liveScrub == live) return;
    m_liveScrub = live;
    m_scaled = QImage();
    update();
}

void ViewerWidget::setStageMoving(bool moving) {
    if (m_stageMoving == moving) return;
    m_stageMoving = moving;
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

QRectF ViewerWidget::videoRect() const { return QRectF(stage::fitted(stage::paddedSlot(rect()), aspect())); }

QRegion ViewerWidget::coveredRegion() const {
    // All of the frame at Fit but the squares its corners are rounded in.
    const QRectF card = videoRect();
    const qreal round = tokens::layout::pictureRadius;
    const auto inward = [](const QRectF &r) {
        if (r.isEmpty()) return QRect();
        const int left = int(std::ceil(r.left()));
        const int top = int(std::ceil(r.top()));
        return QRect(QPoint(left, top), QPoint(int(std::floor(r.right())) - 1, int(std::floor(r.bottom())) - 1));
    };
    QRegion covered(inward(card.adjusted(round, 0, -round, 0)));
    covered += inward(card.adjusted(0, round, 0, -round));
    return covered;
}

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
        if (m_workChip) {
            m_workChip->adjustSize();
            m_workChip->move(picture.left() + kChipInset, picture.top() + kChipInset);
            m_workChip->raise();
        }
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
    if (!m_stageMoving) m_scaled = QImage();
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

void ViewerWidget::setMotionPath(const QString &clip, const QString &text, const QTransform &placement, double along) {
    // A drag in progress keeps its curve until it is let go.
    if (m_pathDrag >= 0 && clip == m_pathClip) {
        if (placement.isInvertible()) m_pathPlacement = placement;
        m_pathAlong = along;
        return;
    }
    auto points = clip.isEmpty() || !placement.isInvertible() ? std::nullopt : motionpath::parsePath(text);
    // All of it on or behind the camera is no curve to show.
    if (points) {
        const QList<QPointF> samples = motionpath::MotionPath(*points, aspect()).samples();
        if (std::none_of(samples.cbegin(), samples.cend(), [&placement](const QPointF &sample) { return inSight(placement, sample); })) points.reset();
    }
    const QString shown = points ? clip : QString();
    const QList<QPointF> list = points.value_or(QList<QPointF>{});
    if (shown == m_pathClip && list == m_pathPoints && placement == m_pathPlacement && along == m_pathAlong) return;
    // The selection goes with the point, as long as a new state still has it: an
    // undo can leave another point in its place.
    if (m_pathSelected >= 0) {
        const QPointF selected = m_pathPoints.at(m_pathSelected);
        m_pathSelected = -1;
        for (qsizetype index = 0; shown == m_pathClip && index < list.size(); ++index)
            if (QLineF(list.at(index), selected).length() < kPathSamePoint) m_pathSelected = int(index);
    }
    m_pathClip = shown;
    m_pathPoints = list;
    m_pathPlacement = placement;
    m_pathAlong = along;
    m_pathDrag = -1;
    updateCursor(mapFromGlobal(QCursor::pos()));
    update();
}

void ViewerWidget::setSampledPath(const QList<std::optional<QPointF>> &places, const std::optional<QPointF> &now) {
    // A drag in progress keeps its curve until it is let go.
    if (m_sampledDrag >= 0) {
        m_sampledNow = now;
        return;
    }
    const bool seen = std::any_of(places.cbegin(), places.cend(), [](const std::optional<QPointF> &place) { return place.has_value(); });
    const QList<std::optional<QPointF>> shown = seen ? places : QList<std::optional<QPointF>>{};
    if (shown.isEmpty()) {
        m_sampledEdit = {};
        m_sampledHandles.clear();
    }
    if (shown == m_sampledPath && now == m_sampledNow) return;
    m_sampledPath = shown;
    m_sampledNow = now;
    update();
}

void ViewerWidget::setSampledPathEdit(const SampledPathEdit &edit) {
    if (m_sampledDrag >= 0) return;
    m_sampledEdit = edit;
    const QList<std::optional<QPointF>> handles = edit.handles ? edit.handles(edit.points) : QList<std::optional<QPointF>>{};
    if (handles == m_sampledHandles) return;
    m_sampledHandles = handles;
    update();
}

QPointF ViewerWidget::sampledToWidget(const QPointF &share) const {
    const QRectF canvas = pictureRect();
    return {canvas.left() + share.x() * canvas.width(), canvas.top() + share.y() * canvas.height()};
}

int ViewerWidget::sampledPointAt(const QPointF &widget) const {
    if (!m_sampledEdit.solve) return -1;
    for (qsizetype index = m_sampledHandles.size() - 1; index >= 0; --index) {
        if (!m_sampledHandles.at(index)) continue;
        const QPointF delta = sampledToWidget(*m_sampledHandles.at(index)) - widget;
        if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) return int(index);
    }
    return -1;
}

QPointF ViewerWidget::pathToWidget(const QPointF &point) const {
    const QRectF canvas = pictureRect();
    const QPointF on = m_pathPlacement.map(point);
    return {canvas.left() + on.x() * canvas.width(), canvas.top() + on.y() * canvas.height()};
}

std::optional<QPointF> ViewerWidget::pathPoint(const QPointF &widget) const {
    const QRectF canvas = pictureRect();
    return pointInSight(m_pathPlacement, QPointF((widget.x() - canvas.left()) / std::max(1.0, canvas.width()),
                                                 (widget.y() - canvas.top()) / std::max(1.0, canvas.height())));
}

int ViewerWidget::pathPointAt(const QPointF &widget) const {
    if (m_pathClip.isEmpty()) return -1;
    for (qsizetype index = m_pathPoints.size() - 1; index >= 0; --index) {
        if (!inSight(m_pathPlacement, m_pathPoints.at(index))) continue;
        const QPointF delta = pathToWidget(m_pathPoints.at(index)) - widget;
        if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) return int(index);
    }
    return -1;
}

double ViewerWidget::pathCurveAt(const QPointF &widget) const {
    const QList<QPointF> line = motionPathCurve();
    const QList<QPointF> samples = m_pathClip.isEmpty() ? QList<QPointF>{} : motionpath::MotionPath(m_pathPoints, aspect()).samples();
    double nearest = kHandleReach;
    double found = -1.0;
    for (qsizetype index = 0; index + 1 < line.size(); ++index) {
        if (!inSight(m_pathPlacement, samples.at(index)) || !inSight(m_pathPlacement, samples.at(index + 1))) continue;
        const QPointF from = line.at(index);
        const QPointF along = line.at(index + 1) - from;
        const double length = QPointF::dotProduct(along, along);
        const double t = length > 0.0 ? std::clamp(QPointF::dotProduct(widget - from, along) / length, 0.0, 1.0) : 0.0;
        const double distance = QLineF(from + along * t, widget).length();
        if (distance > nearest) continue;
        nearest = distance;
        found = (double(index) + t) / motionpath::MotionPath::kSamplesPerSpan;
    }
    return found;
}

void ViewerWidget::selectPathPoint(int point) {
    if (point == m_pathSelected) return;
    m_pathSelected = point;
    update();
}

void ViewerWidget::writePath() {
    emit editRequested(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")},
                                              {QStringLiteral("clip"), m_pathClip},
                                              {QStringLiteral("props"), QJsonObject{{QStringLiteral("path"), motionpath::pathText(m_pathPoints)}}}}});
}

void ViewerWidget::removeSelectedPathPoint() {
    // A path needs its two, and the key has still been answered.
    if (m_pathSelected < 0 || m_pathPoints.size() <= 2) return;
    m_pathPoints.removeAt(m_pathSelected);
    m_pathSelected = -1;
    update();
    writePath();
}

QList<QPointF> ViewerWidget::motionPathPoints() const {
    QList<QPointF> points;
    if (m_pathClip.isEmpty()) return points;
    for (const QPointF &point : m_pathPoints) points.append(pathToWidget(point));
    return points;
}

QList<QPointF> ViewerWidget::motionPathCurve() const {
    QList<QPointF> line;
    if (m_pathClip.isEmpty()) return line;
    for (const QPointF &point : motionpath::MotionPath(m_pathPoints, aspect()).samples()) line.append(pathToWidget(point));
    return line;
}

QPointF ViewerWidget::motionPathMarker() const {
    if (m_pathClip.isEmpty()) return {};
    const auto on = motionpath::MotionPath(m_pathPoints, aspect()).at(m_pathAlong);
    return pathToWidget({on.x, on.y});
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
    return !m_maskClip.isEmpty() && m_placement.isInvertible() && (m_maskTool != MaskTool::None || !m_masks.isEmpty());
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

std::optional<QPointF> ViewerWidget::picturePoint(const QPointF &widget) const {
    return pointInSight(pictureToWidget(), widget);
}

int ViewerWidget::maskIndex(const QString &name) const {
    if (name.isEmpty()) return -1;
    for (int index = 0; index < m_masks.size(); ++index)
        if (m_masks.at(index).toObject().value(QStringLiteral("name")).toString() == name) return index;
    return -1;
}

int ViewerWidget::handleAt(const QPointF &widget) const {
    const int index = maskIndex(m_selectedMask);
    if (index < 0 || !m_placement.isInvertible()) return -1;
    const QTransform toWidget = pictureToWidget();
    const QVector<QPointF> handles = handlesOf(m_masks.at(index).toObject());
    for (int handle = handles.size() - 1; handle >= 0; --handle) {
        if (!inSight(toWidget, handles.at(handle))) continue;
        const QPointF delta = toWidget.map(handles.at(handle)) - widget;
        if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) return handle;
    }
    return -1;
}

QString ViewerWidget::maskAt(const QPointF &widget) const {
    if (!m_placement.isInvertible()) return {};
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
    else if (m_sampledDrag >= 0 || sampledPointAt(widget) >= 0 || m_pathDrag >= 0 || pathPointAt(widget) >= 0) setCursor(Qt::PointingHandCursor);
    else if (handleAt(widget) < 0 && pathCurveAt(widget) >= 0.0) setCursor(Qt::CrossCursor);
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
                            (drawing || maskIndex(m_selectedMask) >= 0 || m_pathSelected >= 0));
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
        // Nothing of its own to cancel: on to the window (a sheet).
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
        } else if (m_pathSelected >= 0) {
            removeSelectedPathPoint();
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
    if (m_maskTool == MaskTool::None) {
        if (const int point = sampledPointAt(position); point >= 0) {
            setFocus(Qt::MouseFocusReason);
            m_sampledDrag = point;
            m_sampledBefore = m_sampledEdit.points;
            m_sampledHeld = sampledToWidget(*m_sampledHandles.at(point)) - position;
            m_sampledPressed = position;
            m_sampledMoved = false;
            updateCursor(position);
            return;
        }
        if (const int point = pathPointAt(position); point >= 0) {
            setFocus(Qt::MouseFocusReason);
            // One thing for Delete to take at a time.
            if (!m_selectedMask.isEmpty()) {
                m_selectedMask.clear();
                emit maskSelected(QString());
            }
            selectPathPoint(point);
            m_pathDrag = point;
            m_pathBefore = m_pathPoints;
            m_pathHeld = pathToWidget(m_pathPoints.at(point)) - position;
            m_pathPressed = position;
            m_pathMoved = false;
            updateCursor(position);
            return;
        }
        // A click on the curve is the start of a double-click, not of playing;
        // a click off it lets go of a point rather than playing.
        const bool hadPoint = m_pathSelected >= 0;
        selectPathPoint(-1);
        if (handleAt(position) < 0 && pathCurveAt(position) >= 0.0) {
            setFocus(Qt::MouseFocusReason);
            return;
        }
        if (hadPoint && !editsMasks()) return;
    }
    if (!editsMasks()) {
        // A click on the picture plays or pauses, as on a video site.
        // On the black bars, a click does nothing.
        if (videoRect().contains(position) && m_playEnabled) emit playRequested();
        return;
    }
    setFocus(Qt::MouseFocusReason);
    // Past the horizon of a picture in depth there is nowhere on it to draw.
    const std::optional<QPointF> at = picturePoint(position);
    if (m_maskTool != MaskTool::None && !at) return;
    if (m_maskTool == MaskTool::Pen) {
        // A click on the first point closes the outline.
        if (m_penPoints.size() >= 3) {
            const QPointF delta = pictureToWidget().map(m_penPoints.first()) - position;
            if (std::abs(delta.x()) <= kHandleReach && std::abs(delta.y()) <= kHandleReach) {
                finishPen(true);
                return;
            }
        }
        m_penPoints.append(*at);
        m_pointer = position;
        update();
        return;
    }
    if (m_maskTool != MaskTool::None) {
        m_drawing = true;
        m_drawStart = m_drawEnd = *at;
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
    if (name.isEmpty() || !at) return;
    m_dragHandle = handle >= 0 ? handle : kMoveMask;
    m_dragFrom = *at;
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
    // Past the horizon of a plane in depth, what is in hand stays where it last was;
    // a path's point does a little before that, where the document can no longer say it.
    if (m_drawing) {
        if (const auto at = picturePoint(position)) m_drawEnd = *at;
        update();
        return;
    }
    if (m_sampledDrag >= 0) {
        if (!m_sampledMoved && QLineF(position, m_sampledPressed).length() < kPathDragStart) return;
        m_sampledMoved = true;
        const QRectF canvas = pictureRect();
        const QPointF to = position + m_sampledHeld;
        const QPointF share((to.x() - canvas.left()) / std::max(1.0, canvas.width()), (to.y() - canvas.top()) / std::max(1.0, canvas.height()));
        // What cannot land there, as past a horizon, stays where it last was.
        if (const auto at = m_sampledEdit.solve(m_sampledEdit.points, m_sampledDrag, share); at && motionpath::pathHolds(*at)) {
            m_sampledEdit.points[m_sampledDrag] = *at;
            m_sampledPath = m_sampledEdit.curve(m_sampledEdit.points);
            m_sampledHandles = m_sampledEdit.handles(m_sampledEdit.points);
        }
        update();
        return;
    }
    if (m_pathDrag >= 0) {
        // A hand that wobbles while clicking has not moved anything.
        if (!m_pathMoved && QLineF(position, m_pathPressed).length() < kPathDragStart) return;
        m_pathMoved = true;
        if (const auto at = pathPoint(position + m_pathHeld); at && motionpath::pathHolds(*at)) m_pathPoints[m_pathDrag] = *at;
        update();
        return;
    }
    if (m_dragHandle != -1) {
        const int index = maskIndex(m_dragOriginal.value(QStringLiteral("name")).toString());
        const auto at = picturePoint(position);
        if (index >= 0 && at)
            m_masks[index] = m_dragHandle == kMoveMask ? movedBy(m_dragOriginal, *at - m_dragFrom)
                                                       : withHandle(m_dragOriginal, m_dragHandle, *at);
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
    // So does it while a path can be dragged.
    if (!m_pathClip.isEmpty() || m_sampledEdit.solve) {
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
        if (const auto at = picturePoint(event->position())) m_drawEnd = *at;
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
    if (m_sampledDrag >= 0) {
        const bool moved = m_sampledMoved && m_sampledEdit.points != m_sampledBefore;
        m_sampledDrag = -1;
        m_sampledMoved = false;
        updateCursor(event->position());
        if (moved)
            emit editRequested(QJsonArray{QJsonObject{{QStringLiteral("op"), QStringLiteral("set")},
                                                      {QStringLiteral("clip"), m_sampledEdit.clip},
                                                      {QStringLiteral("props"), QJsonObject{{QStringLiteral("path"), motionpath::pathText(m_sampledEdit.points)}}}}});
        return;
    }
    if (m_pathDrag >= 0) {
        const bool moved = m_pathMoved && m_pathPoints != m_pathBefore;
        m_pathDrag = -1;
        m_pathMoved = false;
        updateCursor(event->position());
        if (moved) writePath();
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

void ViewerWidget::mouseDoubleClickEvent(QMouseEvent *event) {
    const QPointF position = event->position();
    if (event->button() == Qt::LeftButton && !m_capturing && m_maskTool == MaskTool::None && pathPointAt(position) < 0 && handleAt(position) < 0) {
        if (const double u = pathCurveAt(position); u >= 0.0) {
            m_pathPoints = motionpath::withPointOn(m_pathPoints, aspect(), u);
            selectPathPoint(int(std::clamp<qsizetype>(qsizetype(std::floor(u)), 0, m_pathPoints.size() - 3)) + 1);
            update();
            writePath();
            return;
        }
    }
    // As a press, which is what it has always been to all else.
    mousePressEvent(event);
}

void ViewerWidget::leaveEvent(QEvent *) {
    if (!m_skimming) return;
    m_skimming = false;
    emit scrubEnded();
}


void ViewerWidget::paintMasks(QPainter &p) {
    if (m_maskClip.isEmpty() || !m_placement.isInvertible()) return;
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
        if (!name.isEmpty() && !path.isEmpty()) paintMaskTag(p, path.boundingRect().topLeft(), name, selected, picture);
        if (!selected) continue;
        for (const QPointF &point : handlesOf(mask))
            if (inSight(toWidget, point)) handle(toWidget.map(point), false);
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

void ViewerWidget::paintMotionPath(QPainter &p) {
    if (!m_sampledPath.isEmpty()) {
        const QRectF canvas = pictureRect();
        const auto toWidget = [&canvas](const QPointF &share) { return QPointF(canvas.left() + share.x() * canvas.width(), canvas.top() + share.y() * canvas.height()); };
        p.save();
        p.setRenderHint(QPainter::Antialiasing);
        p.setBrush(Qt::NoBrush);
        QPainterPath path;
        bool going = false;
        for (const std::optional<QPointF> &place : std::as_const(m_sampledPath)) {
            if (place && going) path.lineTo(toWidget(*place));
            else if (place) path.moveTo(toWidget(*place));
            going = place.has_value();
        }
        strokeOverPicture(p, path, true, false);
        if (m_sampledNow) {
            const EditorTheme &theme = editorTheme();
            p.setPen(QPen(theme.maskHandleFill, 1.0));
            p.setBrush(theme.maskHandleBorder);
            p.drawEllipse(toWidget(*m_sampledNow), kHandleSize / 2.0, kHandleSize / 2.0);
        }
        // Its points are square, as those of any path are.
        for (qsizetype index = 0; index < m_sampledHandles.size(); ++index) {
            if (!m_sampledHandles.at(index)) continue;
            const EditorTheme &theme = editorTheme();
            const bool held = index == m_sampledDrag;
            p.setPen(QPen(held ? theme.maskHandleFill : theme.maskHandleBorder, 1.0));
            p.setBrush(held ? theme.maskHandleBorder : theme.maskHandleFill);
            p.drawRect(QRectF(toWidget(*m_sampledHandles.at(index)) - QPointF(kHandleSize / 2.0, kHandleSize / 2.0), QSizeF(kHandleSize, kHandleSize)));
        }
        p.restore();
    }
    if (m_pathClip.isEmpty()) return;
    p.save();
    p.setRenderHint(QPainter::Antialiasing);
    p.setBrush(Qt::NoBrush);
    // In depth, what is on or behind the camera breaks the curve.
    const QList<QPointF> line = motionPathCurve();
    const QList<QPointF> samples = motionpath::MotionPath(m_pathPoints, aspect()).samples();
    QPainterPath path;
    bool going = false;
    for (qsizetype index = 0; index < line.size(); ++index) {
        const bool seen = inSight(m_pathPlacement, samples.at(index));
        if (seen && going) path.lineTo(line.at(index));
        else if (seen) path.moveTo(line.at(index));
        going = seen;
    }
    strokeOverPicture(p, path, true, false);
    const EditorTheme &theme = editorTheme();
    // The points are square, as a mask's handles are; a round one says where the clip is now.
    const QList<QPointF> points = motionPathPoints();
    for (qsizetype index = 0; index < points.size(); ++index) {
        if (!inSight(m_pathPlacement, m_pathPoints.at(index))) continue;
        // The selected point turns white, as the pen's first point does when it would close.
        const bool selected = index == m_pathSelected;
        p.setPen(QPen(selected ? theme.maskHandleFill : theme.maskHandleBorder, 1.0));
        p.setBrush(selected ? theme.maskHandleBorder : theme.maskHandleFill);
        p.drawRect(QRectF(points.at(index) - QPointF(kHandleSize / 2.0, kHandleSize / 2.0), QSizeF(kHandleSize, kHandleSize)));
    }
    const auto on = motionpath::MotionPath(m_pathPoints, aspect()).at(m_pathAlong);
    if (inSight(m_pathPlacement, QPointF(on.x, on.y))) {
        p.setPen(QPen(theme.maskHandleFill, 1.0));
        p.setBrush(theme.maskHandleBorder);
        p.drawEllipse(motionPathMarker(), kHandleSize / 2.0, kHandleSize / 2.0);
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

void ViewerWidget::paintEvent(QPaintEvent *event) {
    const EditorTheme &theme = editorTheme();
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    // Black bars, where the region is not of the picture's shape; floating, the
    // room shows there.
    if (!m_floating) p.fillRect(rect(), Qt::black);
    const QRectF card = videoRect();
    const QRectF picture = pictureRect();
    QPainterPath viewport;
    viewport.addRoundedRect(card, tokens::layout::pictureRadius, tokens::layout::pictureRadius);
    // Kept inside the picture's rounded corners. Rounding costs a pass over all of
    // it; what is drawn again away from the corners (a line crossing it) is kept
    // inside its rect instead, which costs nothing.
    const qreal round = tokens::layout::pictureRadius + 1;
    const QRect dirty = event->rect();
    bool nearCorner = false;
    for (const QPointF &corner : {card.topLeft(), card.topRight(), card.bottomLeft(), card.bottomRight()})
        nearCorner = nearCorner || dirty.intersects(QRectF(corner - QPointF(round, round), QSizeF(2 * round, 2 * round)).toAlignedRect());
    const auto keepInside = [&] {
        if (nearCorner) p.setClipPath(viewport);
        else p.setClipRect(card);
    };
    p.fillPath(viewport, theme.viewerCanvas);
    if (!m_frame.isNull()) {
        p.save();
        keepInside();
        const qreal dpr = m_liveScrub ? 1.0 : std::max(1.0, devicePixelRatioF());
        const QSize target(std::max(1, static_cast<int>(std::round(picture.width() * dpr))),
                           std::max(1, static_cast<int>(std::round(picture.height() * dpr))));
        const bool fast = m_liveScrub || m_stageMoving;
        // On the move what it has will do, unless it is far too small by now: making
        // it again costs far more than drawing it, in the frames that show it least.
        const bool enough = m_stageMoving && !m_scaled.isNull() && m_scaled.width() * 10 >= target.width() * 7;
        // Playing, each frame is on screen for a moment: sampled straight onto it,
        // with no picture of it made first.
        const bool straight = m_streaming && !m_straight.isNull() && !m_stageMoving && !m_liveScrub;
        if (straight) {
            const QSizeF shown = QSizeF(m_straight.size()).scaled(picture.size(), Qt::KeepAspectRatio);
            p.setRenderHint(QPainter::SmoothPixmapTransform, true);
            p.drawImage(QRectF(picture.center() - QPointF(shown.width(), shown.height()) / 2.0, shown), m_straight);
            p.setRenderHint(QPainter::SmoothPixmapTransform, false);
        } else if (!enough && (m_scaled.isNull() || m_scaled.size() != target || (m_scaledFast && !fast) || (!m_scaledFast && m_liveScrub))) {
            m_scaled = m_frame.scaled(target, Qt::KeepAspectRatio, fast ? Qt::FastTransformation : Qt::SmoothTransformation)
                           .convertToFormat(QImage::Format_RGB32);
            m_scaledDpr = dpr;
            m_scaled.setDevicePixelRatio(dpr);
            m_scaledFast = fast;
        }
        if (straight) {
            // Drawn above.
        } else if (m_stageMoving) {
            // Its size is the picture's, as it goes.
            const QSizeF shown = QSizeF(m_scaled.size()).scaled(picture.size(), Qt::KeepAspectRatio);
            p.setRenderHint(QPainter::SmoothPixmapTransform, true);
            p.drawImage(QRectF(picture.center() - QPointF(shown.width(), shown.height()) / 2.0, shown), m_scaled);
            p.setRenderHint(QPainter::SmoothPixmapTransform, false);
        } else {
            const QPointF topLeft(picture.center().x() - m_scaled.width() / m_scaledDpr / 2.0,
                                  picture.center().y() - m_scaled.height() / m_scaledDpr / 2.0);
            p.drawImage(topLeft, m_scaled);
        }
        // What was showing, drawn back as what is showing wipes in over it.
        if (m_wipe >= 0.0 && !m_wipeFrom.isNull()) {
            const qreal edge = card.left() + card.width() * m_wipe;
            p.setClipRect(QRectF(edge, card.top(), card.right() - edge, card.height()), Qt::IntersectClip);
            p.drawImage(card, m_wipeFrom);
            keepInside();
            p.fillRect(QRectF(edge - 1.0, card.top(), 2.0, card.height()), Qt::white);
        }
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
    if (!m_compareFrame.isNull() && !m_frame.isNull()) {
        // What it was, to the left of the moving line; what it is, to its right.
        const qreal edge = card.left() + card.width() * m_compareAt;
        p.save();
        keepInside();
        p.save();
        p.setClipRect(QRectF(card.left(), card.top(), edge - card.left(), card.height()), Qt::IntersectClip);
        // Made at the picture's size once: the line moves every frame, and only
        // what it uncovers is drawn, as a copy.
        const qreal ratio = std::max(1.0, devicePixelRatioF());
        const QSize target = m_compareFrame.size().scaled(QSize(std::max(1, int(std::round(picture.width() * ratio))),
                                                                std::max(1, int(std::round(picture.height() * ratio)))),
                                                          Qt::KeepAspectRatio);
        if (m_compareScaled.size() != target) {
            m_compareScaled = m_compareFrame.scaled(target, Qt::IgnoreAspectRatio, Qt::SmoothTransformation).convertToFormat(QImage::Format_RGB32);
            m_compareScaled.setDevicePixelRatio(ratio);
        }
        p.drawImage(QPointF(picture.center().x() - target.width() / ratio / 2.0, picture.center().y() - target.height() / ratio / 2.0), m_compareScaled);
        p.restore();
        // A soft light either side of the line, in one fill: eight faint bands laid
        // over one another would blend each pixel eight times in every frame.
        QLinearGradient glow(QPointF(edge - 8.0, 0.0), QPointF(edge + 8.0, 0.0));
        glow.setColorAt(0.0, QColor(255, 255, 255, 0));
        glow.setColorAt(0.25, QColor(255, 255, 255, 24));
        glow.setColorAt(0.5, QColor(255, 255, 255, 82));
        glow.setColorAt(0.75, QColor(255, 255, 255, 24));
        glow.setColorAt(1.0, QColor(255, 255, 255, 0));
        p.setRenderHint(QPainter::Antialiasing, false);
        p.fillRect(QRectF(edge - 8.0, card.top(), 16.0, card.height()), glow);
        p.fillRect(QRectF(edge - 1.0, card.top(), 2.0, card.height()), Qt::white);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QFont font = tokens::font(tokens::meta);
        const QFontMetricsF metrics(font);
        p.setFont(font);
        const auto tag = [&](const QString &text, bool left, const QColor &fill) {
            const qreal w = metrics.horizontalAdvance(text) + 16;
            const QRectF box(left ? card.left() + 12 : card.right() - 12 - w, card.bottom() - 12 - 22, w, 22);
            p.setPen(Qt::NoPen);
            p.setBrush(fill);
            p.drawRoundedRect(box, 6, 6);
            p.setPen(Qt::white);
            p.drawText(box, Qt::AlignCenter, text);
        };
        tag(QStringLiteral("Before"), true, QColor(0, 0, 0, 153));
        tag(QStringLiteral("After"), false, tokens::color(tokens::accent));
        p.restore();
    }
    // Over the picture, inside its rect: the caption, the QC overlap box,
    // safe zones and the comments' drawings; the masks' outlines on top.
    p.save();
    keepInside();
    paintCaption(p, m_overlapShown || m_overlapLevel > 0.0);
    paintOverlap(p);
    if (m_safeZones) paintSafeZones(p, picture);
    paintAnnotations(p);
    p.restore();
    paintMasks(p);
    paintMotionPath(p);
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
