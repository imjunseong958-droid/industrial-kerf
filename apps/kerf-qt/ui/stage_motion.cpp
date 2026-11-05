#include "stage_motion.h"

#include "components.h"
#include "design_tokens.h"

#include <QEasingCurve>
#include <QFontMetricsF>
#include <QGraphicsEffect>
#include <QGraphicsOpacityEffect>
#include <QHideEvent>
#include <QPixmap>
#include <QRegion>
#include <QResizeEvent>
#include <QShowEvent>
#include <QTimer>
#include <QPainter>
#include <QPainterPath>
#include <QPaintEvent>
#include <QPropertyAnimation>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

// A tile's name, on a dark chip at its foot.
void paintTileLabel(QPainter &p, const QRectF &box, const QString &label) {
    const QFont font = tokens::font(tokens::meta);
    const QFontMetricsF metrics(font);
    const QString text = metrics.elidedText(label, Qt::ElideRight, std::max(0.0, box.width() - 20.0));
    if (text.isEmpty() || box.height() < 24) return;
    const QRectF chip(box.left() + 5, box.bottom() - 21, metrics.horizontalAdvance(text) + 10, 17);
    p.save();
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(0, 0, 0, 153));
    p.drawRoundedRect(chip, 4, 4);
    p.setFont(font);
    p.setPen(Qt::white);
    p.drawText(chip, Qt::AlignCenter, text);
    p.restore();
}

// An image as it is cheapest to draw over and over: no bigger than it will be
// shown, and in the format the raster engine has its fast paths for.
QImage readyToMove(QImage image, const QSizeF &largest, qreal ratio, bool whole) {
    if (image.isNull()) return image;
    QSize wanted = (largest * ratio).toSize().expandedTo(QSize(1, 1));
    wanted = image.size().scaled(wanted, whole ? Qt::KeepAspectRatio : Qt::KeepAspectRatioByExpanding);
    if (wanted.width() * 3 < image.width() * 2) {
        const qreal own = image.devicePixelRatio();
        image = image.scaled(wanted, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
        image.setDevicePixelRatio(own);
    }
    const QImage::Format fast = image.hasAlphaChannel() ? QImage::Format_ARGB32_Premultiplied : QImage::Format_RGB32;
    return image.format() == fast ? image : image.convertToFormat(fast);
}

constexpr int kSheetSide = 16;
constexpr int kSheetColumns = 4;
constexpr int kSheetGap = 8;

} // namespace

FootageSheet::FootageSheet(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("footageSheet"));
    setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
    m_pop.setStartValue(0.0);
    m_pop.setEndValue(1.0);
    connect(&m_pop, &QVariantAnimation::valueChanged, this, [this] { update(); });
}

void FootageSheet::setTiles(const QList<FootageTile> &tiles) {
    m_tiles = tiles;
    for (FootageTile &tile : m_tiles) tile.poster = readyToMove(tile.poster, QSizeF(tokens::motion::posterMost, tokens::motion::posterMost) / 2.0, 1.0, false);
    m_taken.clear();
    setFixedHeight(heightForWidth(width()));
    const int length = ui::motionDuration(int(tiles.size()) * tokens::motion::popStagger + tokens::motion::pop);
    m_pop.stop();
    if (length > 0) {
        m_pop.setDuration(length);
        m_pop.start();
    }
    update();
}

QStringList FootageSheet::ids() const {
    QStringList out;
    for (const FootageTile &tile : m_tiles) out << tile.id;
    return out;
}

int FootageSheet::heightForWidth(int width) const {
    const int rows = (int(m_tiles.size()) + kSheetColumns - 1) / kSheetColumns;
    const int tile = std::max(1, (width - 2 * kSheetSide - (kSheetColumns - 1) * kSheetGap) / kSheetColumns);
    return rows * int(tile * 0.5625) + std::max(0, rows - 1) * kSheetGap;
}

QRect FootageSheet::tileRect(const QString &id) const {
    const int tile = std::max(1, (width() - 2 * kSheetSide - (kSheetColumns - 1) * kSheetGap) / kSheetColumns);
    const int tall = int(tile * 0.5625);
    for (int index = 0; index < m_tiles.size(); ++index)
        if (m_tiles.at(index).id == id)
            return QRect(kSheetSide + (index % kSheetColumns) * (tile + kSheetGap), (index / kSheetColumns) * (tall + kSheetGap), tile, tall);
    return {};
}

void FootageSheet::take(const QString &id) {
    if (m_taken.contains(id)) return;
    m_taken << id;
    update();
}

void FootageSheet::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    setFixedHeight(heightForWidth(width()));
}

void FootageSheet::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    const bool popping = m_pop.state() == QAbstractAnimation::Running;
    const qreal elapsed = popping ? m_pop.currentTime() : 1e9;
    static const QEasingCurve overshoot = [] {
        QEasingCurve curve(QEasingCurve::OutBack);
        curve.setOvershoot(tokens::motion::popOvershoot);
        return curve;
    }();
    for (int index = 0; index < m_tiles.size(); ++index) {
        const FootageTile &tile = m_tiles.at(index);
        if (m_taken.contains(tile.id)) continue;
        const qreal t = std::clamp((elapsed - index * tokens::motion::popStagger) / tokens::motion::pop, 0.0, 1.0);
        if (t <= 0.0) continue;
        const QRectF home = tileRect(tile.id);
        const qreal scale = 0.8 + 0.2 * overshoot.valueForProgress(t);
        const QRectF box(home.center() - QPointF(home.width(), home.height()) * scale / 2.0, home.size() * scale);
        p.save();
        p.setOpacity(std::min(1.0, t * 2.0));
        QPainterPath clip;
        clip.addRoundedRect(box, 7, 7);
        p.setClipPath(clip);
        QSizeF shown = tile.poster.size();
        shown.scale(box.size(), Qt::KeepAspectRatioByExpanding);
        p.drawImage(QRectF(box.center() - QPointF(shown.width(), shown.height()) / 2.0, shown), tile.poster);
        paintTileLabel(p, box, tile.label);
        p.restore();
    }
}

WorkChip::WorkChip(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("workChip"));
    setAttribute(Qt::WA_TransparentForMouseEvents);
    hide();
    m_pulse.setStartValue(0.0);
    m_pulse.setEndValue(1.0);
    m_pulse.setDuration(tokens::motion::workPulse);
    m_pulse.setLoopCount(-1);
    // Over the picture, for as long as Kerf works: only the dot, and only when it
    // has changed enough to see.
    connect(&m_pulse, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        const int step = qRound(value.toReal() * tokens::motion::workPulseSteps);
        if (step == m_pulseStep) return;
        m_pulseStep = step;
        update(QRect(9, height() / 2 - 5, 10, 10));
    });
}

QSize WorkChip::sizeHint() const {
    const QFontMetrics metrics(tokens::font(tokens::meta));
    return QSize(metrics.horizontalAdvance(QStringLiteral("Kerf is looking")) + 34, 24);
}

void WorkChip::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    if (!ui::reducedMotion()) m_pulse.start();
}

void WorkChip::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    m_pulse.stop();
}

void WorkChip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(0, 0, 0, 140));
    p.drawRoundedRect(QRectF(rect()), height() / 2.0, height() / 2.0);
    const qreal at = qreal(m_pulseStep) / tokens::motion::workPulseSteps;
    p.setOpacity(1.0 - 0.65 * (1.0 - std::abs(at * 2.0 - 1.0)));
    p.setBrush(tokens::color(tokens::accentInk));
    p.drawEllipse(QPointF(14, height() / 2.0), 3, 3);
    p.setOpacity(1.0);
    p.setFont(tokens::font(tokens::meta));
    p.setPen(Qt::white);
    p.drawText(QRect(24, 0, width() - 24, height()), Qt::AlignVCenter | Qt::AlignLeft, QStringLiteral("Kerf is looking"));
}

StageMotion::StageMotion(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageMotion"));
    setAttribute(Qt::WA_TransparentForMouseEvents);
    setAttribute(Qt::WA_TranslucentBackground);
    hide();
    m_ticker.setStartValue(0.0);
    m_ticker.setEndValue(1.0);
    m_ticker.setDuration(1000);
    m_ticker.setLoopCount(-1);
    connect(&m_ticker, &QVariantAnimation::valueChanged, this, &StageMotion::tick);
    m_clock.start();
}

bool StageMotion::fly(const Flight &flight) {
    if (ui::reducedMotion()) return false;
    Running running{flight, m_clock.elapsed() + flight.delay};
    // Prepared once, as it starts, not in each of its frames.
    const QSizeF largest = flight.from.size().expandedTo(flight.to.size());
    running.flight.image = readyToMove(flight.image, largest, std::max(1.0, devicePixelRatioF()), flight.fit);
    m_flights << running;
    show();
    raise();
    wake();
    return true;
}

void StageMotion::wake() {
    if (m_ticker.state() != QAbstractAnimation::Running) m_ticker.start();
    tick();
}

void StageMotion::releaseStaying() {
    const qint64 now = m_clock.elapsed();
    bool any = false;
    for (Running &running : m_flights) {
        if (!running.flight.stays) continue;
        any = true;
        running.flight.stays = false;
        running.atRest = false;
        // Still on its way or still coming in, it goes on and fades after: nothing
        // jumps to where it was going.
        if (now - running.started < running.flight.duration) {
            running.flight.tail = tokens::motion::standInRelease;
            continue;
        }
        running.flight.from = running.flight.to;
        running.flight.fromOpacity = running.flight.toOpacity;
        running.flight.toOpacity = 0.0;
        running.flight.duration = tokens::motion::standInRelease;
        running.started = now;
    }
    if (any) wake();
}

void StageMotion::followPicture(const QRect &picture) {
    if (picture.isEmpty()) {
        releaseStaying();
        return;
    }
    m_now = m_clock.elapsed();
    bool any = false;
    for (Running &running : m_flights) {
        Flight &flight = running.flight;
        if (!flight.stays || !flight.fit || flight.to == QRectF(picture)) continue;
        any = true;
        running.atRest = false;
        // It has not started: it only starts for somewhere else.
        if (m_now < running.started) {
            flight.to = picture;
            continue;
        }
        const qreal opacity = opacityOf(running);
        const int left = int(flight.duration - (m_now - running.started));
        flight.from = boxOf(running);
        flight.to = picture;
        flight.fromOpacity = opacity;
        flight.lively = false;
        flight.duration = std::max(left, tokens::motion::move);
        running.started = m_now;
    }
    if (any) wake();
}

QRectF StageMotion::boxOf(const Running &running) const {
    static const QEasingCurve soft = tokens::easing();
    static const QEasingCurve lively = [] {
        QEasingCurve curve(QEasingCurve::OutBack);
        curve.setOvershoot(tokens::motion::landOvershoot);
        return curve;
    }();
    const Flight &flight = running.flight;
    const qreal t = std::clamp(qreal(m_now - running.started) / std::max(1, flight.duration), 0.0, 1.0);
    const qreal at = (flight.lively ? lively : soft).valueForProgress(t);
    return QRectF(flight.from.left() + (flight.to.left() - flight.from.left()) * at,
                  flight.from.top() + (flight.to.top() - flight.from.top()) * at,
                  flight.from.width() + (flight.to.width() - flight.from.width()) * at,
                  flight.from.height() + (flight.to.height() - flight.from.height()) * at);
}

qreal StageMotion::opacityOf(const Running &running) const {
    const Flight &flight = running.flight;
    const qint64 elapsed = m_now - running.started;
    const qreal t = std::clamp(qreal(elapsed) / std::max(1, flight.duration), 0.0, 1.0);
    const qreal opacity = flight.fromOpacity + (flight.toOpacity - flight.fromOpacity) * t;
    if (flight.tail <= 0 || elapsed <= flight.duration) return opacity;
    return opacity * (1.0 - std::min(1.0, qreal(elapsed - flight.duration) / flight.tail));
}

void StageMotion::tick() {
    m_now = m_clock.elapsed();
    // Only where something changed: the layer is clear, so what is marked is drawn
    // again in every region under it.
    QRegion dirty;
    bool moving = false;
    for (auto running = m_flights.begin(); running != m_flights.end();) {
        const Flight &flight = running->flight;
        const qint64 elapsed = m_now - running->started;
        if (!flight.stays && elapsed >= flight.duration + flight.tail) {
            dirty += running->drawn;
            running = m_flights.erase(running);
            continue;
        }
        if (elapsed < 0) {
            moving = true;
            ++running;
            continue;
        }
        const bool still = flight.stays && elapsed >= flight.duration;
        if (!running->atRest) {
            // Its ring and its edge's smoothing lie just outside the box.
            const QRect box = boxOf(*running).toAlignedRect().adjusted(-3, -3, 3, 3);
            dirty += running->drawn;
            dirty += box;
            running->drawn = box;
        }
        running->atRest = still;
        moving = moving || !still;
        ++running;
    }
    if (m_flights.isEmpty()) {
        m_ticker.stop();
        hide();
        return;
    }
    if (!dirty.isEmpty()) update(dirty);
    // What is left holds still until it is released or sent on, however long that is.
    if (!moving) m_ticker.stop();
}

void StageMotion::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    for (const Running &running : std::as_const(m_flights)) {
        const Flight &flight = running.flight;
        if (m_now < running.started || !event->region().intersects(running.drawn)) continue;
        const QRectF box = boxOf(running);
        p.save();
        p.setOpacity(opacityOf(running));
        if (flight.radius > 0) {
            QPainterPath clip;
            clip.addRoundedRect(box, flight.radius, flight.radius);
            p.setClipPath(clip);
        } else {
            p.setClipRect(box);
        }
        if (flight.image.isNull()) {
            p.fillRect(box, tokens::color(tokens::on));
        } else {
            // It fills its box, as a clip's poster does, unless it stands in for the picture.
            if (flight.fit) p.fillRect(box, Qt::black);
            QSizeF shown = flight.image.size();
            shown.scale(box.size(), flight.fit ? Qt::KeepAspectRatio : Qt::KeepAspectRatioByExpanding);
            p.drawImage(QRectF(box.center() - QPointF(shown.width(), shown.height()) / 2.0, shown), flight.image);
        }
        if (!flight.label.isEmpty()) paintTileLabel(p, box, flight.label);
        p.setClipping(false);
        if (flight.ring) {
            p.setPen(QPen(tokens::color(tokens::accentInk), 2.0));
            p.setBrush(Qt::NoBrush);
            p.drawRoundedRect(box, flight.radius, flight.radius);
        }
        p.restore();
    }
}

namespace moves {

namespace {

// What is under way on `effect` gives way to what comes next; two at once would
// fight over each frame, and the first to end would take the effect from the other.
void stopFades(QGraphicsEffect *effect) {
    const QList<QPropertyAnimation *> fades = effect->findChildren<QPropertyAnimation *>(QString(), Qt::FindDirectChildrenOnly);
    for (QPropertyAnimation *fade : fades) {
        fade->stop();
        fade->deleteLater();
    }
}

// A widget drawn lower than it is, and how much of it shows.
class RiseEffect final : public QGraphicsEffect {
    Q_OBJECT
    Q_PROPERTY(qreal offset READ offset WRITE setOffset)
    Q_PROPERTY(qreal level READ level WRITE setLevel)
public:
    using QGraphicsEffect::QGraphicsEffect;
    qreal offset() const { return m_offset; }
    void setOffset(qreal offset) {
        m_offset = offset;
        updateBoundingRect();
    }
    qreal level() const { return m_level; }
    void setLevel(qreal level) {
        m_level = level;
        update();
    }
    QRectF boundingRectFor(const QRectF &rect) const override { return rect.adjusted(0, 0, 0, std::ceil(m_offset)); }

protected:
    void draw(QPainter *painter) override {
        if (m_level <= 0.0) return;
        QPoint at;
        const QPixmap shown = sourcePixmap(Qt::LogicalCoordinates, &at, NoPad);
        if (shown.isNull()) return;
        painter->save();
        painter->setOpacity(painter->opacity() * m_level);
        painter->setRenderHint(QPainter::SmoothPixmapTransform, true);
        painter->translate(0, m_offset);
        painter->drawPixmap(at, shown);
        painter->restore();
    }

private:
    qreal m_offset = 0.0;
    qreal m_level = 0.0;
};

} // namespace

void fadeIn(QWidget *widget, int delay, int duration) {
    const int length = ui::motionDuration(duration);
    if (!widget || length <= 0) return;
    auto *effect = new QGraphicsOpacityEffect(widget);
    effect->setOpacity(0.0);
    widget->setGraphicsEffect(effect);
    auto *fade = new QPropertyAnimation(effect, "opacity", effect);
    fade->setDuration(length);
    fade->setStartValue(0.0);
    fade->setEndValue(1.0);
    fade->setEasingCurve(QEasingCurve::OutCubic);
    QObject::connect(fade, &QAbstractAnimation::finished, widget, [widget, effect] {
        if (widget->graphicsEffect() == effect) widget->setGraphicsEffect(nullptr);
    });
    QTimer::singleShot(delay, fade, [fade] { fade->start(QAbstractAnimation::DeleteWhenStopped); });
}

void rise(QWidget *widget, int delay, int distance, int duration) {
    const int length = ui::motionDuration(duration);
    if (!widget || length <= 0) return;
    auto *effect = new RiseEffect(widget);
    effect->setOffset(distance);
    widget->setGraphicsEffect(effect);
    auto *slide = new QPropertyAnimation(effect, "offset", effect);
    slide->setDuration(length);
    slide->setStartValue(qreal(distance));
    slide->setEndValue(0.0);
    slide->setEasingCurve(tokens::easing());
    auto *fade = new QPropertyAnimation(effect, "level", effect);
    fade->setDuration(length / 2);
    fade->setStartValue(0.0);
    fade->setEndValue(1.0);
    fade->setEasingCurve(QEasingCurve::OutCubic);
    QObject::connect(slide, &QAbstractAnimation::finished, widget, [widget, effect] {
        if (widget->graphicsEffect() == effect) widget->setGraphicsEffect(nullptr);
    });
    QTimer::singleShot(delay, effect, [slide, fade] {
        slide->start(QAbstractAnimation::DeleteWhenStopped);
        fade->start(QAbstractAnimation::DeleteWhenStopped);
    });
}

void dim(QWidget *widget, qreal level, int duration) {
    if (!widget) return;
    auto *effect = qobject_cast<QGraphicsOpacityEffect *>(widget->graphicsEffect());
    if (effect) stopFades(effect);
    const int length = ui::motionDuration(duration);
    if (length <= 0) {
        // Under reduced motion it takes the level at once.
        if (level >= 1.0) {
            widget->setGraphicsEffect(nullptr);
        } else {
            if (!effect) {
                effect = new QGraphicsOpacityEffect(widget);
                widget->setGraphicsEffect(effect);
            }
            effect->setOpacity(level);
        }
        return;
    }
    if (!effect) {
        if (level >= 1.0) return;
        effect = new QGraphicsOpacityEffect(widget);
        effect->setOpacity(1.0);
        widget->setGraphicsEffect(effect);
    }
    auto *fade = new QPropertyAnimation(effect, "opacity", effect);
    fade->setDuration(length);
    fade->setStartValue(effect->opacity());
    fade->setEndValue(level);
    fade->setEasingCurve(tokens::easing());
    if (level >= 1.0) {
        QObject::connect(fade, &QAbstractAnimation::finished, widget, [widget, effect] {
            if (widget->graphicsEffect() == effect) widget->setGraphicsEffect(nullptr);
        });
    }
    fade->start(QAbstractAnimation::DeleteWhenStopped);
}

} // namespace moves

} // namespace kerf

#include "stage_motion.moc"
