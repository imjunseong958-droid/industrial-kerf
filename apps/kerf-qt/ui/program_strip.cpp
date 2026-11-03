#include "program_strip.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_theme.h"

#include <QAction>
#include <QFrame>
#include <QHBoxLayout>
#include <QJsonObject>
#include <QLabel>
#include <QPainter>
#include <QToolButton>

namespace kerf {

namespace {

QString css(QRgb color) {
    const QColor c = tokens::color(color);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

QString css(const QColor &c) {
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

// The strip's tools as the title-bar tools were: bare on the chrome, the
// hover wash under the pointer and the pressed one held; the chosen tool (a
// mask tool, a switch that is on) the accent tint, darker while held.
QString toolSheet() {
    const EditorTheme &theme = editorTheme();
    return QStringLiteral(
               "QToolButton { background: transparent; border: 1px solid transparent; border-radius: 6px; padding: 0 4px; }"
               "QToolButton:hover { background: %1; border-color: transparent; }"
               "QToolButton:pressed { background: %2; border-color: transparent; }"
               "QToolButton:checked:enabled, QToolButton:checked:enabled:hover { background: %3; border-color: transparent; }"
               "QToolButton:checked:enabled:pressed { background: %4; border-color: transparent; }"
               "QToolButton:disabled { background: transparent; border-color: transparent; }")
        .arg(css(theme.raised), css(theme.on), css(theme.accentTint), css(theme.accentTintPressed));
}

} // namespace

ProgramStrip::ProgramStrip(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("programStrip"));
    setFixedHeight(tokens::layout::strip);
    setStyleSheet(toolSheet());
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    layout->setSpacing(tokens::space::s4);
    m_title = new QLabel(QStringLiteral("Program"));
    m_title->setObjectName(QStringLiteral("programStripTitle"));
    m_title->setFont(tokens::font(tokens::controlStrong));
    m_title->setStyleSheet(QStringLiteral("color: %1;").arg(css(tokens::ink)));
    layout->addWidget(m_title);
    layout->addSpacing(tokens::space::s8 - tokens::space::s4);
    m_previewDot = new QLabel;
    m_previewDot->setFixedSize(6, 6);
    m_previewDot->setStyleSheet(QStringLiteral("background: %1; border-radius: 3px;").arg(css(tokens::ink2)));
    m_previewDot->hide();
    layout->addWidget(m_previewDot);
    m_preview = new QLabel;
    m_preview->setObjectName(QStringLiteral("programPreview"));
    m_preview->setFont(tokens::font(tokens::control));
    m_preview->setStyleSheet(QStringLiteral("color: %1;").arg(css(tokens::ink3)));
    m_preview->hide();
    layout->addWidget(m_preview);
    m_exitPreview = new QToolButton;
    m_exitPreview->setObjectName(QStringLiteral("exitPreview"));
    m_exitPreview->setText(QStringLiteral("Exit preview"));
    m_exitPreview->setAutoRaise(true);
    m_exitPreview->setFont(tokens::font(tokens::control));
    m_exitPreview->setFixedHeight(tokens::height::h24);
    m_exitPreview->hide();
    connect(m_exitPreview, &QToolButton::clicked, this, [this] {
        if (m_bus) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview_exit")}});
        emit exitPreviewRequested();
    });
    layout->addWidget(m_exitPreview);
    layout->addStretch(1);
    m_trailing = new QHBoxLayout;
    m_trailing->setContentsMargins(0, 0, 0, 0);
    m_trailing->setSpacing(tokens::space::s4);
    layout->addLayout(m_trailing);
    if (m_bus)
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("preview")) showPreview(event);
            else if (type == QStringLiteral("viewer_reset")) showPreview(QJsonObject{{QStringLiteral("status"), QStringLiteral("exited")}});
        });
}

void ProgramStrip::setActions(const QList<QAction *> &actions) {
    for (QWidget *widget : std::as_const(m_trailingWidgets)) widget->deleteLater();
    m_trailingWidgets.clear();
    m_actions = actions;
    for (QAction *action : actions) {
        QWidget *widget = nullptr;
        if (!action) {
            auto *line = new QFrame;
            line->setObjectName(QStringLiteral("programStripSeparator"));
            line->setFixedSize(1, 16);
            line->setStyleSheet(QStringLiteral("background: %1;").arg(css(tokens::line)));
            widget = line;
        } else {
            auto *button = new QToolButton;
            button->setDefaultAction(action);
            button->setObjectName(action->objectName());
            button->setAutoRaise(true);
            button->setFixedHeight(tokens::height::h24);
            button->setMinimumWidth(tokens::height::h24);
            button->setFont(tokens::font(tokens::control));
            button->setToolButtonStyle(action->icon().isNull() ? Qt::ToolButtonTextOnly : Qt::ToolButtonIconOnly);
            button->setIconSize(QSize(16, 16));
            widget = button;
        }
        m_trailing->addWidget(widget, 0, Qt::AlignVCenter);
        m_trailingWidgets.append(widget);
    }
}

QToolButton *ProgramStrip::buttonFor(const QAction *action) const {
    for (QWidget *widget : m_trailingWidgets)
        if (auto *button = qobject_cast<QToolButton *>(widget); button && button->defaultAction() == action) return button;
    return nullptr;
}

bool ProgramStrip::previewShown() const {
    return !m_preview->isHidden();
}

QString ProgramStrip::previewText() const {
    return m_preview->text();
}

void ProgramStrip::showPreview(const QJsonObject &event) {
    const QString status = event.value(QStringLiteral("status")).toString();
    const QString proposal = event.value(QStringLiteral("proposalId")).toString();
    const bool on = !proposal.isEmpty() &&
                    (status == QStringLiteral("running") || status == QStringLiteral("ready") || status == QStringLiteral("live"));
    if (on) {
        // The version the proposal is at, from the snapshot's proposals.
        int version = 0;
        if (m_bus)
            for (const QJsonValue &value : m_bus->snapshot().value(QStringLiteral("proposals")).toArray()) {
                const QJsonObject entry = value.toObject();
                bool matches = entry.value(QStringLiteral("proposalId")).toString() == proposal;
                for (const QJsonValue &v : entry.value(QStringLiteral("versions")).toArray())
                    if (v.toObject().value(QStringLiteral("proposalId")).toString() == proposal) {
                        matches = true;
                        version = v.toObject().value(QStringLiteral("version")).toInt();
                    }
                if (matches && version == 0) version = entry.value(QStringLiteral("version")).toInt();
            }
        m_preview->setText(version > 0 ? QStringLiteral("Previewing proposal v%1").arg(version) : QStringLiteral("Previewing proposal"));
    } else {
        // Stopped, exited, failed, or the program's own preview.
        m_preview->clear();
    }
    const bool shown = !m_preview->text().isEmpty();
    m_previewDot->setVisible(shown);
    m_preview->setVisible(shown);
    m_exitPreview->setVisible(shown);
}

QSize ProgramStrip::sizeHint() const {
    return {QWidget::sizeHint().width(), tokens::layout::strip};
}

QSize ProgramStrip::minimumSizeHint() const {
    return {0, tokens::layout::strip};
}

void ProgramStrip::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, height() - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
