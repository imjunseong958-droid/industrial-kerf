#include "qc_view.h"
#include "tray_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"

#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QToolButton>
#include <QVBoxLayout>

namespace kerf {

namespace {

using shell::text;

} // namespace

QcView::QcView(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("qcView"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    m_status = text(QStringLiteral("QC has not run"), tokens::meta, tokens::ink3);
    m_status->setObjectName(QStringLiteral("qcStatus"));
    m_status->setContentsMargins(tokens::space::s12, tokens::space::s4, tokens::space::s12, tokens::space::s4);
    m_status->setWordWrap(true);
    layout->addWidget(m_status);
    m_rows = new QWidget;
    m_rowLayout = new QVBoxLayout(m_rows);
    m_rowLayout->setContentsMargins(0, 0, 0, 0);
    m_rowLayout->setSpacing(0);
    layout->addWidget(m_rows);
    layout->addStretch(1);
    if (m_bus) {
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("check")) setCheck(event);
        });
        connect(this, &QcView::threadRequested, m_bus, &EditorBus::openThread);
    }
}

void QcView::setCheck(const QJsonObject &check) {
    m_check = check;
    rebuild();
}

QString QcView::summaryText() const {
    const QJsonObject summary = m_check.value(QStringLiteral("summary")).toObject();
    if (summary.isEmpty()) return {};
    return QStringLiteral("%1 of %2 pass").arg(summary.value(QStringLiteral("passed")).toInt()).arg(summary.value(QStringLiteral("total")).toInt());
}

int QcView::rowCount() const {
    return m_rowLayout->count();
}

QString QcView::statusText() const {
    return m_status->isHidden() ? QString() : m_status->text();
}

void QcView::rebuild() {
    while (QLayoutItem *item = m_rowLayout->takeAt(0)) {
        delete item->widget();
        delete item;
    }
    const QString status = m_check.value(QStringLiteral("status")).toString();
    const QJsonArray rows = m_check.value(QStringLiteral("summary")).toObject().value(QStringLiteral("rows")).toArray();
    m_status->setText(status == QStringLiteral("running") ? QStringLiteral("Running QC…")
                      : status == QStringLiteral("failed")
                          ? m_check.value(QStringLiteral("message")).toString(QStringLiteral("QC did not finish"))
                      : rows.isEmpty() ? QStringLiteral("QC has not run")
                                       : QString());
    m_status->setVisible(!m_status->text().isEmpty());
    for (const QJsonValue &value : rows) {
        const QJsonObject row = value.toObject();
        const bool passed = row.value(QStringLiteral("passed")).toBool();
        auto *line = new QWidget;
        line->setObjectName(QStringLiteral("qcRow"));
        line->setFixedHeight(tokens::height::h28);
        auto *layout = new QHBoxLayout(line);
        layout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
        layout->setSpacing(tokens::space::s8);
        auto *dot = new QLabel;
        dot->setObjectName(passed ? QStringLiteral("qcPass") : QStringLiteral("qcFail"));
        dot->setFixedSize(6, 6);
        const QColor color = tokens::color(passed ? tokens::pos : tokens::neg);
        dot->setStyleSheet(QStringLiteral("background: %1; border-radius: 3px;").arg(color.name()));
        layout->addWidget(dot);
        layout->addWidget(text(row.value(QStringLiteral("name")).toString(row.value(QStringLiteral("code")).toString()), tokens::control,
                               tokens::ink));
        const QString measured = row.value(QStringLiteral("value")).toVariant().toString();
        const QString limit = row.value(QStringLiteral("limit")).toVariant().toString();
        QLabel *valueLabel = text(limit.isEmpty() ? measured : QStringLiteral("%1 · %2").arg(measured, limit), tokens::monoSmall,
                                  passed ? tokens::ink2 : tokens::neg);
        valueLabel->setObjectName(QStringLiteral("qcValue"));
        layout->addWidget(valueLabel, 1);
        const QString thread = row.value(QStringLiteral("commentId")).toString();
        if (!passed && !thread.isEmpty()) {
            auto *open = new QToolButton;
            open->setObjectName(QStringLiteral("qcInThread"));
            open->setText(QStringLiteral("In thread"));
            open->setAutoRaise(true);
            open->setFont(tokens::font(tokens::control));
            connect(open, &QToolButton::clicked, this, [this, thread] { emit threadRequested(thread); });
            layout->addWidget(open);
        } else {
            QLabel *state = text(passed ? QStringLiteral("Pass") : QStringLiteral("Fail"), tokens::control,
                                 passed ? tokens::ink3 : tokens::neg);
            state->setFixedWidth(64);
            state->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
            layout->addWidget(state);
        }
        m_rowLayout->addWidget(line);
    }
}

} // namespace kerf
