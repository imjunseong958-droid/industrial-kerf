#include "activity.h"

#include "components.h"

#include <QHideEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QRegularExpression>
#include <QShowEvent>
#include <QTimer>

#include <algorithm>
#include <cmath>

namespace kerf::activity {

QRgb hueColor(Hue hue) {
    switch (hue) {
    case Hue::Kerf: return tokens::activity::kerf;
    case Hue::Gpu: return tokens::activity::gpu;
    case Hue::None: break;
    }
    return tokens::ink3;
}

MakingLine parseMakingLine(const QString &line) {
    MakingLine out;
    const QString said = line.trimmed();
    if (said.isEmpty()) return out;
    const auto number = [](const QRegularExpressionMatch &match, int group) {
        bool ok = false;
        const int value = match.captured(group).toInt(&ok);
        return ok ? value : -1;
    };
    static const QRegularExpression asked(QStringLiteral("^asked\\b"), QRegularExpression::CaseInsensitiveOption);
    static const QRegularExpression waiting(QStringLiteral("^waiting\\b(?:.*?(\\d+)\\s+ahead)?(?:.*?(\\d+)\\s*s\\b)?"),
                                            QRegularExpression::CaseInsensitiveOption);
    static const QRegularExpression running(QStringLiteral("^running\\b(?:.*?(\\d+)\\s*s\\b)?"), QRegularExpression::CaseInsensitiveOption);
    static const QRegularExpression rendering(QStringLiteral("^rendering\\b(?:.*?(\\d+(?:\\.\\d+)?)\\s*%)?"),
                                              QRegularExpression::CaseInsensitiveOption);
    static const QRegularExpression made(QStringLiteral("^made\\b(?:.*?(\\d+)\\s*s\\b)?"), QRegularExpression::CaseInsensitiveOption);
    if (asked.match(said).hasMatch()) {
        out.phase = MakingLine::Phase::Asked;
    } else if (const QRegularExpressionMatch match = waiting.match(said); match.hasMatch()) {
        out.phase = MakingLine::Phase::Waiting;
        out.ahead = number(match, 1);
        out.seconds = number(match, 2);
    } else if (const QRegularExpressionMatch match = running.match(said); match.hasMatch()) {
        out.phase = MakingLine::Phase::Running;
        out.seconds = number(match, 1);
    } else if (const QRegularExpressionMatch match = rendering.match(said); match.hasMatch()) {
        out.phase = MakingLine::Phase::Rendering;
        bool ok = false;
        const double share = match.captured(1).toDouble(&ok);
        if (ok) out.percent = std::clamp(int(std::floor(share)), 0, 100);
    } else if (const QRegularExpressionMatch match = made.match(said); match.hasMatch()) {
        out.phase = MakingLine::Phase::Made;
        out.seconds = number(match, 1);
    } else {
        out.phase = MakingLine::Phase::Other;
    }
    return out;
}

qint64 clock() {
    static QElapsedTimer timer;
    if (!timer.isValid()) timer.start();
    return timer.elapsed();
}

QString elapsedText(qint64 ms) {
    const qint64 seconds = std::max<qint64>(0, ms) / 1000;
    const qint64 hours = seconds / 3600;
    const qint64 minutes = (seconds / 60) % 60;
    const qint64 rest = seconds % 60;
    if (hours > 0) return QStringLiteral("%1:%2:%3").arg(hours).arg(minutes, 2, 10, QLatin1Char('0')).arg(rest, 2, 10, QLatin1Char('0'));
    return QStringLiteral("%1:%2").arg(minutes).arg(rest, 2, 10, QLatin1Char('0'));
}

QString clipLabel(const MediaActivity &activity, qint64 now) {
    using State = MediaActivity::State;
    switch (activity.state) {
    case State::Making: return QStringLiteral("Making %1").arg(elapsedText(now - activity.since));
    case State::Queued: return QStringLiteral("Queued");
    case State::NotMade: return QStringLiteral("Not made");
    case State::Offline: return QStringLiteral("Offline");
    case State::Ready: break;
    }
    return {};
}

Stage stageFor(const Work &work, qint64 now) {
    Stage stage;
    if (!work.makings.isEmpty()) {
        stage.hue = Hue::Gpu;
        // The one making, ahead of those waiting their turn.
        const Work::Named *shown = &work.makings.constFirst();
        for (const Work::Named &named : work.makings)
            if (!parseMakingLine(named.making.line).queued()) {
                shown = &named;
                break;
            }
        const MakingLine line = parseMakingLine(shown->making.line);
        QString chip = QStringLiteral("Making %1 · ").arg(shown->name);
        if (line.queued()) chip += line.ahead > 0 ? QStringLiteral("queued, %1 ahead").arg(line.ahead) : QStringLiteral("queued");
        else chip += elapsedText(now - shown->making.since);
        if (work.makings.size() > 1) chip += QStringLiteral(" +%1").arg(work.makings.size() - 1);
        stage.chip = chip;
    } else if (!work.render.isEmpty()) {
        stage.hue = Hue::Gpu;
        stage.chip = QStringLiteral("%1 %2%").arg(work.render).arg(int(std::floor(std::clamp(work.fraction, 0.0, 1.0) * 100.0)));
    } else if (work.agentBusy) {
        stage.hue = Hue::Kerf;
        stage.chip = work.cutting ? QStringLiteral("Kerf is cutting") : QStringLiteral("Kerf is looking");
    }
    return stage;
}

void paintStripes(QPainter &painter, const QRectF &rect, qreal phase, const QColor &color) {
    if (rect.isEmpty()) return;
    const qreal space = tokens::motion::stripeSpace;
    painter.save();
    painter.setClipRect(rect, Qt::IntersectClip);
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setPen(Qt::NoPen);
    painter.setBrush(color);
    // Bands leaning right, half a space wide, from before the left edge by the
    // rect's height so the first reaches its foot.
    const qreal h = rect.height();
    const qreal shift = std::fmod(phase, 1.0) * space;
    for (qreal x = rect.left() - h - space + shift; x < rect.right() + space; x += space) {
        const QPolygonF band{QPointF(x, rect.bottom()), QPointF(x + space / 2, rect.bottom()), QPointF(x + space / 2 + h, rect.top()),
                             QPointF(x + h, rect.top())};
        painter.drawPolygon(band);
    }
    painter.restore();
}

qreal stripePhase(qint64 now) {
    if (ui::reducedMotion()) return 0.0;
    return qreal(now % tokens::motion::stripeCycle) / tokens::motion::stripeCycle;
}

// ================================================================ EdgeLight

namespace {

// Points along the ring, about this far apart.
constexpr qreal kRingStep = 3.0;
// The tail is drawn as this many runs, brightest first.
constexpr int kTailRuns = 8;

} // namespace

EdgeLight::EdgeLight(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("edgeLight"));
    setAttribute(Qt::WA_TransparentForMouseEvents);
    setFocusPolicy(Qt::NoFocus);
    m_clock.start();
    m_ticker = new QTimer(this);
    m_ticker->setInterval(tokens::motion::edgeFrame);
    connect(m_ticker, &QTimer::timeout, this, &EdgeLight::tick);
}

void EdgeLight::setFrame(const QRectF &picture, qreal radius) {
    if (picture == m_frame && radius == m_radius) return;
    if (isLit()) update(band());
    m_frame = picture;
    m_radius = radius;
    m_lit = QRegion();
    if (isLit()) update(band());
    follow();
}

qreal EdgeLight::level() const {
    if (m_changed < 0) return m_to;
    const qreal t = std::clamp(qreal(m_clock.elapsed() - m_changed) / tokens::motion::edgeFade, 0.0, 1.0);
    static const QEasingCurve curve = tokens::easing();
    return m_from + (m_to - m_from) * curve.valueForProgress(t);
}

bool EdgeLight::isLit() const { return m_hue != Hue::None || level() > 0.0; }

bool EdgeLight::isTicking() const { return m_ticker->isActive(); }

void EdgeLight::setHue(Hue hue) {
    if (hue == m_hue) return;
    const qreal was = level();
    m_hue = hue;
    if (hue != Hue::None) m_shown = hue;
    const qreal target = hue == Hue::None ? 0.0 : 1.0;
    if (ui::reducedMotion() || !isVisible() || m_frame.isEmpty()) {
        m_from = m_to = target;
        m_changed = -1;
    } else {
        m_from = was;
        m_to = target;
        m_changed = m_clock.elapsed();
    }
    if (hue == Hue::None && m_changed < 0) m_shown = Hue::None;
    update(band());
    follow();
}

void EdgeLight::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    follow();
}

void EdgeLight::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    m_ticker->stop();
}

// Frames only while it moves, and only while it can be seen.
void EdgeLight::follow() {
    const bool moving = isVisible() && !m_frame.isEmpty() && isLit() && (!ui::reducedMotion() || m_changed >= 0);
    if (moving && !m_ticker->isActive()) m_ticker->start();
    if (!moving) m_ticker->stop();
}

QRegion EdgeLight::band() const {
    if (m_frame.isEmpty()) return {};
    const int reach = int(std::ceil(tokens::motion::edgeGap + tokens::motion::edgeWidth)) + 2;
    const QRect outer = m_frame.toAlignedRect().adjusted(-reach, -reach, reach, reach);
    const int corner = int(std::ceil(m_radius)) + reach;
    // The ring and a little either side of it; not the picture inside it.
    return QRegion(outer) - QRegion(outer.adjusted(corner, reach * 2, -corner, -reach * 2)) -
           QRegion(outer.adjusted(reach * 2, corner, -reach * 2, -corner));
}

const QList<QPointF> &EdgeLight::ring() const {
    if (m_ringFor == m_frame && !m_ring.isEmpty()) return m_ring;
    m_ringFor = m_frame;
    m_ring.clear();
    if (m_frame.isEmpty()) return m_ring;
    const qreal out = tokens::motion::edgeGap + tokens::motion::edgeWidth / 2;
    QPainterPath path;
    path.addRoundedRect(m_frame.adjusted(-out, -out, out, out), m_radius + out, m_radius + out);
    const int count = std::max(16, int(path.length() / kRingStep));
    m_ring.reserve(count);
    for (int index = 0; index < count; ++index) m_ring << path.pointAtPercent(qreal(index) / count);
    return m_ring;
}

QList<QPolygonF> EdgeLight::comet(qreal turn) const {
    const QList<QPointF> &points = ring();
    QList<QPolygonF> runs;
    const int count = int(points.size());
    if (count == 0) return runs;
    const int head = int(turn * count) % count;
    const int tail = std::max(kTailRuns, int(tokens::motion::edgeTail * count));
    const int each = std::max(1, tail / kTailRuns);
    for (int run = 0; run < kTailRuns; ++run) {
        QPolygonF line;
        for (int step = 0; step <= each; ++step) line << points.at(((head - run * each - step) % count + count) % count);
        runs << line;
    }
    return runs;
}

void EdgeLight::tick() {
    const bool fading = m_changed >= 0 && m_clock.elapsed() - m_changed < tokens::motion::edgeFade;
    if (fading) {
        // All of the ring changes as it comes and goes: only the ring is drawn again.
        update(band());
    } else {
        if (m_changed >= 0) {
            m_changed = -1;
            m_from = m_to;
            if (m_hue == Hue::None) m_shown = Hue::None;
            update(band());
        }
        if (m_hue != Hue::None && !ui::reducedMotion()) {
            // Where the light was and is, and nothing between.
            const qreal turn = qreal(clock() % tokens::motion::edgeTurn) / tokens::motion::edgeTurn;
            QRegion lit;
            for (const QPolygonF &run : comet(turn)) lit += run.boundingRect().toAlignedRect().adjusted(-3, -3, 3, 3);
            update(m_lit + lit);
            m_lit = lit;
        }
    }
    follow();
}

void EdgeLight::paintEvent(QPaintEvent *) {
    const qreal shown = level();
    if (shown <= 0.0 || m_frame.isEmpty() || m_shown == Hue::None) return;
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setOpacity(shown);
    const QColor hue = tokens::color(hueColor(m_shown));
    const qreal out = tokens::motion::edgeGap + tokens::motion::edgeWidth / 2;
    const QRectF ringRect = m_frame.adjusted(-out, -out, out, out);
    const bool still = ui::reducedMotion();
    // The ring, quiet under the light; whole and steady under Reduce Motion.
    QColor rest = hue;
    rest.setAlphaF(still ? 0.85f : float(tokens::motion::edgeRest));
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(rest, tokens::motion::edgeWidth));
    p.drawRoundedRect(ringRect, m_radius + out, m_radius + out);
    if (still || m_hue == Hue::None) return;
    // A head of light with a tail going out behind it.
    const qreal turn = qreal(clock() % tokens::motion::edgeTurn) / tokens::motion::edgeTurn;
    const QList<QPolygonF> runs = comet(turn);
    for (int run = 0; run < runs.size(); ++run) {
        const qreal fade = 1.0 - qreal(run) / runs.size();
        QColor light = run == 0 ? ui::mix(hue, Qt::white, 0.45) : hue;
        light.setAlphaF(float(0.95 * fade * fade));
        p.setPen(QPen(light, tokens::motion::edgeWidth, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
        p.drawPolyline(runs.at(run));
    }
}

} // namespace kerf::activity
