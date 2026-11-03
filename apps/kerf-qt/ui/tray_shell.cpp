#include "tray_shell.h"

#include <QAbstractButton>
#include <QAction>
#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>

namespace kerf::shell {

QLabel *text(const QString &content, const tokens::TypeStyle &style, QRgb ink) {
    auto *label = new QLabel(content);
    label->setFont(tokens::font(style));
    QPalette palette = label->palette();
    palette.setColor(QPalette::WindowText, tokens::color(ink));
    label->setPalette(palette);
    return label;
}

TrayPane::TrayPane(const QString &objectName, const QString &name, QLabel *meta, QWidget *body, QWidget *parent)
    : QWidget(parent) {
    setObjectName(objectName);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(1, 0, 1, 4);
    layout->setSpacing(0);
    auto *header = new QHBoxLayout;
    header->setContentsMargins(tokens::space::s12 - 1, 0, tokens::space::s8 - 1, 0);
    header->setSpacing(tokens::space::s8);
    auto *title = text(name, tokens::controlStrong, tokens::ink);
    title->setFixedHeight(tokens::height::h28);
    header->addWidget(title);
    if (meta) header->addWidget(meta);
    header->addStretch(1);
    layout->addLayout(header);
    layout->addWidget(body, 1);
}

void TrayPane::setHighlighted(bool on) {
    if (m_highlighted == on) return;
    m_highlighted = on;
    update();
}

void TrayPane::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    QPainterPath card;
    card.addRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), tokens::radius::card, tokens::radius::card);
    painter.fillPath(card, tokens::color(tokens::card));
    painter.setPen(QPen(tokens::color(m_highlighted ? tokens::accent : tokens::line), 1));
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
