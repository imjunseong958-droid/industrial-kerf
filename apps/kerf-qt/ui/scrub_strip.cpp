#include "scrub_strip.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPainter>

namespace kerf {

ScrubStrip::ScrubStrip(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("scrubStrip"));
    setFixedHeight(tokens::layout::scrub);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    m_time = new QLabel;
    m_time->setObjectName(QStringLiteral("scrubTime"));
    m_time->setFont(tokens::font(tokens::timecodeStrong));
    m_time->setAlignment(Qt::AlignCenter);
    QPalette palette = m_time->palette();
    palette.setColor(QPalette::WindowText, tokens::color(tokens::ink));
    m_time->setPalette(palette);
    layout->addWidget(m_time);
    if (m_bus) {
        connect(m_bus, &EditorBus::playheadChanged, this, &ScrubStrip::updateTime);
        connect(m_bus, &EditorBus::snapshotChanged, this, &ScrubStrip::updateTime);
    }
    updateTime();
}

QString ScrubStrip::timeText() const {
    return m_time->text();
}

void ScrubStrip::updateTime() {
    if (!m_bus) return;
    m_time->setText(QStringLiteral("%1 / %2").arg(timelineTimecode(m_bus->playhead(), m_bus->fps()),
                                                  timelineTimecode(m_bus->duration(), m_bus->fps())));
}

QSize ScrubStrip::sizeHint() const {
    return {360, tokens::layout::scrub};
}

void ScrubStrip::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::surround));
}

} // namespace kerf
