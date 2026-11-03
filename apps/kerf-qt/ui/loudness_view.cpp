#include "loudness_view.h"
#include "tray_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"

#include <QGridLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QVBoxLayout>

#include <cmath>

namespace kerf {

namespace {

// "−23.1", or a dash for a value not measured (null: silence or no signal).
QString lufs(const QJsonValue &value) {
    if (!value.isDouble() || !std::isfinite(value.toDouble())) return QStringLiteral("—");
    const double v = value.toDouble();
    const QString number = QString::number(std::abs(v), 'f', 1);
    return v < 0 ? QStringLiteral("−") + number : number;
}

using shell::text;

} // namespace

LoudnessView::LoudnessView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("loudnessView"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, tokens::space::s8);
    layout->setSpacing(tokens::space::s4);
    auto *stats = new QGridLayout;
    stats->setContentsMargins(0, 0, 0, 0);
    stats->setHorizontalSpacing(tokens::space::s4);
    stats->setVerticalSpacing(0);
    const struct {
        const char *key;
        const char *label;
    } cells[] = {{"M", "M"}, {"S", "S"}, {"I", "I"}, {"LRA", "LRA LU"}};
    int column = 0;
    for (const auto &cell : cells) {
        QLabel *value = text(QStringLiteral("—"), tokens::monoLarge, tokens::ink);
        value->setObjectName(QStringLiteral("loudness%1").arg(QString::fromLatin1(cell.key)));
        stats->addWidget(value, 0, column);
        stats->addWidget(text(QString::fromLatin1(cell.label), tokens::meta, tokens::ink3), 1, column);
        m_values.insert(QString::fromLatin1(cell.key), value);
        ++column;
    }
    layout->addLayout(stats);
    for (const char *channel : {"L", "R"}) {
        auto *row = new QHBoxLayout;
        row->setSpacing(tokens::space::s8);
        row->addWidget(text(QString::fromLatin1(channel), tokens::monoSmall, tokens::ink3));
        row->addStretch(1);
        QLabel *value = text(QStringLiteral("— dBTP"), tokens::monoSmall, tokens::ink2);
        value->setObjectName(QStringLiteral("truePeak%1").arg(QString::fromLatin1(channel)));
        row->addWidget(value);
        layout->addLayout(row);
        m_values.insert(QStringLiteral("TP %1").arg(QString::fromLatin1(channel)), value);
    }
    m_status = text(QStringLiteral("Not measured yet"), tokens::meta, tokens::ink3);
    m_status->setObjectName(QStringLiteral("loudnessStatus"));
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    layout->addStretch(1);
    if (m_bus)
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("loudness")) setLoudness(event);
            else if (type == QStringLiteral("meter")) setMeter(event);
        });
}

void LoudnessView::setLoudness(const QJsonObject &loudness) {
    m_loudness = loudness;
    const QString status = loudness.value(QStringLiteral("status")).toString();
    m_values.value(QStringLiteral("I"))->setText(lufs(loudness.value(QStringLiteral("integrated"))));
    m_values.value(QStringLiteral("LRA"))->setText(lufs(loudness.value(QStringLiteral("lra"))));
    const QJsonArray peaks = loudness.value(QStringLiteral("truePeak")).toArray();
    m_values.value(QStringLiteral("TP L"))->setText(lufs(peaks.at(0)) + QStringLiteral(" dBTP"));
    m_values.value(QStringLiteral("TP R"))->setText(lufs(peaks.at(1)) + QStringLiteral(" dBTP"));
    m_status->setText(status == QStringLiteral("running") ? QStringLiteral("Measuring the programme…")
                      : status == QStringLiteral("failed")
                          ? loudness.value(QStringLiteral("message")).toString(QStringLiteral("The measurement failed"))
                      : measured() ? QStringLiteral("Programme · target %1 LUFS · limit %2 dBTP")
                                         .arg(lufs(loudness.value(QStringLiteral("target")).toDouble(-23)),
                                              lufs(loudness.value(QStringLiteral("limit")).toDouble(-1)))
                                   : QStringLiteral("Not measured yet"));
}

bool LoudnessView::measured() const {
    return m_loudness.value(QStringLiteral("status")).toString() == QStringLiteral("ready");
}

void LoudnessView::setMeter(const QJsonObject &meter) {
    m_values.value(QStringLiteral("M"))->setText(lufs(meter.value(QStringLiteral("momentary"))));
    m_values.value(QStringLiteral("S"))->setText(lufs(meter.value(QStringLiteral("shortTerm"))));
}

QString LoudnessView::statText(const QString &key) const {
    const QLabel *label = m_values.value(key);
    return label ? label->text() : QString();
}

QString LoudnessView::statusText() const {
    return m_status->text();
}

void LoudnessView::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::card));
}

} // namespace kerf
