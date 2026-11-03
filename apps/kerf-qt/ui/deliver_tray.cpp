#include "deliver_tray.h"
#include "tray_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "loudness_view.h"
#include "qc_view.h"

#include <QAction>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QProgressBar>
#include <QPushButton>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

using shell::TrayPane;
using shell::text;

DeliverTray::DeliverTray(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("deliverTray"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *bar = new QWidget;
    bar->setObjectName(QStringLiteral("deliverBar"));
    bar->setFixedHeight(tokens::layout::strip);
    auto *barLayout = new QHBoxLayout(bar);
    barLayout->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    barLayout->setSpacing(tokens::space::s8);
    barLayout->addWidget(text(QStringLiteral("Delivery"), tokens::controlStrong, tokens::ink));
    m_project = text(QString(), tokens::meta, tokens::ink3);
    m_project->setObjectName(QStringLiteral("deliverProject"));
    barLayout->addWidget(m_project);
    barLayout->addStretch(1);
    m_blockingLabel = text(QString(), tokens::meta, tokens::ink3);
    m_blockingLabel->setObjectName(QStringLiteral("deliverBlocking"));
    barLayout->addWidget(m_blockingLabel);
    m_render = new QPushButton(QStringLiteral("Render master"));
    m_render->setObjectName(QStringLiteral("deliverRender"));
    m_render->setFont(tokens::font(tokens::controlStrong));
    m_render->setFixedHeight(tokens::height::h24);
    connect(m_render, &QPushButton::clicked, this, [this] {
        if (m_bus && m_blocking == 0) m_bus->run(QStringLiteral("export.master"));
    });
    barLayout->addWidget(m_render);
    layout->addWidget(bar);

    auto *panes = new QHBoxLayout;
    panes->setContentsMargins(tokens::space::s8, tokens::space::s8, tokens::space::s8, tokens::space::s8);
    panes->setSpacing(tokens::space::s8);
    m_masters = new QWidget;
    m_masters->setObjectName(QStringLiteral("deliverMasters"));
    m_masterRows = new QVBoxLayout(m_masters);
    m_masterRows->setContentsMargins(0, 0, 0, 0);
    m_masterRows->setSpacing(0);
    m_masterRows->addStretch(1);
    m_loudness = new LoudnessView(nullptr, bus);
    m_qc = new QcView(nullptr, bus);
    m_mastersMeta = text(QStringLiteral("No renders yet"), tokens::meta, tokens::ink3);
    m_qcMeta = text(QString(), tokens::meta, tokens::ink3);
    auto *paperwork = new QLabel(QStringLiteral("Coming in this release"));
    paperwork->setObjectName(QStringLiteral("deliverPaperworkEmpty"));
    paperwork->setAlignment(Qt::AlignCenter);
    paperwork->setFont(tokens::font(tokens::meta));
    const struct {
        const char *id;
        const char *name;
        QLabel *meta;
        QWidget *body;
        int width;
        int stretch;
    } specs[] = {{"masters", "Masters", m_mastersMeta, m_masters, 0, 552},
                 {"qc", "Preflight QC", m_qcMeta, m_qc, 0, 404},
                 {"loudness", "Loudness", text(QStringLiteral("Programme"), tokens::meta, tokens::ink3), m_loudness, 240, 0},
                 {"paperwork", "Paperwork", text(QStringLiteral("travels with the master"), tokens::meta, tokens::ink3), paperwork, 0, 364}};
    for (const auto &spec : specs) {
        const QString id = QString::fromLatin1(spec.id);
        auto *pane = new TrayPane(QStringLiteral("deliverPane_%1").arg(id), QString::fromLatin1(spec.name), spec.meta, spec.body);
        if (spec.width) pane->setFixedWidth(spec.width);
        panes->addWidget(pane, spec.stretch);
        m_panes.insert(id, pane);
    }
    layout->addLayout(panes, 1);
    if (m_bus) {
        connect(m_bus, &EditorBus::highlightPane, this, &DeliverTray::highlightPane);
        connect(m_bus, &EditorBus::snapshotChanged, this, &DeliverTray::updateBar);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("render_queue")) setQueue(event);
            else if (type == QStringLiteral("comments")) setComments(event);
            else if (type == QStringLiteral("check")) updateBar();
        });
    }
    updateBar();
}

void DeliverTray::setRenderAction(QAction *master) {
    if (m_renderAction) disconnect(m_renderAction, nullptr, this, nullptr);
    m_renderAction = master;
    if (master) connect(master, &QAction::changed, this, &DeliverTray::updateBar);
    updateBar();
}

void DeliverTray::setQueue(const QJsonObject &queue) {
    m_queue = queue;
    while (m_masterRows->count() > 1) {
        QLayoutItem *item = m_masterRows->takeAt(0);
        delete item->widget();
        delete item;
    }
    const QJsonArray jobs = queue.value(QStringLiteral("jobs")).toArray();
    int index = 0;
    for (const QJsonValue &value : jobs) {
        const QJsonObject job = value.toObject();
        auto *row = new QWidget;
        row->setObjectName(QStringLiteral("deliverQueueRow"));
        row->setFixedHeight(52);
        auto *rowLayout = new QHBoxLayout(row);
        rowLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
        rowLayout->setSpacing(tokens::space::s12);
        auto *words = new QVBoxLayout;
        words->setSpacing(0);
        words->addWidget(text(job.value(QStringLiteral("name")).toString(job.value(QStringLiteral("jobId")).toString()),
                              tokens::controlStrong, tokens::ink));
        QStringList spec;
        for (const char *key : {"quality", "codec", "sequence"})
            if (const QString part = job.value(QLatin1String(key)).toString(); !part.isEmpty()) spec << part;
        words->addWidget(text(spec.join(QStringLiteral(" · ")), tokens::meta, tokens::ink3));
        rowLayout->addLayout(words, 1);
        const QString status = job.value(QStringLiteral("status")).toString();
        if (status == QStringLiteral("running")) {
            auto *progress = new QProgressBar;
            progress->setFixedSize(96, 4);
            progress->setTextVisible(false);
            progress->setRange(0, 100);
            progress->setValue(int(std::clamp(job.value(QStringLiteral("progress")).toDouble(), 0.0, 1.0) * 100));
            rowLayout->addWidget(progress);
            rowLayout->addWidget(text(QStringLiteral("%1%").arg(progress->value()), tokens::monoSmall, tokens::ink2));
        } else {
            const bool bad = status == QStringLiteral("failed");
            rowLayout->addWidget(text(bad ? job.value(QStringLiteral("error")).toString(QStringLiteral("Failed")) : status,
                                      tokens::control, bad ? tokens::neg : tokens::ink2));
        }
        m_masterRows->insertWidget(index++, row);
    }
    m_mastersMeta->setText(jobs.isEmpty() ? QStringLiteral("No renders yet")
                           : jobs.size() == 1 ? QStringLiteral("1 output")
                                              : QStringLiteral("%1 outputs").arg(jobs.size()));
}

int DeliverTray::queueRowCount() const {
    return m_masterRows->count() - 1;
}

void DeliverTray::setLoudness(const QJsonObject &loudness) {
    m_loudness->setLoudness(loudness);
}

void DeliverTray::setCheck(const QJsonObject &check) {
    m_qc->setCheck(check);
    updateBar();
}

void DeliverTray::highlightPane(const QString &id) {
    if (!m_panes.contains(id)) return;
    m_highlighted = id;
    for (auto it = m_panes.cbegin(); it != m_panes.cend(); ++it)
        static_cast<TrayPane *>(it.value())->setHighlighted(it.key() == id);
    QTimer::singleShot(1600, this, [this, id] {
        if (m_highlighted != id) return;
        m_highlighted.clear();
        static_cast<TrayPane *>(m_panes.value(id))->setHighlighted(false);
    });
}

QString DeliverTray::blockingText() const {
    return m_blockingLabel->text();
}

void DeliverTray::setComments(const QJsonObject &comments) {
    // Open notes marked blocking, replies or not.
    int blocking = 0;
    for (const QJsonValue &value : comments.value(QStringLiteral("items")).toArray()) {
        const QJsonObject comment = value.toObject();
        if (comment.value(QStringLiteral("blocking")).toBool() && !comment.value(QStringLiteral("resolved")).toBool()) ++blocking;
    }
    m_blocking = blocking;
    updateBar();
}

void DeliverTray::updateBar() {
    if (m_bus) {
        const QString name = m_bus->timeline().value(QStringLiteral("name")).toString();
        QStringList parts;
        if (!name.isEmpty()) parts << name;
        if (m_bus->revision() > 0) parts << QStringLiteral("rev %1").arg(m_bus->revision());
        m_project->setText(parts.join(QStringLiteral(" · ")));
    }
    // Nothing is said of the notes before the first `comments` event.
    m_blockingLabel->setText(m_blocking < 0    ? QString()
                             : m_blocking == 0 ? QStringLiteral("No blocking notes")
                             : m_blocking == 1 ? QStringLiteral("1 blocking note open")
                                               : QStringLiteral("%1 blocking notes open").arg(m_blocking));
    QPalette palette = m_blockingLabel->palette();
    palette.setColor(QPalette::WindowText, tokens::color(m_blocking > 0 ? tokens::neg : tokens::ink3));
    m_blockingLabel->setPalette(palette);
    // Render master is the registry's export.master (enabled with it, its
    // tip the registry's: the keymap's shortcut, why it is unavailable),
    // held back until the notes are known and while one blocks.
    shell::followAction(m_render, m_renderAction,
                        m_blocking < 0   ? QStringLiteral("Waiting for the review notes")
                        : m_blocking > 0 ? QStringLiteral("Resolve blocking notes first")
                                         : QString());
    // Primary once nothing blocks it.
    m_render->setProperty("primary", m_render->isEnabled());
    m_qcMeta->setText(m_qc->summaryText());
}

void DeliverTray::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
