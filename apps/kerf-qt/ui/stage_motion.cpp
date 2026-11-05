#include "stage_motion.h"

#include "components.h"
#include "design_tokens.h"

#include <QEasingCurve>
#include <QFontMetricsF>
#include <QHideEvent>
#include <QResizeEvent>
#include <QShowEvent>
#include <QGraphicsOpacityEffect>
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
    connect(&m_pulse, &QVariantAnimation::valueChanged, this, [this] { update(); });
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
    const qreal at = m_pulse.currentValue().toReal();
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
    m_timer.setInterval(16);
    connect(&m_timer, &QTimer::timeout, this, &StageMotion::tick);
    m_clock.start();
}

bool StageMotion::fly(const Flight &flight) {
    if (ui::reducedMotion()) return false;
    m_flights << Running{flight, m_clock.elapsed() + flight.delay};
    show();
    raise();
    if (!m_timer.isActive()) m_timer.start();
    update();
    return true;
}

void StageMotion::releaseStaying() {
    const qint64 now = m_clock.elapsed();
    for (Running &running : m_flights) {
        if (!running.flight.stays) continue;
        running.flight.stays = false;
        running.flight.from = running.flight.to;
        running.flight.fromOpacity = running.flight.toOpacity;
        running.flight.toOpacity = 0.0;
        running.flight.duration = tokens::motion::standInRelease;
        running.started = now;
    }
}

void StageMotion::tick() {
    const qint64 now = m_clock.elapsed();
    m_flights.erase(std::remove_if(m_flights.begin(), m_flights.end(),
                                   [now](const Running &running) {
                                       return !running.flight.stays && now - running.started >= running.flight.duration + running.flight.tail;
                                   }),
                    m_flights.end());
    if (m_flights.isEmpty()) {
        m_timer.stop();
        hide();
        return;
    }
    update();
}

void StageMotion::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    const qint64 now = m_clock.elapsed();
    static const QEasingCurve soft = tokens::easing();
    static const QEasingCurve lively = [] {
        QEasingCurve curve(QEasingCurve::OutBack);
        curve.setOvershoot(tokens::motion::landOvershoot);
        return curve;
    }();
    for (const Running &running : std::as_const(m_flights)) {
        const Flight &flight = running.flight;
        const qint64 elapsed = now - running.started;
        if (elapsed < 0) continue;
        const qreal t = std::clamp(qreal(elapsed) / std::max(1, flight.duration), 0.0, 1.0);
        const qreal at = (flight.lively ? lively : soft).valueForProgress(t);
        const QRectF box(flight.from.left() + (flight.to.left() - flight.from.left()) * at,
                         flight.from.top() + (flight.to.top() - flight.from.top()) * at,
                         flight.from.width() + (flight.to.width() - flight.from.width()) * at,
                         flight.from.height() + (flight.to.height() - flight.from.height()) * at);
        p.save();
        qreal opacity = flight.fromOpacity + (flight.toOpacity - flight.fromOpacity) * t;
        if (flight.tail > 0 && elapsed > flight.duration) opacity *= 1.0 - std::min(1.0, qreal(elapsed - flight.duration) / flight.tail);
        p.setOpacity(opacity);
        QPainterPath clip;
        clip.addRoundedRect(box, flight.radius, flight.radius);
        p.setClipPath(clip);
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

void fadeIn(QWidget *widget, int delay, int duration) {
    const int length = ui::motionDuration(duration);
    if (!widget || length <= 0) return;
    auto *effect = new QGraphicsOpacityEffect(widget);
    effect->setOpacity(0.0);
    widget->setGraphicsEffect(effect);
    auto *fade = new QPropertyAnimation(effect, "opacity", widget);
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
    const QPoint home = widget->pos();
    widget->move(home + QPoint(0, distance));
    auto *slide = new QPropertyAnimation(widget, "pos", widget);
    slide->setDuration(length);
    slide->setStartValue(home + QPoint(0, distance));
    slide->setEndValue(home);
    slide->setEasingCurve(tokens::easing());
    QTimer::singleShot(delay, slide, [slide] { slide->start(QAbstractAnimation::DeleteWhenStopped); });
    fadeIn(widget, delay, duration / 2);
}

void dim(QWidget *widget, qreal level, int duration) {
    if (!widget) return;
    auto *effect = qobject_cast<QGraphicsOpacityEffect *>(widget->graphicsEffect());
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
    auto *fade = new QPropertyAnimation(effect, "opacity", widget);
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
