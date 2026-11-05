#include "compare_view.h"

#include "components.h"
#include "design_tokens.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPalette>
#include <QVBoxLayout>

namespace kerf {

using namespace tokens;

namespace {

void setInk(QLabel *label, QRgb ink_) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink_));
    label->setPalette(palette);
}

constexpr int kMostChanges = 4;

// The card's fill, painted: the app's sheet leaves frames clear.
class CompareCard final : public QFrame {
public:
    using QFrame::QFrame;

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(rect(), radius::card, radius::card);
    }
};

} // namespace

ComparePoster::ComparePoster(QWidget *parent) : QWidget(parent) {
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMinimumHeight(height::h40);
}

void ComparePoster::setImage(const QImage &image) {
    m_image = image;
    update();
}

void ComparePoster::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::SmoothPixmapTransform, true);
    p.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath clip;
    clip.addRoundedRect(QRectF(rect()), radius::small, radius::small);
    p.setClipPath(clip);
    p.fillRect(rect(), color(well));
    if (m_image.isNull()) return;
    // It fills the card; a poster only has to say which clip this is.
    QSize shown = m_image.size();
    shown.scale(size(), Qt::KeepAspectRatioByExpanding);
    const QRect target(QPoint((width() - shown.width()) / 2, (height() - shown.height()) / 2), shown);
    p.drawImage(target, m_image);
}

CompareView::CompareView(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("compareView"));
    auto *column = new QVBoxLayout(this);
    column->setContentsMargins(space::s12, 0, space::s8, space::s12);
    column->setSpacing(space::s4);
    auto *header = new QHBoxLayout;
    header->setContentsMargins(0, 0, 0, 0);
    auto *title = new QLabel(QStringLiteral("Versions"));
    title->setObjectName(QStringLiteral("compareTitle"));
    title->setFont(tokens::font(tokens::title));
    setInk(title, ink);
    header->addWidget(title);
    header->addStretch(1);
    m_close = new ui::IconButton(QStringLiteral("g-close"));
    m_close->setObjectName(QStringLiteral("compareClose"));
    m_close->setAccessibleName(QStringLiteral("Close versions"));
    m_close->setTip(QStringLiteral("Close versions"));
    connect(m_close, &QAbstractButton::clicked, this, &CompareView::closeRequested);
    header->addWidget(m_close);
    auto *headerRow = new QWidget;
    headerRow->setFixedHeight(height::h40);
    headerRow->setLayout(header);
    column->addWidget(headerRow);
    m_cards = new QHBoxLayout;
    m_cards->setContentsMargins(0, 0, space::s4, 0);
    m_cards->setSpacing(space::s8);
    column->addLayout(m_cards, 1);
}

CompareView::Card &CompareView::cardFor(const QString &key) {
    auto found = m_byKey.find(key);
    if (found != m_byKey.end()) return *found;
    Card card;
    card.frame = new CompareCard;
    card.frame->setObjectName(QStringLiteral("compareCard"));
    auto *inner = new QVBoxLayout(card.frame);
    inner->setContentsMargins(space::s8, space::s8, space::s8, space::s8);
    inner->setSpacing(space::s6);
    card.poster = new ComparePoster;
    inner->addWidget(card.poster, 1);
    auto *name = new QLabel(QStringLiteral("Version %1").arg(key));
    name->setFont(tokens::font(bodyName));
    setInk(name, ink);
    inner->addWidget(name);
    card.status = new QLabel;
    card.status->setFont(tokens::font(control));
    card.status->setWordWrap(true);
    setInk(card.status, ink2);
    inner->addWidget(card.status);
    card.changes = new QLabel;
    card.changes->setFont(tokens::font(meta));
    card.changes->setWordWrap(true);
    setInk(card.changes, ink2);
    inner->addWidget(card.changes);
    auto *actions = new QHBoxLayout;
    actions->setContentsMargins(0, space::s4, 0, 0);
    actions->setSpacing(space::s4);
    card.watch = new ui::Button(QStringLiteral("Watch"), ui::Button::Kind::Secondary);
    card.watch->setObjectName(QStringLiteral("compareWatch"));
    card.use = new ui::Button(QStringLiteral("Use this"), ui::Button::Kind::Primary);
    card.use->setObjectName(QStringLiteral("compareUse"));
    actions->addWidget(card.watch);
    actions->addStretch(1);
    actions->addWidget(card.use);
    inner->addLayout(actions);
    m_cards->addWidget(card.frame, 1);
    connect(card.watch, &QAbstractButton::clicked, this, [this, key] { emit watchRequested(m_byKey.value(key).proposal); });
    connect(card.use, &QAbstractButton::clicked, this, [this, key] { emit useRequested(m_byKey.value(key).proposal); });
    return *m_byKey.insert(key, card);
}

void CompareView::setVersions(const QList<Version> &versions) {
    QStringList keys;
    for (const Version &version : versions) {
        keys << version.key;
        Card &card = cardFor(version.key);
        card.proposal = version.proposalId;
        card.poster->setImage(version.poster);
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
        const bool usable = !version.proposalId.isEmpty();
        card.watch->setEnabled(usable);
        card.use->setEnabled(usable && version.state == State::Ready);
    }
    const QStringList gone = m_byKey.keys();
    for (const QString &key : gone) {
        if (keys.contains(key)) continue;
        Card card = m_byKey.take(key);
        m_cards->removeWidget(card.frame);
        card.frame->deleteLater();
    }
    // In the order given, however they first came.
    for (const QString &key : std::as_const(keys)) m_cards->removeWidget(m_byKey.value(key).frame);
    for (const QString &key : std::as_const(keys)) m_cards->addWidget(m_byKey.value(key).frame, 1);
    m_keys = keys;
}

QStringList CompareView::keys() const { return m_keys; }

QString CompareView::statusText(const QString &key) const { return m_byKey.contains(key) ? m_byKey.value(key).status->text() : QString(); }

ui::Button *CompareView::watchButton(const QString &key) const { return m_byKey.value(key).watch; }

ui::Button *CompareView::useButton(const QString &key) const { return m_byKey.value(key).use; }

void CompareView::paintEvent(QPaintEvent *event) {
    QPainter p(this);
    p.fillRect(event->rect(), color(chrome));
}

} // namespace kerf
