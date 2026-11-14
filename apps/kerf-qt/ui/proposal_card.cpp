#include "proposal_card.h"

#include "comment_row.h"
#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QElapsedTimer>
#include <QEvent>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QLabel>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPaintEvent>
#include <QSet>
#include <QSignalBlocker>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>
#include <utility>

namespace kerf {

using namespace tokens;

namespace {

QLabel *makeLabel(const TypeStyle &style, QRgb ink, QWidget *parent = nullptr) {
    auto *label = new QLabel(parent);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
    label->setFixedHeight(style.line);
    return label;
}

void setInk(QLabel *label, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
}

// The script diff (.diff): chrome, radius 4, an inset line, pad 4 8, mono 11/16.
class DiffView final : public QLabel {
public:
    DiffView() {
        setObjectName(QStringLiteral("proposalScript"));
        setFont(tokens::font(monoSmall));
        setTextFormat(Qt::RichText);
        setContentsMargins(8, 4, 8, 4);
        setWordWrap(false);
        setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        setTextInteractionFlags(Qt::TextSelectableByMouse);
        QPalette palette = this->palette();
        palette.setColor(QPalette::WindowText, color(ink2));
        setPalette(palette);
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        {
            QPainter p(this);
            p.setRenderHint(QPainter::Antialiasing);
            p.setPen(Qt::NoPen);
            p.setBrush(color(chrome));
            p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
            ui::paintInsetLine(p, QRectF(rect()), radius::small, color(line));
        }
        QLabel::paintEvent(event);
    }
};

// The "Waiting for the editor" chip: 24 tall, radius 6, on, 11 ink2.
class WaitChip final : public QLabel {
public:
    WaitChip() {
        setText(QStringLiteral("Waiting for the editor"));
        setFont(tokens::font(meta));
        setContentsMargins(8, 0, 8, 0);
        setFixedHeight(24);
        setFixedWidth(ui::snapWidth(QFontMetricsF(tokens::font(meta)), text()) + 16);
        QPalette palette = this->palette();
        palette.setColor(QPalette::WindowText, color(ink2));
        setPalette(palette);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing);
        p.setPen(Qt::NoPen);
        p.setBrush(color(on));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
        const QFont face = tokens::font(meta);
        const QFontMetricsF metrics(face);
        p.setFont(face);
        p.setPen(color(ink2));
        p.drawText(QPointF(8, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    }
};

// A change row (.chg): 24 tall, name UI 12 ink (elided), target mono 12
// ink2, shift mono 11 ink3 right-aligned in 40, gap 12.
class ChangeRow final : public QWidget {
public:
    explicit ChangeRow(const ProposalCard::Change &change) : m_change(change) {
        setObjectName(QStringLiteral("proposalChange"));
        setFixedHeight(24);
        setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        ui::setTip(this, QStringLiteral("%1 %2 %3").arg(change.name, change.target, change.shift).trimmed());
        setAccessibleName(QStringLiteral("%1 %2 %3").arg(change.name, change.target, change.shift).trimmed());
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        const QFont name = tokens::font(control);
        const QFont target = tokens::font(timecode);
        const QFont shift = tokens::font(monoSmall);
        const int shiftWidth = 40;
        const int targetWidth = ui::snapWidth(QFontMetricsF(target), m_change.target);
        const int nameWidth = std::max(0, width() - shiftWidth - targetWidth - 24);
        const auto draw = [&p, this](const QFont &face, QRgb ink, const QRectF &box, const QString &text, Qt::Alignment align) {
            const QFontMetricsF metrics(face);
            p.setFont(face);
            p.setPen(color(ink));
            const QString shown = metrics.elidedText(text, Qt::ElideRight, box.width());
            const qreal baseline = (height() - metrics.height()) / 2 + metrics.ascent();
            const qreal w = metrics.horizontalAdvance(shown);
            const qreal x = align & Qt::AlignRight ? box.right() - w : box.left();
            p.drawText(QPointF(x, baseline), shown);
        };
        draw(name, ink, QRectF(0, 0, nameWidth, height()), m_change.name, Qt::AlignLeft);
        draw(target, ink2, QRectF(nameWidth + 12, 0, targetWidth, height()), m_change.target, Qt::AlignLeft);
        draw(shift, ink3, QRectF(width() - shiftWidth, 0, shiftWidth, height()), m_change.shift, Qt::AlignRight);
    }

private:
    ProposalCard::Change m_change;
};

// A line of text that ends in an ellipsis when it has less room than it needs.
class LineLabel final : public QLabel {
public:
    LineLabel() {
        setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        setMinimumWidth(0);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        const QFontMetricsF metrics(font());
        p.setFont(font());
        p.setPen(palette().color(QPalette::WindowText));
        const QString shown = metrics.elidedText(text(), Qt::ElideRight, width());
        p.drawText(QPointF(0, (height() - metrics.height()) / 2 + metrics.ascent()), shown);
    }
};

QWidget *spaced(QWidget *child, int top) {
    auto *box = new QWidget;
    auto *layout = new QVBoxLayout(box);
    layout->setContentsMargins(0, top, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(child);
    return box;
}

} // namespace

// ------------------------------------------------------------------ VersionButton

VersionButton::VersionButton(QWidget *parent) : QAbstractButton(parent) {
    setFont(tokens::font(monoSmall));
    setFixedHeight(20);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    ui::setTip(this, QStringLiteral("Proposal versions"));
}

QSize VersionButton::sizeHint() const {
    return QSize(6 + ui::snapWidth(QFontMetricsF(font()), text()) + 2 + 14 + 2, 20);
}

void VersionButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const bool hover = underMouse() || isDown();
    if (hover) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(hover2));
        p.drawRoundedRect(QRectF(rect()), radius::small, radius::small);
    }
    const QColor ink = hover ? color(tokens::ink) : color(ink3);
    const QFontMetricsF metrics(font());
    p.setFont(font());
    p.setPen(ink);
    p.drawText(QPointF(6, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    icons::paint(p, QStringLiteral("g-chev"), QRectF(width() - 2 - 14, (height() - 14) / 2.0, 14, 14), ink, 14);
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::small);
}

// ------------------------------------------------------------------ CardPicture

namespace {
// Under the picture, before the title.
constexpr int kPictureGap = 8;
constexpr int kPlayMark = 40;
// Where things show on the picture as it finishes.
constexpr int kMarkAfter = 900;
constexpr int kDoneFade = 1000;
constexpr double kPi = 3.14159265358979323846;

qreal ramp(qint64 at, qint64 from, qint64 length) {
    if (length <= 0) return at >= from ? 1.0 : 0.0;
    return std::clamp(qreal(at - from) / qreal(length), 0.0, 1.0);
}
} // namespace

constexpr int kRimPoints = 360;
constexpr qreal kTail = 0.3;
constexpr int kTailRuns = 18;

const QList<QPointF> &CardPicture::rim() const {
    const QRect box = pictureRect();
    if (box == m_rimFor && !m_rim.isEmpty()) return m_rim;
    m_rimFor = box;
    m_rim.clear();
    const qreal ring = tokens::layout::cardRing;
    QPainterPath path;
    path.addRoundedRect(QRectF(box).adjusted(-ring / 2, -ring / 2, ring / 2, ring / 2), radius::card + ring / 2, radius::card + ring / 2);
    for (int index = 0; index < kRimPoints; ++index) m_rim << path.pointAtPercent(qreal(index) / kRimPoints);
    return m_rim;
}

QList<QPolygonF> CardPicture::comet(qreal turn) const {
    const QList<QPointF> &points = rim();
    QList<QPolygonF> runs;
    if (points.isEmpty()) return runs;
    const int head = int(turn * kRimPoints) % kRimPoints;
    const int tail = int(kTail * kRimPoints);
    const int each = std::max(1, tail / kTailRuns);
    for (int run = 0; run < kTailRuns; ++run) {
        QPolygonF line;
        for (int step = 0; step <= each; ++step) line << points.at(((head - run * each - step) % kRimPoints + kRimPoints) % kRimPoints);
        runs << line;
    }
    return runs;
}

qint64 CardPicture::clock() {
    static QElapsedTimer timer;
    if (!timer.isValid()) timer.start();
    return timer.elapsed();
}

CardPicture::CardPicture(QWidget *parent) : QAbstractButton(parent) {
    setObjectName(QStringLiteral("cardPicture"));
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setMouseTracking(true);
    QSizePolicy policy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    policy.setHeightForWidth(true);
    setSizePolicy(policy);
    ui::setTip(this, QStringLiteral("Open on the stage"));
    setAccessibleName(QStringLiteral("Open on the stage"));
    // It fills all of itself, with the card's own ground round the picture, so
    // nothing under it is drawn again with it.
    setAttribute(Qt::WA_OpaquePaintEvent);
    m_ticker = new QTimer(this);
    m_ticker->setInterval(tokens::motion::cardFrame);
    connect(m_ticker, &QTimer::timeout, this, &CardPicture::tick);
}

void CardPicture::setPicture(const CutPicture &picture) {
    // At work, the picture becomes the newest clip, from what it was.
    if (m_working && !ui::reducedMotion()) {
        const QImage was = poster();
        m_picture = picture;
        if (poster().cacheKey() != was.cacheKey()) {
            m_before = m_still;
            m_turnedAt = m_before.isNull() ? -1 : clock();
        }
    } else {
        m_picture = picture;
    }
    // Keep what is ready of the posters it still has.
    QSet<qint64> still;
    for (const CutPicture::Frame &frame : m_picture.frames) still.insert(frame.poster.cacheKey());
    if (!m_before.isNull()) still.insert(m_before.cacheKey());
    for (auto it = m_ready.begin(); it != m_ready.end();)
        it = still.contains(it.key()) ? std::next(it) : m_ready.erase(it);
    updateGeometry();
    update();
    followTicks();
}

void CardPicture::setWorking(bool working) {
    if (working == m_working) return;
    m_working = working;
    m_before = QImage();
    m_turnedAt = -1;
    m_scrub = -1.0;
    update();
    followTicks();
}

void CardPicture::setFinished(qint64 ago) {
    m_finishedAt = clock() - std::max<qint64>(0, ago);
    update();
    followTicks();
}

bool CardPicture::isFinishing() const { return m_finishedAt >= 0 && clock() - m_finishedAt < tokens::motion::cardDone; }

void CardPicture::setOnStage(bool on) {
    if (on == m_onStage) return;
    m_onStage = on;
    setCursor(on ? Qt::ArrowCursor : Qt::PointingHandCursor);
    update();
}

void CardPicture::showEvent(QShowEvent *event) {
    QAbstractButton::showEvent(event);
    followTicks();
}

void CardPicture::hideEvent(QHideEvent *event) {
    QAbstractButton::hideEvent(event);
    m_ticker->stop();
}

// Frames only while something moves, and only while it can be seen.
void CardPicture::followTicks() {
    const bool moving = !ui::reducedMotion() && isVisible() && (m_working || isFinishing());
    if (moving && !m_ticker->isActive()) m_ticker->start();
    if (!moving) m_ticker->stop();
}

void CardPicture::tick() {
    const QRect box = pictureRect();
    const qint64 now = clock();
    const bool turning = m_turnedAt >= 0 && now - m_turnedAt < tokens::motion::cardCrossfade;
    if (!turning && m_turnedAt >= 0) {
        m_before = QImage();
        m_turnedAt = -1;
        update(box);
    }
    const qint64 since = m_finishedAt >= 0 ? now - m_finishedAt : -1;
    if (since >= 0 && since < kMarkAfter + 300) {
        // The whole picture changes: it lightens, and its marks come.
        update(rect());
    } else if (since >= 0) {
        // Only Done, going.
        update(chipRect(box).toAlignedRect().adjusted(-1, -4, 1, 1));
    } else {
        // A clip turning changes the picture; otherwise only where the light was
        // and is, the chip and the line, each marked on its own, so what is put
        // back is these and not the picture between them.
        if (turning) update(box);
        const qreal turn = qreal(now % tokens::motion::cardTurn) / tokens::motion::cardTurn;
        QRegion lit;
        for (const QPolygonF &run : comet(turn)) lit += run.boundingRect().toAlignedRect().adjusted(-3, -3, 3, 3);
        update(m_lit + lit);
        m_lit = lit;
        const int ring = tokens::layout::cardRing;
        update(chipRect(box).toAlignedRect().adjusted(-1, -4, 1, 1));
        update(QRect(box.left(), box.bottom() + ring + 1, box.width(), height() - box.bottom() - ring - 1));
    }
    if (m_finishedAt >= 0 && now - m_finishedAt >= tokens::motion::cardDone) {
        m_finishedAt = -1;
        update(rect());
    }
    followTicks();
}

QRect CardPicture::boxFor(int width) const {
    const int ring = tokens::layout::cardRing;
    const double aspect = m_picture.aspect > 0.0 ? m_picture.aspect : 16.0 / 9.0;
    int w = std::min(width - 2 * ring, tokens::layout::cardPictureWidest);
    int h = int(std::lround(w / aspect));
    if (h > tokens::layout::cardPictureTallest) {
        h = tokens::layout::cardPictureTallest;
        w = int(std::lround(h * aspect));
    }
    return QRect(ring, ring, std::max(0, w), std::max(0, h));
}

QRect CardPicture::pictureRect() const { return boxFor(width()); }

int CardPicture::heightForWidth(int width) const { return boxFor(width).height() + 2 * tokens::layout::cardRing + kPictureGap; }

QSize CardPicture::sizeHint() const {
    const int widest = tokens::layout::cardPictureWidest + 2 * tokens::layout::cardRing;
    return QSize(widest, heightForWidth(widest));
}

QImage CardPicture::poster() const {
    if (m_picture.frames.isEmpty()) return {};
    // At work, the newest clip; else the one under the pointer, or the first.
    if (m_working) {
        for (auto it = m_picture.frames.crbegin(); it != m_picture.frames.crend(); ++it)
            if (!it->poster.isNull()) return it->poster;
        return {};
    }
    const double at = m_scrub >= 0.0 ? m_scrub * m_picture.duration : 0.0;
    QImage shown = m_picture.frames.constFirst().poster;
    for (const CutPicture::Frame &frame : m_picture.frames)
        if (frame.at <= at && !frame.poster.isNull()) shown = frame.poster;
    return shown;
}

QImage CardPicture::readyAt(const QImage &poster, const QSize &size) const {
    if (poster.isNull() || size.isEmpty()) return {};
    const auto found = m_ready.constFind(poster.cacheKey());
    if (found != m_ready.constEnd() && found->size() == size) return *found;
    const QImage filled = poster.scaled(size, Qt::KeepAspectRatioByExpanding, Qt::SmoothTransformation);
    QImage ready = filled.copy((filled.width() - size.width()) / 2, (filled.height() - size.height()) / 2, size.width(), size.height())
                       .convertToFormat(QImage::Format_RGB32);
    m_ready.insert(poster.cacheKey(), ready);
    return ready;
}

void CardPicture::mouseMoveEvent(QMouseEvent *event) {
    const QRect box = pictureRect();
    const qreal scrub = m_onStage || m_working || box.width() <= 0 || !box.contains(event->position().toPoint())
                            ? -1.0
                            : std::clamp((event->position().x() - box.left()) / box.width(), 0.0, 0.999);
    if (scrub != m_scrub) {
        m_scrub = scrub;
        update();
    }
    QAbstractButton::mouseMoveEvent(event);
}

void CardPicture::leaveEvent(QEvent *event) {
    m_scrub = -1.0;
    update();
    QAbstractButton::leaveEvent(event);
}

// The picture in its rounded box: the poster (turning from the last while a clip
// turns), how far it sits back, the pointer's place, and On the stage.
void CardPicture::paintPicture(QPainter &p, const QRectF &box, qreal work) const {
    const QSize pixels = (box.size() * devicePixelRatioF()).toSize();
    QPainterPath shape;
    shape.addRoundedRect(box, radius::card, radius::card);
    p.save();
    p.setClipPath(shape);
    p.fillRect(box, color(overlay::deviceScreen));
    const QImage shown = poster();
    if (shown.isNull()) {
        // Nothing placed yet: a wash in the room's greys and the accent, never black.
        p.fillRect(box, QColor(0x14, 0x17, 0x20));
        const auto wash = [&](qreal x, qreal y, qreal r, QColor tint) {
            QRadialGradient glow(QPointF(box.left() + box.width() * x, box.top() + box.height() * y), box.width() * r);
            glow.setColorAt(0.0, tint);
            tint.setAlpha(0);
            glow.setColorAt(1.0, tint);
            p.fillRect(box, glow);
        };
        wash(0.32, 0.42, 0.42, QColor(143, 176, 255, 70));
        wash(0.68, 0.58, 0.36, QColor(180, 170, 255, 46));
    } else {
        p.drawImage(box, readyAt(shown, pixels));
    }
    if (m_scrub >= 0.0) p.fillRect(QRectF(box.left(), box.bottom() - 2, box.width() * m_scrub, 2), QColor(255, 255, 255, 230));
    // At work the picture sits a little back.
    if (work > 0.0) p.fillRect(box, QColor(0, 0, 0, int(alpha(0.22) * work)));
    if (m_onStage) {
        // It is on the stage now: what is left here says so.
        p.fillRect(box, QColor(18, 18, 18, alpha(0.78)));
        p.setFont(tokens::font(control));
        p.setPen(color(ink2));
        p.drawText(box, Qt::AlignCenter, QStringLiteral("On the stage"));
    }
    p.restore();
}

// How long it is, at its foot, and that it plays.
void CardPicture::paintMarks(QPainter &p, const QRectF &box, qreal opacity) const {
    if (opacity <= 0.0) return;
    p.save();
    p.setOpacity(opacity);
    const int seconds = int(std::lround(m_picture.duration));
    const QString length = QStringLiteral("%1:%2").arg(seconds / 60).arg(seconds % 60, 2, 10, QLatin1Char('0'));
    const QFont mono = tokens::font(monoSmall);
    const QFontMetricsF metrics(mono);
    const QRectF badge(box.right() - 6 - metrics.horizontalAdvance(length) - 10, box.bottom() - 6 - 18, metrics.horizontalAdvance(length) + 10, 18);
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(0, 0, 0, alpha(0.66)));
    p.drawRoundedRect(badge, radius::small, radius::small);
    p.setFont(mono);
    p.setPen(Qt::white);
    p.drawText(badge, Qt::AlignCenter, length);
    const QRectF mark(box.center() - QPointF(kPlayMark, kPlayMark) / 2.0, QSizeF(kPlayMark, kPlayMark));
    p.setOpacity(opacity * (underMouse() || isDown() ? 1.0 : 0.85));
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(0, 0, 0, alpha(0.5)));
    p.drawEllipse(mark);
    icons::paint(p, QStringLiteral("g-play"), QRectF(mark.center() - QPointF(7, 8), QSizeF(16, 16)), Qt::white, 16);
    p.restore();
}

QRectF CardPicture::chipRect(const QRectF &box) const {
    const QFontMetricsF metrics(tokens::font(meta));
    const QString text = m_working ? QStringLiteral("Cutting") : QStringLiteral("Done");
    return QRectF(box.left() + 8, box.top() + 8, 8 + 18 + metrics.horizontalAdvance(text) + 9, 22);
}

void CardPicture::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setRenderHint(QPainter::SmoothPixmapTransform);
    const QRectF box(pictureRect());
    if (box.isEmpty()) return;
    const bool still = ui::reducedMotion();
    const qint64 now = clock();
    const qint64 since = m_finishedAt >= 0 ? now - m_finishedAt : std::numeric_limits<qint64>::max() / 2;
    const bool finishing = !still && since < tokens::motion::cardDone;
    const bool turning = !still && m_turnedAt >= 0 && now - m_turnedAt < tokens::motion::cardCrossfade;
    // How much it is still at work: 1 while Kerf works, going to 0 as it finishes.
    const qreal work = m_working ? 1.0 : finishing ? 1.0 - ramp(since, 0, tokens::motion::cardRing) : 0.0;
    const QRect dirty = event->rect();

    const qreal ring = tokens::layout::cardRing;
    const QRectF band = box.adjusted(-ring / 2, -ring / 2, ring / 2, ring / 2);
    // While Kerf works, a soft glow round the picture holds still; only the light moves.
    const auto glow = [&](QPainter &q) {
        q.setPen(QPen(QColor(143, 176, 255, 46), 6));
        q.setBrush(Qt::NoBrush);
        q.drawRoundedRect(band, radius::card + ring / 2, radius::card + ring / 2);
    };
    const qreal ratio = devicePixelRatioF();
    const auto put = [&](const QImage &image) {
        // Rect by rect, as it is opaque: a copy, with nothing to blend.
        for (const QRect &part : event->region())
            p.drawImage(part, image, QRectF(QPointF(part.topLeft()) * ratio, QSizeF(part.size()) * ratio).toRect());
    };
    if (finishing && since < kMarkAfter + 300) {
        // Lightening, and its marks coming: drawn as it is, for a moment.
        p.fillRect(dirty, color(raised));
        paintPicture(p, box, work);
        if (!m_onStage) paintMarks(p, box, ramp(since, kMarkAfter, 300));
    } else {
        // Still: drawn once, and from then on put back where it is asked for.
        const QString key = QStringLiteral("%1/%2x%3@%4/%5%6%7/%8/%9")
                                .arg(poster().cacheKey())
                                .arg(width())
                                .arg(height())
                                .arg(ratio)
                                .arg(int(m_working))
                                .arg(int(m_onStage))
                                .arg(int(underMouse() || isDown()))
                                .arg(m_scrub)
                                .arg(m_picture.duration);
        if (key != m_stillKey || m_still.isNull()) {
            m_still = QImage((size() * ratio), QImage::Format_RGB32);
            m_still.setDevicePixelRatio(ratio);
            m_still.fill(color(raised));
            QPainter q(&m_still);
            q.setRenderHint(QPainter::Antialiasing);
            q.setRenderHint(QPainter::SmoothPixmapTransform);
            if (m_working && !ui::reducedMotion()) glow(q);
            paintPicture(q, box, work);
            if (!m_onStage && !m_working) paintMarks(q, box, 1.0);
            m_stillKey = key;
        }
        // A clip turning: the still that showed the last, and this one over it.
        if (turning && m_before.size() == m_still.size()) {
            put(m_before);
            p.setOpacity(ramp(now, m_turnedAt, tokens::motion::cardCrossfade));
        }
        put(m_still);
        p.setOpacity(1.0);
    }

    // What Kerf is doing, at its top left: hopping dots and Cutting, then a tick and Done.
    const qreal chip = m_working ? 1.0 : finishing ? 1.0 - ramp(since, tokens::motion::cardDone - kDoneFade, kDoneFade) : 0.0;
    const QRectF pill = chipRect(box);
    if (chip > 0.0 && !m_onStage && dirty.intersects(pill.toAlignedRect())) {
        const QFont face = tokens::font(meta);
        p.save();
        p.setOpacity(chip);
        p.setPen(Qt::NoPen);
        p.setBrush(QColor(10, 12, 18, alpha(0.62)));
        p.drawRoundedRect(pill, pill.height() / 2, pill.height() / 2);
        p.setBrush(color(accentInk));
        if (m_working) {
            for (int dot = 0; dot < 3; ++dot) {
                const qreal t = still ? 0.0 : std::fmod(qreal(now) / 1100.0 - dot * 0.14 + 1.0, 1.0);
                const qreal hop = t < 0.6 ? std::sin(t / 0.6 * kPi) : 0.0;
                p.setOpacity(chip * (0.5 + 0.5 * hop));
                p.drawEllipse(QPointF(pill.left() + 10 + dot * 6, pill.center().y() - 3 * hop), 2, 2);
            }
        } else {
            p.setPen(QPen(color(accentInk), 1.8, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            p.setBrush(Qt::NoBrush);
            const QPointF at(pill.left() + 9, pill.center().y());
            p.drawPolyline(QPolygonF{at + QPointF(0, 0.5), at + QPointF(3.5, 4), at + QPointF(10, -3.5)});
        }
        p.setOpacity(chip);
        p.setFont(face);
        p.setPen(Qt::white);
        p.drawText(QRectF(pill.left() + 26, pill.top(), pill.width() - 26, pill.height()), Qt::AlignVCenter | Qt::AlignLeft,
                   m_working ? QStringLiteral("Cutting") : QStringLiteral("Done"));
        p.restore();
    }

    // The light round it: running while Kerf works, lit once and going out when done.
    if (m_working) {
        p.setBrush(Qt::NoBrush);
        if (still) {
            p.setPen(QPen(QColor(143, 176, 255, 150), 2));
        } else {
            // A head of white light with a tail going into the accent, drawn as short
            // runs of the rim, brightest first.
            const qreal turn = qreal(now % tokens::motion::cardTurn) / tokens::motion::cardTurn;
            const QList<QPolygonF> runs = comet(turn);
            for (int run = 0; run < runs.size(); ++run) {
                const qreal fade = 1.0 - qreal(run) / runs.size();
                const QColor white(255, 255, 255), blue(143, 176, 255);
                const qreal mix = std::min(1.0, run / 3.0);
                QColor shade = QColor::fromRgbF(white.redF() + (blue.redF() - white.redF()) * mix,
                                                white.greenF() + (blue.greenF() - white.greenF()) * mix,
                                                white.blueF() + (blue.blueF() - white.blueF()) * mix, 0.95 * fade * fade);
                p.setPen(QPen(shade, 2, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
                p.drawPolyline(runs.at(run));
            }
        }
        if (still) p.drawRoundedRect(band, radius::card + ring / 2, radius::card + ring / 2);
    } else if (finishing && since < tokens::motion::cardRing) {
        p.setPen(QPen(QColor(255, 255, 255, int(240 * (1.0 - ramp(since, 0, tokens::motion::cardRing)))), 2));
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(band, radius::card + ring / 2, radius::card + ring / 2);
    }

    // Under it, a line of light runs while Kerf works.
    if (work > 0.0 && !still) {
        const QRectF line(box.left(), box.bottom() + ring + 3, box.width(), 2);
        p.save();
        p.setOpacity(work);
        p.setPen(Qt::NoPen);
        p.setBrush(color(on));
        p.drawRoundedRect(line, 1, 1);
        const qreal run = qreal(now % 1400) / 1400.0;
        const qreal head = line.left() - line.width() * 0.4 + run * line.width() * 1.4;
        QLinearGradient light(QPointF(head, 0), QPointF(head + line.width() * 0.4, 0));
        light.setColorAt(0.0, QColor(143, 176, 255, 0));
        light.setColorAt(0.5, QColor(220, 230, 255, 230));
        light.setColorAt(1.0, QColor(143, 176, 255, 0));
        p.setClipRect(line);
        p.fillRect(line, light);
        p.restore();
    }
    if (hasFocus()) ui::paintFocusRing(p, box, radius::card);
}

// ------------------------------------------------------------------ ProposalCard

ProposalCard::ProposalCard(const QString &proposalId, QWidget *parent) : QFrame(parent), m_id(proposalId) {
    setObjectName(QStringLiteral("proposalCard"));
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    m_layout = new QVBoxLayout(this);
    m_layout->setContentsMargins(12, 12, 12, 12);
    m_layout->setSpacing(0);

    // The pinned header: 16 tall, gap 6, 8 above the title.
    m_pinHeader = new QWidget;
    m_pinHeader->setFixedHeight(16 + 8);
    auto *pinLayout = new QHBoxLayout(m_pinHeader);
    pinLayout->setContentsMargins(0, 0, 0, 8);
    pinLayout->setSpacing(6);
    auto *pin = new QLabel;
    pin->setFixedSize(12, 16);
    pin->setPixmap(icons::pixmap(QStringLiteral("g-pin"), color(ink3), 12, devicePixelRatioF()));
    pin->setAlignment(Qt::AlignCenter);
    pinLayout->addWidget(pin);
    m_pinText = makeLabel(meta, ink3);
    m_pinText->setText(QStringLiteral("Pinned proposal"));
    pinLayout->addWidget(m_pinText, 1);
    m_layout->addWidget(m_pinHeader);
    m_pinHeader->hide();

    // A cut's picture, over its title.
    m_picture = new CardPicture;
    m_picture->hide();
    connect(m_picture, &QAbstractButton::clicked, this, [this] {
        if (!m_picture->onStage()) emit pictureRequested();
    });
    m_layout->addWidget(m_picture);

    // The title: click opens or closes the card.
    m_title = new QLabel;
    m_title->setObjectName(QStringLiteral("proposalTitle"));
    m_title->setFont(tokens::font(bodyName));
    m_title->setWordWrap(true);
    m_title->setTextFormat(Qt::PlainText);
    m_title->setCursor(Qt::PointingHandCursor);
    m_title->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    setInk(m_title, ink);
    m_title->installEventFilter(this);
    m_layout->addWidget(m_title);

    // The second line: N changes, then the version button at the right.
    m_second = new QWidget;
    m_second->setFixedHeight(20);
    auto *second = new QHBoxLayout(m_second);
    second->setContentsMargins(0, 0, 0, 0);
    second->setSpacing(8);
    m_changesLink = new ChatExpander;
    m_changesLink->setText(QStringLiteral("0 changes"));
    // Toggled rather than clicked: an accessibility press toggles it without a click.
    connect(m_changesLink, &QAbstractButton::toggled, this, [this](bool open) {
        if (open == m_open) return;
        setOpen(open);
        emit openToggled(open);
    });
    second->addWidget(m_changesLink);
    second->addStretch(1);
    m_version = new VersionButton;
    m_version->hide();
    connect(m_version, &QAbstractButton::clicked, this, &ProposalCard::showVersionMenu);
    second->addWidget(m_version);
    m_layout->addWidget(m_second);

    // The changes, 4 under it.
    m_changes = new QWidget;
    m_changesLayout = new QVBoxLayout(m_changes);
    m_changesLayout->setContentsMargins(0, 4, 0, 0);
    m_changesLayout->setSpacing(0);
    m_colourBox = new QWidget;
    m_colourBox->setObjectName(QStringLiteral("colourPreview"));
    auto *colour = new QHBoxLayout(m_colourBox);
    colour->setContentsMargins(0, 4, 0, 4);
    colour->setSpacing(12);
    auto *colourText = new QVBoxLayout;
    colourText->setContentsMargins(0, 0, 0, 0);
    colourText->setSpacing(4);
    m_colourTitle = makeLabel(control, ink);
    m_colourParams = makeLabel(monoSmall, ink2);
    for (QLabel *label : {m_colourTitle, m_colourParams}) {
        label->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Fixed);
        colourText->addWidget(label);
    }
    colourText->addStretch(1);
    colour->addLayout(colourText, 1);
    m_colourBox->hide();
    m_changesLayout->addWidget(m_colourBox);
    m_layout->addWidget(m_changes);

    // The script diff, 8 under the changes.
    m_diff = new DiffView;
    QWidget *diffBox = spaced(m_diff, 8);
    diffBox->setObjectName(QStringLiteral("diffBox"));
    diffBox->hide();
    m_layout->addWidget(diffBox);

    // The footer: 24 tall, 8 under, gap 8.
    m_footer = new QWidget;
    m_footer->setObjectName(QStringLiteral("proposalFooter"));
    m_footer->setFixedHeight(24 + 8);
    auto *footer = new QHBoxLayout(m_footer);
    footer->setContentsMargins(0, 8, 0, 0);
    footer->setSpacing(8);
    m_lineTitle = new LineLabel;
    m_lineTitle->setObjectName(QStringLiteral("proposalLineTitle"));
    m_lineTitle->setFont(tokens::font(bodyName));
    m_lineTitle->setFixedHeight(24);
    setInk(m_lineTitle, ink);
    m_lineTitle->hide();
    // It has the line's room; the stretch after Script has the rest.
    footer->addWidget(m_lineTitle, 100);
    m_watch = new ui::Button(QString(), ui::Button::Kind::Secondary);
    m_watch->setObjectName(QStringLiteral("proposalWatch"));
    m_watch->setTip(QStringLiteral("Play only what this proposal changes"));
    // Across the card, 8 under the changes.
    m_watch->setMinimumWidth(0);
    m_watch->setMaximumWidth(QWIDGETSIZE_MAX);
    m_watch->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    connect(m_watch, &QAbstractButton::clicked, this, &ProposalCard::watchRequested);
    m_watchBox = spaced(m_watch, 8);
    m_watchBox->setObjectName(QStringLiteral("proposalWatchBox"));
    m_watchBox->hide();
    m_layout->addWidget(m_watchBox);
    m_preview = new ui::CheckBox(QStringLiteral("Preview"));
    m_preview->setTip(QStringLiteral("Preview the proposal on the program"));
    connect(m_preview, &QAbstractButton::toggled, this, &ProposalCard::previewToggled);
    footer->addWidget(m_preview);
    m_script = new ui::GhostButton(QStringLiteral("Script"));
    m_script->setHorizontalPadding(6);
    m_script->setCheckable(true);
    m_script->setTip(QStringLiteral("Show the script"));
    connect(m_script, &QAbstractButton::toggled, this, [this](bool shown) {
        setScriptShown(shown);
        emit scriptToggled(shown);
    });
    footer->addWidget(m_script);
    footer->addStretch(1);
    m_wait = new WaitChip;
    m_wait->hide();
    footer->addWidget(m_wait);
    m_reject = new ui::Button(QStringLiteral("Reject"), ui::Button::Kind::Secondary);
    m_reject->setTip(QStringLiteral("Reject"), QStringLiteral("Esc"));
    connect(m_reject, &QAbstractButton::clicked, this, [this] { emit rejectRequested(shownId()); });
    footer->addWidget(m_reject);
    m_accept = new ui::Button(QStringLiteral("Accept"), ui::Button::Kind::Secondary);
    m_acceptKey = ui::keyText(QKeySequence(Qt::CTRL | Qt::Key_Return));
    m_accept->setTip(m_acceptTip, m_acceptKey);
    connect(m_accept, &QAbstractButton::clicked, this, [this] { emit acceptRequested(shownId()); });
    footer->addWidget(m_accept);
    m_layout->addWidget(m_footer);

    // The settled line.
    m_settledLabel = makeLabel(control, accentInk);
    m_settledLabel->setFixedHeight(20);
    m_settledLabel->hide();
    m_layout->addWidget(m_settledLabel);

    updateParts();
}

void ProposalCard::setPinned(bool pinned, bool blocking) {
    m_pinned = pinned;
    m_pinText->setText(blocking ? QStringLiteral("Pinned proposal · blocking first") : QStringLiteral("Pinned proposal"));
    updateParts();
}

void ProposalCard::setTitle(const QString &title) {
    m_title->setText(title);
    m_lineTitle->setText(title);
    ui::setTip(m_lineTitle, title);
    setAccessibleName(title);
}

void ProposalCard::setChanges(const QList<Change> &changes) {
    for (QWidget *row : std::as_const(m_changeRows)) {
        m_changesLayout->removeWidget(row);
        delete row;
    }
    m_changeRows.clear();
    int index = 0;
    for (const Change &change : changes) {
        auto *row = new ChangeRow(change);
        m_changesLayout->insertWidget(index++, row);
        m_changeRows.append(row);
    }
    const int count = int(changes.size());
    m_changesLink->setText(count == 1 ? QStringLiteral("1 change") : QStringLiteral("%1 changes").arg(count));
    m_changesLink->updateGeometry();
}

void ProposalCard::setVersions(const QList<VersionItem> &versions, const QString &shownId) {
    m_versions = versions;
    m_shown = shownId;
    int shownVersion = 0;
    for (const VersionItem &item : versions)
        if (item.proposalId == shownId) shownVersion = item.version;
    if (shownVersion == 0 && !versions.isEmpty()) shownVersion = versions.constLast().version;
    m_version->setText(QStringLiteral("v%1").arg(shownVersion));
    m_version->setFixedSize(m_version->sizeHint());
    m_version->setVisible(versions.size() > 1);
    updateParts();
}

void ProposalCard::showVersionMenu() {
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    menu->setObjectName(QStringLiteral("versionsMenu"));
    for (auto it = m_versions.crbegin(); it != m_versions.crend(); ++it) {
        const VersionItem item = *it;
        QAction *action = menu->addAction(item.label.isEmpty() ? QStringLiteral("v%1").arg(item.version) : item.label);
        action->setCheckable(true);
        action->setChecked(item.proposalId == shownId());
        action->setData(item.proposalId);
        connect(action, &QAction::triggered, this, [this, id = item.proposalId] {
            if (id == shownId()) return;
            setVersions(m_versions, id);
            emit versionChosen(id);
        });
    }
    // Oldest first, as the design lists them.
    QList<QAction *> actions = menu->actions();
    std::reverse(actions.begin(), actions.end());
    for (QAction *action : actions) menu->removeAction(action);
    menu->addActions(actions);
    ui::popupMenu(menu, m_version);
}

void ProposalCard::setColour(const Colour &colour) {
    m_colour = colour;
    m_colourTitle->setText(colour.title);
    m_colourParams->setText(colour.params);
    m_colourParams->setVisible(!colour.params.isEmpty());
    m_colourBox->setVisible(!colour.clip.isEmpty());
}

void ProposalCard::clearColour() { setColour(Colour{}); }


void ProposalCard::setDiff(const QStringList &lines) {
    m_diffLines = lines;
    QString html;
    for (const QString &line : lines) {
        const bool header = line.startsWith(QLatin1String("+++")) || line.startsWith(QLatin1String("---"));
        const QRgb ink = !header && line.startsWith(QLatin1Char('+'))   ? tokens::pos
                         : !header && line.startsWith(QLatin1Char('-')) ? tokens::neg
                                                                         : tokens::ink2;
        if (!html.isEmpty()) html += QStringLiteral("<br>");
        html += QStringLiteral("<span style=\"color:%1; white-space:pre;\">%2</span>")
                    .arg(color(ink).name(QColor::HexRgb), line.toHtmlEscaped());
    }
    m_diff->setText(QStringLiteral("<div style=\"line-height:16px;\">%1</div>").arg(html));
    m_script->setEnabled(!lines.isEmpty());
}

void ProposalCard::setOpen(bool open) {
    m_open = open;
    m_changesLink->setChecked(open);
    updateParts();
}

void ProposalCard::setScriptShown(bool shown) {
    {
        const QSignalBlocker blocker(m_script);
        m_script->setChecked(shown);
    }
    m_diff->parentWidget()->setVisible(shown && m_open && !m_settled && m_picture->picture().isEmpty());
    updateGeometry();
}

bool ProposalCard::scriptShown() const { return m_script->isChecked(); }

void ProposalCard::setWatchLength(double seconds) {
    m_hasWatch = seconds > 0.0;
    m_watch->setText(QStringLiteral("Watch what changed · %1 s").arg(int(std::ceil(seconds))));
    updateParts();
}

void ProposalCard::setPreview(bool on) {
    const QSignalBlocker blocker(m_preview);
    m_preview->setChecked(on);
}

bool ProposalCard::previewOn() const { return m_preview->isChecked(); }

void ProposalCard::setPrimary(bool primary) { m_accept->setKind(primary && !m_compact ? ui::Button::Kind::Primary : ui::Button::Kind::Secondary); }

void ProposalCard::setAcceptTip(const QString &tip) {
    m_acceptTip = tip;
    if (!m_acceptBlocked) m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setAcceptKey(const QString &key) {
    m_acceptKey = key;
    if (!m_acceptBlocked) m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setGuest(bool guest) {
    m_guest = guest;
    updateParts();
}

void ProposalCard::setAcceptBlocked(const QString &reason) {
    m_accept->setEnabled(reason.isEmpty());
    m_acceptBlocked = !reason.isEmpty();
    if (m_acceptBlocked) m_accept->setTip(reason);
    else m_accept->setTip(m_acceptTip, m_acceptKey);
}

void ProposalCard::setActionsEnabled(bool enabled) {
    m_reject->setEnabled(enabled);
    m_preview->setEnabled(enabled);
    if (!enabled) m_accept->setEnabled(false);
}

void ProposalCard::setSettled(const QString &text, bool accepted) {
    m_settled = !text.isEmpty();
    m_settledLabel->setText(text);
    setInk(m_settledLabel, accepted ? accentInk : ink2);
    updateParts();
}

void ProposalCard::setCompact(bool compact) {
    m_compact = compact;
    m_watch->setKind(compact ? ui::Button::Kind::Primary : ui::Button::Kind::Secondary);
    m_accept->setKind(compact ? ui::Button::Kind::Secondary : ui::Button::Kind::Primary);
    updateParts();
}

void ProposalCard::setPicture(const CutPicture &picture) {
    m_picture->setPicture(picture);
    updateParts();
}

void ProposalCard::setBuilding(bool building) {
    m_building = building;
    m_picture->setWorking(building);
    updateParts();
}

void ProposalCard::updateParts() {
    const bool card = !m_settled;
    const bool marked = m_pinned && !m_compact;
    // With its picture a cut says what it is: the card keeps to its title and what
    // to do with it.
    const bool pictured = !m_picture->picture().isEmpty() || m_building;
    // A picture's ring lies in the card's padding, so the picture lines up with the title.
    const int inset = card && pictured ? 12 - tokens::layout::cardRing : card ? 12 : 0;
    m_layout->setContentsMargins(inset, card && !pictured ? (marked ? 8 : 12) : inset, inset, card ? 12 : 0);
    m_pinHeader->setVisible(card && marked);
    m_picture->setVisible(card && pictured);
    // And it is as wide as its picture.
    setMaximumWidth(card && pictured ? tokens::layout::cardPictureWidest + 2 * tokens::layout::cardRing + 24 : QWIDGETSIZE_MAX);
    m_title->setVisible(card && !pictured);
    m_lineTitle->setVisible(card && pictured);
    // Under a picture the line comes right after it.
    const int ring = pictured ? tokens::layout::cardRing : 0;
    m_footer->layout()->setContentsMargins(ring, pictured ? 0 : 8, ring, 0);
    m_footer->setFixedHeight(pictured ? 24 : 24 + 8);
    m_second->setVisible(card && (!pictured || (m_versions.size() > 1 && !m_building)));
    m_changesLink->setVisible(!pictured);
    m_changes->setVisible(card && m_open && !pictured);
    m_diff->parentWidget()->setVisible(card && m_open && m_script->isChecked() && !pictured);
    m_footer->setVisible(card && (m_open || m_compact || pictured));
    m_watchBox->setVisible(card && (m_open || m_compact) && m_hasWatch && !pictured);
    m_preview->setVisible(!pictured);
    m_script->setVisible(!pictured);
    m_settledLabel->setVisible(m_settled);
    // Nothing to do with a cut until Kerf has made it.
    m_reject->setVisible(!m_guest && !m_building);
    m_accept->setVisible(!m_guest && !m_building);
    m_wait->setVisible(m_guest && !m_building);
    updateGeometry();
    update();
}

void ProposalCard::paintEvent(QPaintEvent *) {
    if (m_settled) return;
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(area, radius::card, radius::card);
    if (m_pinned && !m_compact) {
        // inset 0 2px 0 accent: a 2 pt stripe inside the top edge, following its corners.
        p.save();
        QPainterPath clip;
        clip.addRoundedRect(area, radius::card, radius::card);
        p.setClipPath(clip);
        QPainterPath outer;
        outer.addRoundedRect(area, radius::card, radius::card);
        QPainterPath inner;
        inner.addRoundedRect(area.adjusted(0, 2, 0, 0), radius::card, radius::card);
        p.fillPath(outer.subtracted(inner), color(accent));
        p.restore();
    }
    ui::paintInsetLine(p, area, radius::card, color(line));
}

bool ProposalCard::eventFilter(QObject *watched, QEvent *event) {
    if (watched == m_title && event->type() == QEvent::MouseButtonRelease) {
        auto *mouse = static_cast<QMouseEvent *>(event);
        if (mouse->button() == Qt::LeftButton && m_title->rect().contains(mouse->position().toPoint())) {
            setOpen(!m_open);
            emit openToggled(m_open);
            return true;
        }
    }
    return QFrame::eventFilter(watched, event);
}

} // namespace kerf
