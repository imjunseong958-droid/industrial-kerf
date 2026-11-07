#include "tray_shell.h"

#include "components.h"

#include <QAbstractButton>
#include <QAction>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>

namespace kerf::shell {

QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink) {
    auto *label = new QLabel(content);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, tokens::color(ink));
    label->setPalette(palette);
    setTextStyle(label, style, ink);
    return label;
}

void setTextStyle(QLabel *label, const tokens::TypeStyle &style, QRgb ink) {
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, tokens::color(ink));
    label->setPalette(palette);
    label->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: %2px; font-weight: %3; background: transparent; }")
                             .arg(tokens::color(ink).name(QColor::HexRgb))
                             .arg(style.px)
                             .arg(style.weight));
}

TrayPane::TrayPane(const QString &objectName, const QString &name, QLabel *meta, QWidget *body, QWidget *parent)
    : QWidget(parent) {
    setObjectName(objectName);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(1, 0, 1, 4);
    layout->setSpacing(0);
    auto *header = m_header = new QHBoxLayout;
    header->setContentsMargins(tokens::space::s12 - 1, 0, tokens::space::s8 - 1, 0);
    header->setSpacing(tokens::space::s8);
    auto *title = text(name, tokens::controlStrong, tokens::ink);
    title->setFixedHeight(tokens::height::h28);
    // The name stays whole, and what follows it gives way.
    title->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    title->setMinimumWidth(title->sizeHint().width());
    header->addWidget(title);
    if (meta) header->addWidget(meta);
    header->addStretch(1);
    layout->addLayout(header);
    layout->addWidget(body, 1);
}

void TrayPane::addTrailing(QWidget *widget) {
    if (widget) m_header->addWidget(widget);
}

void TrayPane::setHighlighted(bool on) {
    if (m_highlighted == on) return;
    m_highlighted = on;
    const double end = on ? 1.0 : 0.0;
    const int duration = ui::motionDuration(tokens::motion::fade);
    if (!m_fade) {
        m_fade = new QVariantAnimation(this);
        m_fade->setEasingCurve(tokens::easing());
        connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
            m_level = value.toDouble();
            update();
        });
    }
    m_fade->stop();
    if (duration <= 0 || !isVisible()) {
        m_level = end;
        update();
        return;
    }
    m_fade->setDuration(duration);
    m_fade->setStartValue(m_level);
    m_fade->setEndValue(end);
    m_fade->start();
}

QColor TrayPane::outlineColor() const {
    return ui::mix(tokens::color(tokens::line), tokens::color(tokens::accent), std::clamp(m_level, 0.0, 1.0));
}

void TrayPane::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    QPainterPath card;
    card.addRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), tokens::radius::card, tokens::radius::card);
    painter.fillPath(card, tokens::color(tokens::card));
    painter.setPen(QPen(outlineColor(), 1));
    painter.drawPath(card);
}

void followAction(QAbstractButton *button, const QAction *action, const QString &blocked) {
    if (!button) return;
    const bool available = action && action->isEnabled();
    button->setEnabled(available && blocked.isEmpty());
    // The registry's reason first (offline, say), then what holds it back here.
    button->setToolTip(!action ? QString() : available && !blocked.isEmpty() ? blocked : action->toolTip());
}

} // namespace kerf::shell
