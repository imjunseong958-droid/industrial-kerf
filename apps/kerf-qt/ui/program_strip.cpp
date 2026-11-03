#include "program_strip.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"

#include <QAction>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QPainter>

namespace kerf {

namespace {

QString css(QRgb color) {
    const QColor c = tokens::color(color);
    return QStringLiteral("rgba(%1,%2,%3,%4)").arg(c.red()).arg(c.green()).arg(c.blue()).arg(c.alphaF(), 0, 'f', 3);
}

// The strip's actions by object name: the kit icon, and the name its tip
// gives (mockup-spec 2 and 15).
struct StripEntry {
    const char *object;
    const char *icon;
    const char *tip;
};
constexpr StripEntry kEntries[] = {
    {"maskRectangle", "g-mrect", "Rectangle mask"},
    {"maskEllipse", "g-mell", "Ellipse mask"},
    {"maskPen", "g-mpen", "Pen mask"},
    {"view.annotations", "g-annot", "Annotations"},
    {"view.safeZones", "g-safe", "Safe zones · action 93%, title 90%"},
    {"view.dim", "g-dim", "Dim surround"},
    {"viewer.zoom", "", "Viewer zoom"},
};

const StripEntry *entryFor(const QAction *action) {
    if (!action) return nullptr;
    for (const StripEntry &entry : kEntries)
        if (action->objectName() == QLatin1String(entry.object)) return &entry;
    return nullptr;
}

bool isZoom(const QAction *action) { return action && action->objectName() == QLatin1String("viewer.zoom"); }

} // namespace

ProgramStrip::ProgramStrip(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("programStrip"));
    setFixedHeight(tokens::layout::strip);
    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    layout->setSpacing(tokens::space::s4);
    m_title = new QLabel(QStringLiteral("Program"));
    m_title->setObjectName(QStringLiteral("programStripTitle"));
    m_title->setFont(tokens::font(tokens::controlStrong));
    m_title->setStyleSheet(QStringLiteral("color: %1;").arg(css(tokens::ink)));
    layout->addWidget(m_title);
    // The preview text, 8 after the title, its dot and words 8 apart.
    layout->addSpacing(tokens::space::s8 - tokens::space::s4);
    m_previewDot = new ui::Dot(tokens::color(tokens::ink2));
    m_previewDot->setObjectName(QStringLiteral("programPreviewDot"));
    m_previewDot->setFixedSize(6, 6);
    m_previewDot->hide();
    layout->addWidget(m_previewDot, 0, Qt::AlignVCenter);
    layout->addSpacing(tokens::space::s8 - tokens::space::s4);
    m_preview = new QLabel;
    m_preview->setObjectName(QStringLiteral("programPreview"));
    m_preview->setFont(tokens::font(tokens::control));
    m_preview->setStyleSheet(QStringLiteral("color: %1;").arg(css(tokens::ink3)));
    m_preview->hide();
    layout->addWidget(m_preview);
    m_exitPreview = new ui::GhostButton(QStringLiteral("Exit preview"));
    m_exitPreview->setObjectName(QStringLiteral("exitPreview"));
    m_exitPreview->setTip(QStringLiteral("Exit preview"));
    m_exitPreview->hide();
    connect(m_exitPreview, &QAbstractButton::clicked, this, [this] {
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

QString ProgramStrip::iconFor(const QAction *action) {
    const StripEntry *entry = entryFor(action);
    return entry ? QString::fromLatin1(entry->icon) : QString();
}

QString ProgramStrip::tipFor(const QAction *action) {
    if (const StripEntry *entry = entryFor(action)) return QString::fromUtf8(entry->tip);
    return action ? action->text().remove(QLatin1Char('&')) : QString();
}

void ProgramStrip::follow(QAbstractButton *button, const QAction *action) {
    if (!button || !action) return;
    button->setEnabled(action->isEnabled());
    if (action->isCheckable()) {
        button->setCheckable(true);
        const QSignalBlocker blocker(button);
        button->setChecked(action->isChecked());
    }
    // name · shortcut, and why while it is unavailable (the registry keeps
    // the reason as the action's status tip).
    QString name = tipFor(action);
    QString reason = action->statusTip();
    // The mask tools wait for a picture clip to be selected.
    if (reason.isEmpty() && action->objectName().startsWith(QLatin1String("mask"))) reason = QStringLiteral("Select a picture clip");
    if (!action->isEnabled() && !reason.isEmpty()) name = QStringLiteral("%1 · %2").arg(name, reason);
    ui::setTip(button, name, action->shortcut().toString(QKeySequence::NativeText));
    if (auto *ghost = qobject_cast<ui::GhostButton *>(button)) ghost->setText(m_zoomText);
}

void ProgramStrip::setActions(const QList<QAction *> &actions) {
    for (QWidget *widget : std::as_const(m_trailingWidgets)) widget->deleteLater();
    m_trailingWidgets.clear();
    m_buttons.clear();
    m_actions = actions;
    for (QAction *action : actions) {
        QWidget *widget = nullptr;
        if (!action) {
            auto *line = new ui::Separator(tokens::space::s4);
            line->setObjectName(QStringLiteral("programStripSeparator"));
            widget = line;
        } else {
            QAbstractButton *button = nullptr;
            if (isZoom(action)) {
                auto *ghost = new ui::GhostButton(m_zoomText);
                ghost->setChevron(true);
                connect(ghost, &QAbstractButton::clicked, this, [this, ghost] { emit zoomMenuRequested(ghost); });
                button = ghost;
            } else {
                auto *icon = new ui::IconButton(iconFor(action));
                // A press runs the action, as its menu item and key do; the
                // action's own state then comes back to the button.
                connect(icon, &QAbstractButton::clicked, action, [action, icon] {
                    if (action->isCheckable()) {
                        // The button flipped itself; the action decides.
                        const QSignalBlocker blocker(icon);
                        icon->setChecked(action->isChecked());
                    }
                    action->trigger();
                });
                button = icon;
            }
            button->setObjectName(action->objectName());
            connect(action, &QAction::changed, button, [this, button, action] { follow(button, action); });
            connect(action, &QAction::toggled, button, [this, button, action] { follow(button, action); });
            m_buttons.append({button, action});
            follow(button, action);
            widget = button;
        }
        m_trailing->addWidget(widget, 0, Qt::AlignVCenter);
        m_trailingWidgets.append(widget);
    }
}

QAbstractButton *ProgramStrip::buttonFor(const QAction *action) const {
    for (const auto &[button, owner] : m_buttons)
        if (owner == action) return button;
    return nullptr;
}

QAbstractButton *ProgramStrip::zoomButton() const {
    for (const auto &[button, owner] : m_buttons)
        if (isZoom(owner)) return button;
    return nullptr;
}

void ProgramStrip::setZoomText(const QString &text) {
    m_zoomText = text;
    if (auto *ghost = qobject_cast<ui::GhostButton *>(zoomButton())) {
        ghost->setText(text);
        ghost->updateGeometry();
    }
}

void ProgramStrip::setDetail(const QString &detail) {
    m_detail = detail;
    ui::setTip(m_title, detail.isEmpty() ? QStringLiteral("Program") : QStringLiteral("Program · %1").arg(detail));
}

bool ProgramStrip::previewShown() const { return !m_preview->isHidden(); }

QString ProgramStrip::previewText() const { return m_preview->text(); }

QAbstractButton *ProgramStrip::exitPreviewButton() const { return m_exitPreview; }

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

QSize ProgramStrip::sizeHint() const { return {QWidget::sizeHint().width(), tokens::layout::strip}; }

QSize ProgramStrip::minimumSizeHint() const { return {0, tokens::layout::strip}; }

void ProgramStrip::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, height() - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
