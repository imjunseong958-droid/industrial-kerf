#include "finish_tray.h"
#include "tray_shell.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "loudness_view.h"
#include "qc_view.h"

#include <QAction>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QPainter>
#include <QPushButton>
#include <QTimer>
#include <QToolButton>
#include <QVBoxLayout>

#include <cmath>

namespace kerf {

namespace {

using shell::TrayPane;
using shell::text;

// A scope's plot, empty until program frames are measured (Q-FINISH).
QWidget *emptyPlot(const QString &id) {
    auto *plot = new QLabel(QStringLiteral("Coming in this release"));
    plot->setObjectName(QStringLiteral("scopePlot_%1").arg(id));
    plot->setAlignment(Qt::AlignCenter);
    plot->setFont(tokens::font(tokens::meta));
    plot->setStyleSheet(QStringLiteral("QLabel { background: %1; border-radius: 4px; color: %2; margin: 0 4px; }")
                            .arg(tokens::color(tokens::well).name(), tokens::color(tokens::ink3).name()));
    return plot;
}

} // namespace

FinishTray::FinishTray(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("finishTray"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *bar = new QWidget;
    bar->setObjectName(QStringLiteral("finishBar"));
    bar->setFixedHeight(tokens::layout::strip);
    auto *barLayout = new QHBoxLayout(bar);
    barLayout->setContentsMargins(tokens::space::panelLeft, 0, tokens::space::panelHeaderRight, 0);
    barLayout->setSpacing(tokens::space::s8);
    barLayout->addWidget(text(QStringLiteral("Scopes"), tokens::controlStrong, tokens::ink));
    m_scopes = text(QString(), tokens::meta, tokens::ink3);
    m_scopes->setObjectName(QStringLiteral("finishScopesMeta"));
    barLayout->addWidget(m_scopes);
    m_ask = new QToolButton;
    m_ask->setObjectName(QStringLiteral("finishAsk"));
    m_ask->setText(QStringLiteral("Ask"));
    m_ask->setAutoRaise(true);
    m_ask->setFont(tokens::font(tokens::control));
    connect(m_ask, &QToolButton::clicked, this, [this] {
        if (m_bus) m_bus->run(QStringLiteral("ai.ask"));
    });
    barLayout->addWidget(m_ask);
    barLayout->addStretch(1);
    m_targets = text(QString(), tokens::meta, tokens::ink3);
    m_targets->setObjectName(QStringLiteral("finishTargets"));
    barLayout->addWidget(m_targets);
    m_runQc = new QPushButton(QStringLiteral("Run QC"));
    m_runQc->setObjectName(QStringLiteral("finishRunQc"));
    m_runQc->setFont(tokens::font(tokens::controlStrong));
    m_runQc->setFixedHeight(tokens::height::h24);
    connect(m_runQc, &QPushButton::clicked, this, [this] {
        if (m_bus) m_bus->run(QStringLiteral("qc.run"));
    });
    barLayout->addWidget(m_runQc);
    layout->addWidget(bar);

    auto *panes = new QHBoxLayout;
    panes->setContentsMargins(tokens::space::s8, tokens::space::s8, tokens::space::s8, tokens::space::s8);
    panes->setSpacing(tokens::space::s8);
    m_loudness = new LoudnessView(nullptr, bus);
    m_qc = new QcView(nullptr, bus);
    m_qcMeta = text(QString(), tokens::meta, tokens::ink3);
    const struct {
        const char *id;
        const char *name;
        const char *meta;
        int width; // 0: shares the rest by stretch
        int stretch;
    } specs[] = {// The scopes' metas (scale, precision) come with their plots (Q-FINISH).
                 {"waveform", "Waveform", nullptr, 0, 352},
                 {"vectorscope", "Vectorscope", nullptr, 240, 0},
                 {"parade", "RGB parade", nullptr, 0, 352},
                 {"loudness", "Loudness", "Programme", 240, 0},
                 {"qc", "QC", nullptr, 0, 368}};
    for (const auto &spec : specs) {
        const QString id = QString::fromLatin1(spec.id);
        QWidget *body = id == QStringLiteral("loudness") ? static_cast<QWidget *>(m_loudness)
                        : id == QStringLiteral("qc")     ? static_cast<QWidget *>(m_qc)
                                                         : emptyPlot(id);
        QLabel *meta = spec.meta ? text(QString::fromUtf8(spec.meta), tokens::meta, tokens::ink3)
                       : id == QStringLiteral("qc") ? m_qcMeta
                                                    : nullptr;
        auto *pane = new TrayPane(QStringLiteral("finishPane_%1").arg(id), QString::fromUtf8(spec.name), meta, body);
        if (spec.width) pane->setFixedWidth(spec.width);
        panes->addWidget(pane, spec.stretch);
        m_panes.insert(id, pane);
    }
    layout->addLayout(panes, 1);
    if (m_bus) {
        connect(m_bus, &EditorBus::highlightPane, this, &FinishTray::highlightPane);
        connect(m_bus, &EditorBus::playheadChanged, this, &FinishTray::updateBar);
        connect(m_bus, &EditorBus::selectionChanged, this, &FinishTray::updateBar);
        connect(m_bus, &EditorBus::snapshotChanged, this, &FinishTray::updateBar);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &) {
            if (type == QStringLiteral("loudness") || type == QStringLiteral("check")) updateBar();
        });
    }
    updateBar();
    followActions();
}

void FinishTray::setActions(QAction *ask, QAction *runQc) {
    for (QAction *old : {m_askAction.data(), m_runQcAction.data()})
        if (old) disconnect(old, nullptr, this, nullptr);
    m_askAction = ask;
    m_runQcAction = runQc;
    for (QAction *action : {ask, runQc})
        if (action) connect(action, &QAction::changed, this, &FinishTray::followActions);
    followActions();
}

void FinishTray::followActions() {
    shell::followAction(m_ask, m_askAction);
    shell::followAction(m_runQc, m_runQcAction);
}

void FinishTray::setLoudness(const QJsonObject &loudness) {
    m_loudness->setLoudness(loudness);
    updateBar();
}

void FinishTray::setCheck(const QJsonObject &check) {
    m_qc->setCheck(check);
    updateBar();
}

void FinishTray::highlightPane(const QString &id) {
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

QString FinishTray::scopesText() const {
    return m_scopes->text();
}

void FinishTray::updateBar() {
    // "Program · 00:00:09:03 · B003_C011": what the scopes measure.
    QStringList parts{QStringLiteral("Program")};
    if (m_bus) {
        parts << timelineTimecode(m_bus->playhead(), m_bus->fps());
        const QString clip = m_bus->selection().value(m_bus->selection().size() - 1);
        for (const QJsonValue &track : m_bus->timeline().value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray())
                if (!clip.isEmpty() && value.toObject().value(QStringLiteral("id")).toString() == clip)
                    parts << value.toObject().value(QStringLiteral("name")).toString(clip);
    }
    m_scopes->setText(parts.join(QStringLiteral(" · ")));
    // The R128 targets the measurement reports, else the standard's.
    const QJsonObject loudness = m_loudness->loudness();
    const auto signedNumber = [](double value) {
        const QString number = QString::number(std::abs(value), 'g', 3);
        return value < 0 ? QStringLiteral("−") + number : number;
    };
    m_targets->setText(QStringLiteral("Loudness EBU R128 · %1 LUFS · %2 dBTP")
                           .arg(signedNumber(loudness.value(QStringLiteral("target")).toDouble(-23)),
                                signedNumber(loudness.value(QStringLiteral("limit")).toDouble(-1))));
    m_qcMeta->setText(m_qc->summaryText());
}

void FinishTray::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), tokens::color(tokens::chrome));
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), tokens::color(tokens::line));
}

} // namespace kerf
