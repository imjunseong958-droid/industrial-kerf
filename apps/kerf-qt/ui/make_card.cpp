// MakeCard: a making Kerf asked for, with Make and Not now (make_card.h).

#include "make_card.h"
#include "components.h"
#include "design_tokens.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

using namespace tokens;

namespace {

QLabel *label(const TypeStyle &style, QRgb ink, bool wrap = false) {
    auto *made = new QLabel;
    made->setFont(tokens::font(style));
    QPalette palette = made->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    made->setPalette(palette);
    made->setWordWrap(wrap);
    made->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return made;
}

void setInk(QLabel *label, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, color(ink));
    label->setPalette(palette);
}

} // namespace

MakeCard::MakeCard(const Ask &ask, QWidget *parent) : QFrame(parent), m_ask(ask), m_name(ask.media) {
    setObjectName(QStringLiteral("makeCard"));
    setAccessibleName(QStringLiteral("Making asked for"));
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(12, 12, 12, 12);
    layout->setSpacing(space::s8);

    auto *head = new QHBoxLayout;
    head->setSpacing(space::s8);
    m_title = label(controlStrong, ink);
    m_title->setObjectName(QStringLiteral("makeCardTitle"));
    head->addWidget(m_title, 1);
    m_takes = label(monoSmall, ink3);
    head->addWidget(m_takes);
    layout->addLayout(head);

    // What would run: the guide's render, the model's program, the take.
    m_plan = label(monoSmall, ink2, true);
    m_plan->setObjectName(QStringLiteral("makeCardPlan"));
    m_plan->setText(planOf(ask.text));
    layout->addWidget(m_plan);

    m_status = label(control, ink2, true);
    m_status->setObjectName(QStringLiteral("makeCardStatus"));
    m_status->hide();
    layout->addWidget(m_status);

    auto *footer = new QHBoxLayout;
    footer->setSpacing(space::s8);
    m_planToggle = new ui::GhostButton(QStringLiteral("Plan"));
    m_planToggle->setObjectName(QStringLiteral("makeCardPlanToggle"));
    m_planToggle->setCheckable(true);
    m_planToggle->setChecked(true);
    m_planToggle->setChevron(true);
    connect(m_planToggle, &QAbstractButton::toggled, m_plan, &QWidget::setVisible);
    footer->addWidget(m_planToggle);
    footer->addStretch(1);
    m_notNow = new ui::Button(QStringLiteral("Not now"), ui::Button::Kind::Secondary);
    m_notNow->setObjectName(QStringLiteral("makeCardNotNow"));
    connect(m_notNow, &QAbstractButton::clicked, this, [this] {
        setState(State::Declined);
        emit declined(m_ask.key());
    });
    footer->addWidget(m_notNow);
    m_stop = new ui::Button(QStringLiteral("Stop"), ui::Button::Kind::Secondary);
    m_stop->setObjectName(QStringLiteral("makeCardStop"));
    connect(m_stop, &QAbstractButton::clicked, this, [this] { emit stopRequested(m_ask.media); });
    footer->addWidget(m_stop);
    m_look = new ui::Button(QStringLiteral("Look at it"), ui::Button::Kind::Secondary);
    m_look->setObjectName(QStringLiteral("makeCardLook"));
    connect(m_look, &QAbstractButton::clicked, this, [this] { emit lookRequested(m_ask.media, m_made); });
    footer->addWidget(m_look);
    m_make = new ui::Button(QStringLiteral("Make"), ui::Button::Kind::Primary);
    m_make->setObjectName(QStringLiteral("makeCardMake"));
    connect(m_make, &QAbstractButton::clicked, this, [this] {
        // Shown at once; the bridge's media_making events take it from here.
        setState(State::Making);
        emit makeRequested(m_ask.media, std::max(1, m_ask.rolls));
    });
    footer->addWidget(m_make);
    layout->addLayout(footer);
    refresh();
}

QString MakeCard::planOf(const QString &text) {
    QStringList kept;
    for (const QString &line : text.split(QLatin1Char('\n'))) {
        // Where else a person makes it is said by the card's own Make.
        if (line.startsWith(QStringLiteral("Make it with ")) || line.startsWith(QStringLiteral("Make them with "))) continue;
        kept << line;
    }
    while (!kept.isEmpty() && kept.constLast().trimmed().isEmpty()) kept.removeLast();
    return kept.join(QLatin1Char('\n'));
}

void MakeCard::setMediaName(const QString &name) {
    m_name = name.isEmpty() ? m_ask.media : name;
    refresh();
}

void MakeCard::setState(State state, const QString &detail, int take) {
    // A card made already stays made: a later making of the same media is
    // another card's.
    if (m_state == State::Made && state != State::Made) return;
    m_state = state;
    m_detail = detail;
    if (take > 0) m_made = take;
    refresh();
}

void MakeCard::setCanMake(bool can) {
    m_canMake = can;
    refresh();
}

QString MakeCard::takesText() const {
    if (m_ask.take <= 0) return {};
    if (m_ask.rolls > 1) return QStringLiteral("takes %1–%2").arg(m_ask.take).arg(m_ask.take + m_ask.rolls - 1);
    return QStringLiteral("take %1").arg(m_ask.take);
}

void MakeCard::refresh() {
    m_takes->setText(takesText());
    const bool asked = m_state == State::Asked;
    const bool again = m_state == State::Stopped || m_state == State::Failed || m_state == State::Declined;
    switch (m_state) {
    case State::Asked:
        m_title->setText(m_ask.rolls > 1 ? QStringLiteral("Make %1 takes of %2?").arg(m_ask.rolls).arg(m_name)
                                          : QStringLiteral("Make %1?").arg(m_name));
        break;
    case State::Making:
        m_title->setText(QStringLiteral("Making %1").arg(m_name));
        break;
    case State::Made:
        m_title->setText(m_made > 0 ? QStringLiteral("Made take %1 of %2").arg(m_made).arg(m_name)
                                    : QStringLiteral("Made %1").arg(m_name));
        break;
    case State::Stopped:
        m_title->setText(QStringLiteral("Stopped making %1").arg(m_name));
        break;
    case State::Failed:
        m_title->setText(QStringLiteral("%1 was not made").arg(m_name));
        break;
    case State::Declined:
        m_title->setText(QStringLiteral("Not made: %1").arg(m_name));
        break;
    }
    QString status;
    if (m_state == State::Making) status = m_detail.isEmpty() ? QStringLiteral("Starting…") : m_detail;
    else if (m_state == State::Failed) status = m_detail;
    else if (m_state == State::Stopped) status = QStringLiteral("The media is as it was.");
    m_status->setText(status);
    m_status->setVisible(!status.isEmpty());
    setInk(m_status, m_state == State::Failed ? neg : ink2);
    // The plan is what is asked about: shown while asked, folded after.
    const bool planShown = asked || again;
    if (m_planToggle->isChecked() != planShown) m_planToggle->setChecked(planShown);
    m_planToggle->setVisible(!m_plan->text().isEmpty());
    m_notNow->setVisible(asked);
    m_make->setVisible(asked || again);
    m_make->setText(again ? QStringLiteral("Make") : (m_ask.rolls > 1 ? QStringLiteral("Make %1").arg(m_ask.rolls) : QStringLiteral("Make")));
    m_make->setEnabled(m_canMake);
    m_stop->setVisible(m_state == State::Making);
    m_look->setVisible(m_state == State::Made && m_made > 0);
    QFrame::update();
}

void MakeCard::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(area, radius::card, radius::card);
    // Asked: an accent stripe inside the top edge, as a card waiting on the person.
    if (m_state == State::Asked) {
        p.save();
        QPainterPath outer;
        outer.addRoundedRect(area, radius::card, radius::card);
        QPainterPath inner;
        inner.addRoundedRect(area.adjusted(0, 2, 0, 0), radius::card, radius::card);
        p.setClipPath(outer);
        p.fillPath(outer.subtracted(inner), color(activity::gpu));
        p.restore();
    }
    ui::paintInsetLine(p, area, radius::card, color(line));
}

} // namespace kerf
