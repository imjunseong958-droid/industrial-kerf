// ModalScrim: a panel over the dimmed window (modal_scrim.h).

#include "modal_scrim.h"
#include "design_tokens.h"

#include <QMouseEvent>
#include <QPainter>

#include <algorithm>

namespace kerf {

namespace {

// The room kept between the panel and the window's edges.
constexpr int kMargin = 48;

} // namespace

ModalScrim::ModalScrim(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("modalScrim"));
    setAttribute(Qt::WA_NoSystemBackground);
}

void ModalScrim::setPanel(QWidget *panel) {
    m_panel = panel;
    if (!panel) return;
    panel->setParent(this);
    panel->setGeometry(panelRect());
    panel->show();
}

void ModalScrim::setPanelSize(const QSize &size) {
    m_most = size;
    if (m_panel) m_panel->setGeometry(panelRect());
}

QRect ModalScrim::panelRect() const {
    const int width = std::max(0, std::min(m_most.width(), this->width() - 2 * kMargin));
    const int height = std::max(0, std::min(m_most.height(), this->height() - 2 * kMargin));
    return QRect((this->width() - width) / 2, (this->height() - height) / 2, width, height);
}

void ModalScrim::paintEvent(QPaintEvent *) {
    QPainter p(this);
    // The work under it, dimmed: still there, not in the way.
    p.fillRect(rect(), QColor(0, 0, 0, 140));
    if (!m_panel) return;
    // A soft shadow round the panel lifts it off the dim.
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF panel = QRectF(m_panel->geometry());
    for (int ring = 1; ring <= 12; ++ring) {
        p.setPen(QPen(QColor(0, 0, 0, std::max(0, 26 - 2 * ring)), 1));
        p.setBrush(Qt::NoBrush);
        p.drawRoundedRect(panel.adjusted(-ring, -ring + 4, ring, ring + 4), tokens::radius::card + ring, tokens::radius::card + ring);
    }
}

void ModalScrim::resizeEvent(QResizeEvent *) {
    if (m_panel) m_panel->setGeometry(panelRect());
}

void ModalScrim::mousePressEvent(QMouseEvent *event) {
    if (m_panel && !m_panel->geometry().contains(event->position().toPoint())) {
        event->accept();
        emit dismissed();
        return;
    }
    QWidget::mousePressEvent(event);
}

} // namespace kerf
