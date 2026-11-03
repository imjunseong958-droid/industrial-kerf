#include "deliver_pane.h"
#include "tray_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"

#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QVBoxLayout>

namespace kerf {

namespace {

using shell::text;

} // namespace

DeliverPane::DeliverPane(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("deliverPane"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *header = new QWidget;
    header->setFixedHeight(44);
    auto *headerLayout = new QVBoxLayout(header);
    headerLayout->setContentsMargins(tokens::space::s12, tokens::space::s4, tokens::space::s8, tokens::space::s4);
    headerLayout->setSpacing(0);
    m_title = text(QStringLiteral("Master"), tokens::title, tokens::ink);
    m_title->setObjectName(QStringLiteral("deliverTitle"));
    m_file = text(QStringLiteral("No master output yet"), tokens::monoSmall, tokens::ink3);
    m_file->setObjectName(QStringLiteral("deliverFile"));
    headerLayout->addWidget(m_title);
    headerLayout->addWidget(m_file);
    layout->addWidget(header);
    m_body = new QWidget;
    m_bodyLayout = new QVBoxLayout(m_body);
    m_bodyLayout->setContentsMargins(0, 0, 0, 0);
    m_bodyLayout->setSpacing(0);
    layout->addWidget(m_body);
    layout->addStretch(1);
}

void DeliverPane::setOutput(const QJsonObject &output) {
    m_output = output;
    m_title->setText(output.value(QStringLiteral("title")).toString(QStringLiteral("Master")));
    m_file->setText(output.value(QStringLiteral("file")).toString(QStringLiteral("No master output yet")));
    while (QLayoutItem *item = m_bodyLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    for (const QJsonValue &groupValue : output.value(QStringLiteral("groups")).toArray()) {
        const QJsonObject group = groupValue.toObject();
        QLabel *heading = text(group.value(QStringLiteral("title")).toString(), tokens::controlStrong, tokens::ink);
        heading->setFixedHeight(tokens::height::h28);
        heading->setContentsMargins(tokens::space::s12, 0, tokens::space::s8, 0);
        m_bodyLayout->addWidget(heading);
        for (const QJsonValue &rowValue : group.value(QStringLiteral("rows")).toArray()) {
            const QJsonArray row = rowValue.toArray();
            auto *line = new QWidget;
            line->setObjectName(QStringLiteral("deliverRow"));
            line->setFixedHeight(tokens::height::h24);
            auto *rowLayout = new QHBoxLayout(line);
            rowLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
            QLabel *label = text(row.at(0).toString(), tokens::control, tokens::ink2);
            label->setObjectName(QStringLiteral("deliverLabel"));
            QLabel *value = text(row.at(1).toString(), tokens::timecode, tokens::ink);
            value->setObjectName(QStringLiteral("deliverValue"));
            value->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
            rowLayout->addWidget(label);
            rowLayout->addWidget(value, 1);
            m_bodyLayout->addWidget(line);
        }
    }
}

QString DeliverPane::titleText() const {
    return m_title->text();
}

QString DeliverPane::fileText() const {
    return m_file->text();
}

QString DeliverPane::value(const QString &label) const {
    for (const QJsonValue &group : m_output.value(QStringLiteral("groups")).toArray())
        for (const QJsonValue &row : group.toObject().value(QStringLiteral("rows")).toArray())
            if (row.toArray().at(0).toString() == label) return row.toArray().at(1).toString();
    return {};
}

void DeliverPane::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, 43, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
