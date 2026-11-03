#include "agent_hud.h"
#include "design_tokens.h"
#include "editor_bus.h"

#include <QHBoxLayout>
#include <QJsonObject>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>

namespace kerf {

AgentHud::AgentHud(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("agentHud"));
    setAttribute(Qt::WA_TransparentForMouseEvents);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(tokens::space::s12, tokens::space::s6, tokens::space::s12, tokens::space::s6);
    m_text = new QLabel;
    m_text->setObjectName(QStringLiteral("agentHudText"));
    m_text->setFont(tokens::font(tokens::control));
    QPalette palette = m_text->palette();
    palette.setColor(QPalette::WindowText, tokens::color(tokens::ink));
    m_text->setPalette(palette);
    layout->addWidget(m_text);
    hide();
    if (m_bus) connect(m_bus, &EditorBus::event, this, &AgentHud::handle);
}

QString AgentHud::text() const {
    return m_busy ? m_text->text() : QString();
}

void AgentHud::handle(const QString &type, const QJsonObject &event) {
    if (type == QStringLiteral("agent_busy")) {
        const bool busy = event.value(QStringLiteral("busy")).toBool();
        if (busy == m_busy) return;
        m_busy = busy;
        if (!busy) m_step.clear();
    } else if (type == QStringLiteral("agent")) {
        const QString kind = event.value(QStringLiteral("kind")).toString();
        if (kind != QStringLiteral("tool") && kind != QStringLiteral("status")) return;
        m_step = event.value(QStringLiteral("text")).toString().simplified();
    } else {
        return;
    }
    m_text->setText(m_step.isEmpty() ? QStringLiteral("Kerf · working") : QStringLiteral("Kerf · %1").arg(m_step));
    setVisible(m_busy);
    emit activeChanged(m_busy);
}

void AgentHud::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    QPainterPath card;
    card.addRoundedRect(QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5), tokens::radius::card, tokens::radius::card);
    painter.fillPath(card, tokens::color(tokens::card));
    painter.setPen(QPen(tokens::color(tokens::line2), 1));
    painter.drawPath(card);
}

} // namespace kerf
