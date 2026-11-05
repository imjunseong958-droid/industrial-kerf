#include "compare_view.h"

#include "components.h"
#include "design_tokens.h"

#include <QGraphicsEffect>
#include <QHBoxLayout>
#include <QLabel>
#include <QLinearGradient>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPalette>
#include <QPropertyAnimation>
#include <QResizeEvent>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <utility>

namespace kerf {

using namespace tokens;

namespace {

void setInk(QLabel *label, const QColor &ink_) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, ink_);
    label->setPalette(palette);
}

constexpr int kMostChanges = 2;
constexpr int kMargin = 16;
constexpr int kGap = 14;
constexpr int kCardWidth = 250;
constexpr qreal kCardShape = 1.36;
constexpr int kUprightCardWidth = 216;
constexpr qreal kUprightCardShape = 1.78;
constexpr qreal kTilt = 1.5;
constexpr int kFan = 850;
constexpr int kStagger = 110;

// A card a little askew, and how much of it shows. It leans the card and all that
// is on it; a lean this small keeps its buttons under the pointer.
class CardLean final : public QGraphicsEffect {
    Q_OBJECT
    Q_PROPERTY(qreal angle READ angle WRITE setAngle)
    Q_PROPERTY(qreal level READ level WRITE setLevel)
public:
    using QGraphicsEffect::QGraphicsEffect;
    qreal angle() const { return m_angle; }
    void setAngle(qreal angle) {
        m_angle = angle;
        update();
    }
    qreal level() const { return m_level; }
    void setLevel(qreal level) {
        m_level = level;
        update();
    }
    QRectF boundingRectFor(const QRectF &rect) const override { return rect.adjusted(-12, -12, 12, 12); }

protected:
    void draw(QPainter *painter) override {
        QPoint offset;
        const QPixmap shown = sourcePixmap(Qt::LogicalCoordinates, &offset, PadToEffectiveBoundingRect);
        if (shown.isNull()) return;
        const qreal ratio = shown.devicePixelRatio();
        const QPointF centre = offset + QPointF(shown.width(), shown.height()) / (2.0 * ratio);
        painter->save();
        painter->setRenderHint(QPainter::SmoothPixmapTransform, true);
        painter->setOpacity(painter->opacity() * m_level);
        painter->translate(centre);
        painter->rotate(m_angle);
        painter->translate(-centre);
        painter->drawPixmap(offset, shown);
        painter->restore();
    }

private:
    qreal m_angle = 0.0;
    qreal m_level = 1.0;
};

CardLean *leanOf(QWidget *widget) { return qobject_cast<CardLean *>(widget->graphicsEffect()); }

// The card is its poster, edge to edge.
class CompareCard final : public QFrame {
public:
    using QFrame::QFrame;
    ComparePoster *poster = nullptr;

protected:
    void resizeEvent(QResizeEvent *event) override {
        QFrame::resizeEvent(event);
        if (poster) poster->setGeometry(rect());
    }
};

void glide(QWidget *widget, const QRect &to, int delay, int duration) {
    auto *move = new QPropertyAnimation(widget, "geometry", widget);
    move->setDuration(duration);
    move->setEndValue(to);
    move->setEasingCurve(tokens::easing());
    QTimer::singleShot(delay, move, [move, widget] {
        move->setStartValue(widget->geometry());
        move->start(QAbstractAnimation::DeleteWhenStopped);
    });
}

void ease(QObject *target, const char *property, const QVariant &to, int delay, int duration) {
    auto *change = new QPropertyAnimation(target, property, target);
    change->setDuration(duration);
    change->setEndValue(to);
    change->setEasingCurve(tokens::easing());
    QTimer::singleShot(delay, change, [change] { change->start(QAbstractAnimation::DeleteWhenStopped); });
}

} // namespace

ComparePoster::ComparePoster(QWidget *parent) : QWidget(parent) {}

void ComparePoster::setImage(const QImage &image) {
    m_image = image;
    update();
}

void ComparePoster::setPlain(bool plain) {
    m_plain = plain;
    update();
}

void ComparePoster::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    p.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath clip;
    clip.addRoundedRect(QRectF(rect()), radius::panel, radius::panel);
    p.setClipPath(clip);
    p.fillRect(rect(), color(raised));
    if (!m_image.isNull()) {
        QSize shown = m_image.size();
        shown.scale(size(), Qt::KeepAspectRatioByExpanding);
        p.drawImage(QRect(QPoint((width() - shown.width()) / 2, (height() - shown.height()) / 2), shown), m_image);
    }
    if (m_plain) return;
    // What is written on it stays readable.
    QLinearGradient foot(0, height() * 0.35, 0, height());
    foot.setColorAt(0.0, QColor(0, 0, 0, 0));
    foot.setColorAt(1.0, QColor(0, 0, 0, 200));
    p.fillRect(rect(), foot);
    p.setClipping(false);
    ui::paintInsetLine(p, QRectF(rect()), radius::panel, color(line));
}

CompareView::CompareView(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("compareView"));
    m_close = new ui::IconButton(QStringLiteral("g-close"), ui::IconButton::Size::S24, this);
    m_close->setObjectName(QStringLiteral("compareClose"));
    m_close->setAccessibleName(QStringLiteral("Close versions"));
    m_close->setTip(QStringLiteral("Close versions"));
    connect(m_close, &QAbstractButton::clicked, this, &CompareView::closeRequested);
}

CompareView::Card &CompareView::cardFor(const QString &key, bool *made) {
    auto found = m_byKey.find(key);
    if (found != m_byKey.end()) {
        *made = false;
        return *found;
    }
    *made = true;
    Card card;
    auto *frame = new CompareCard(this);
    card.frame = frame;
    card.frame->setObjectName(QStringLiteral("compareCard"));
    card.poster = new ComparePoster(card.frame);
    frame->poster = card.poster;
    card.poster->lower();
    auto *inner = new QVBoxLayout(card.frame);
    inner->setContentsMargins(space::s12, space::s12, space::s12, space::s12);
    inner->setSpacing(space::s4);
    inner->addStretch(1);
    auto *name = new QLabel(QStringLiteral("Version %1").arg(key));
    name->setFont(tokens::font(bodyName));
    setInk(name, Qt::white);
    inner->addWidget(name);
    card.status = new QLabel;
    card.status->setFont(tokens::font(meta));
    card.status->setWordWrap(true);
    setInk(card.status, QColor(255, 255, 255, 200));
    inner->addWidget(card.status);
    card.changes = new QLabel;
    card.changes->setFont(tokens::font(meta));
    card.changes->setWordWrap(true);
    setInk(card.changes, QColor(255, 255, 255, 170));
    inner->addWidget(card.changes);
    card.progress = new ui::ThinProgress;
    card.progress->setFixedHeight(space::s2);
    inner->addSpacing(space::s4);
    inner->addWidget(card.progress);
    inner->addSpacing(space::s4);
    auto *actions = new QHBoxLayout;
    actions->setContentsMargins(0, 0, 0, 0);
    actions->setSpacing(space::s4);
    card.watch = new ui::Button(QStringLiteral("Watch"), ui::Button::Kind::Secondary);
    card.watch->setObjectName(QStringLiteral("compareWatch"));
    card.use = new ui::Button(QStringLiteral("Use this"), ui::Button::Kind::Primary);
    card.use->setObjectName(QStringLiteral("compareUse"));
    actions->addWidget(card.watch);
    actions->addWidget(card.use, 1);
    inner->addLayout(actions);
    card.frame->setGraphicsEffect(new CardLean(card.frame));
    connect(card.watch, &QAbstractButton::clicked, this, [this, key] { emit watchRequested(m_byKey.value(key).proposal); });
    connect(card.use, &QAbstractButton::clicked, this, [this, key] { emit useRequested(m_byKey.value(key).proposal); });
    return *m_byKey.insert(key, card);
}

void CompareView::setUpright(bool upright) {
    if (upright == m_upright) return;
    m_upright = upright;
    placeCards(false);
}

void CompareView::setVersions(const QList<Version> &versions) {
    // The one chosen is on its way: what it is now does not change that.
    if (m_picking) return;
    QStringList keys;
    bool fresh = false;
    for (const Version &version : versions) {
        keys << version.key;
        bool made = false;
        Card &card = cardFor(version.key, &made);
        if (made) {
            fresh = true;
            // It starts as the picture.
            card.frame->setGeometry(m_origin.isValid() ? m_origin : QRect(width() / 2, height() / 2, 1, 1));
            card.frame->show();
        }
        card.proposal = version.proposalId;
        // Once it has a picture it keeps one.
        if (!version.poster.isNull()) card.poster->setImage(version.poster);
        QString status = version.status;
        if (status.isEmpty()) {
            switch (version.state) {
            case State::Running: status = QStringLiteral("Working"); break;
            case State::Failed: status = QStringLiteral("Failed"); break;
            case State::Stopped: status = QStringLiteral("Stopped"); break;
            case State::Ready: status = version.proposalId.isEmpty() ? QStringLiteral("Finished with no proposal") : QStringLiteral("Ready"); break;
            }
        }
        card.status->setText(status);
        QStringList lines = version.changes.mid(0, kMostChanges);
        if (version.changes.size() > kMostChanges) lines << QStringLiteral("and %1 more").arg(version.changes.size() - kMostChanges);
        card.changes->setText(lines.join(QLatin1Char('\n')));
        card.progress->setValue(version.state == State::Running ? (version.proposalId.isEmpty() ? 0.35 : 0.8) : 1.0);
        const bool usable = !version.proposalId.isEmpty();
        card.watch->setEnabled(usable);
        card.use->setEnabled(usable && version.state == State::Ready);
    }
    const QStringList gone = m_byKey.keys();
    for (const QString &key : gone) {
        if (keys.contains(key)) continue;
        Card card = m_byKey.take(key);
        card.frame->hide();
        card.frame->deleteLater();
    }
    m_keys = keys;
    placeCards(fresh);
}

QRect CompareView::cardRect(int index, int count) const {
    const int shown = std::max(1, count);
    const int room = std::max(1, width() - 2 * kMargin - (shown - 1) * kGap);
    const int tall = std::max(1, height() - 2 * kMargin);
    const qreal cardShape = m_upright ? kUprightCardShape : kCardShape;
    const int w = std::min({m_upright ? kUprightCardWidth : kCardWidth, room / shown, int(tall / cardShape)});
    const int h = std::min(tall, int(w * cardShape));
    const int left = (width() - (shown * w + (shown - 1) * kGap)) / 2;
    return QRect(left + index * (w + kGap), (height() - h) / 2, w, h);
}

void CompareView::placeCards(bool animate) {
    const int duration = animate ? ui::motionDuration(kFan) : 0;
    const int count = int(m_keys.size());
    for (int index = 0; index < count; ++index) {
        QFrame *frame = m_byKey.value(m_keys.at(index)).frame;
        const QRect to = cardRect(index, count);
        // The outer ones lean out from the middle.
        const qreal lean = (index - (count - 1) / 2.0) * kTilt;
        if (duration > 0) {
            glide(frame, to, index * kStagger, duration);
            if (CardLean *effect = leanOf(frame)) ease(effect, "angle", lean, index * kStagger, duration);
        } else {
            frame->setGeometry(to);
            if (CardLean *effect = leanOf(frame)) effect->setAngle(lean);
        }
    }
}

void CompareView::playPick(const QString &proposalId, const QRect &to, std::function<void()> done) {
    const int duration = ui::motionDuration(kFan);
    if (duration <= 0) {
        done();
        return;
    }
    m_picking = true;
    for (const QString &key : std::as_const(m_keys)) {
        const Card &card = m_byKey[key];
        CardLean *effect = leanOf(card.frame);
        if (card.proposal == proposalId) {
            card.frame->raise();
            // Only its picture goes on, and gives way to the real one as it lands.
            for (QWidget *part : card.frame->findChildren<QWidget *>(Qt::FindDirectChildrenOnly))
                if (part != card.poster) part->hide();
            card.poster->setPlain(true);
            glide(card.frame, to, 0, duration);
            if (effect) {
                ease(effect, "angle", 0.0, 0, duration);
                ease(effect, "level", 0.0, duration * 7 / 10, duration * 3 / 10);
            }
        } else {
            glide(card.frame, card.frame->geometry().translated(0, 80), 0, duration);
            if (effect) ease(effect, "level", 0.0, 0, duration / 2);
        }
    }
    QTimer::singleShot(duration, this, [this, done = std::move(done)] {
        m_picking = false;
        done();
    });
}

QStringList CompareView::keys() const { return m_keys; }

QString CompareView::statusText(const QString &key) const { return m_byKey.contains(key) ? m_byKey.value(key).status->text() : QString(); }

ui::Button *CompareView::watchButton(const QString &key) const { return m_byKey.value(key).watch; }

ui::Button *CompareView::useButton(const QString &key) const { return m_byKey.value(key).use; }

void CompareView::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    m_close->move(width() - space::s8 - m_close->width(), space::s8);
    m_close->raise();
    placeCards(false);
}

} // namespace kerf

#include "compare_view.moc"
