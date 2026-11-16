#include "stage_wiring.h"

#include "agent_hud.h"
#include "command_registry.h"
#include "components.h"
#include "editor_bus.h"
#include "main_window.h"
#include "proposal_model.h"
#include "timeline_widget.h"
#include "viewer_widget.h"

#include <QAbstractButton>
#include <QAction>
#include <QJsonArray>
#include <QHBoxLayout>
#include <QKeySequence>
#include <QLabel>
#include <QMenu>
#include <QPainter>
#include <QSettings>
#include <QSignalBlocker>

#include <cmath>

namespace kerf {

namespace {


// Half a frame either side, so a time on a frame boundary counts as in.
double frameSlack(double fps) { return 0.5 / std::max(1.0, fps); }

} // namespace

StageWiring::StageWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("stageWiring"));
}

StageWiring::~StageWiring() = default;

// ---------------------------------------------------------------- PreviewChip

PreviewChip::PreviewChip(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("previewChip"));
    setFixedHeight(24);
    auto *layout = new QHBoxLayout(this);
    // The chip's padding of 8, then .ptxt's own 8 before its 6 pt dot.
    layout->setContentsMargins(16, 0, 4, 0);
    layout->setSpacing(8);
    auto *dot = new ui::Dot(tokens::color(tokens::ink2));
    dot->setObjectName(QStringLiteral("previewChipDot"));
    layout->addWidget(dot, 0, Qt::AlignVCenter);
    m_text = new QLabel;
    m_text->setObjectName(QStringLiteral("previewChipText"));
    m_text->setFont(tokens::font(tokens::control));
    m_text->setStyleSheet(QStringLiteral("QLabel { color: %1; background: transparent; }").arg(tokens::color(tokens::ink3).name()));
    layout->addWidget(m_text);
    auto *exit = new ui::GhostButton(QStringLiteral("Exit preview"));
    exit->setObjectName(QStringLiteral("exitPreview"));
    exit->setTip(QStringLiteral("Exit preview"));
    exit->setControlHeight(20);
    connect(exit, &QAbstractButton::clicked, this, [this] {
        if (m_bus) m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("preview_exit")}});
    });
    m_exit = exit;
    layout->addWidget(exit);
    hide();
    if (m_bus)
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &event) {
            if (type == QStringLiteral("preview")) follow(event);
            else if (type == QStringLiteral("viewer_reset")) follow(QJsonObject{{QStringLiteral("status"), QStringLiteral("exited")}});
        });
}

QString PreviewChip::text() const { return m_text->text(); }

QAbstractButton *PreviewChip::exitButton() const { return m_exit; }

void PreviewChip::follow(const QJsonObject &event) {
    const QString status = event.value(QStringLiteral("status")).toString();
    const QString proposal = event.value(QStringLiteral("proposalId")).toString();
    const bool on = !proposal.isEmpty() &&
                    (status == QStringLiteral("running") || status == QStringLiteral("ready") || status == QStringLiteral("live"));
    if (!on) {
        // Stopped, exited, failed, or the program's own preview.
        m_text->clear();
        setVisible(false);
        return;
    }
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
    m_text->setText(version > 0 ? QStringLiteral("Previewing proposal v%1").arg(version) : QStringLiteral("Previewing proposal"));
    adjustSize();
    setVisible(!m_suppressed);
    raise();
}

void PreviewChip::setSuppressed(bool suppressed) {
    m_suppressed = suppressed;
    setVisible(!suppressed && !m_text->text().isEmpty());
}

void PreviewChip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QRectF area(rect());
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(28, 28, 28, tokens::alpha(0.94)));
    p.drawRoundedRect(area, tokens::radius::control, tokens::radius::control);
    ui::paintInsetLine(p, area, tokens::radius::control, tokens::color(tokens::line2));
}

// ---------------------------------------------------------------- StageWiring

void StageWiring::build() {
    ViewerWidget *viewer = m_window->m_viewer;
    m_hud = new AgentHud(nullptr, m_bus);
    m_hud->setObjectName(QStringLiteral("agentHud"));
    m_chip = new PreviewChip(nullptr, m_bus);
    viewer->setOverlays(m_hud, m_chip);
}

void StageWiring::attach() {
    if (m_attached) return;
    m_attached = true;
    MainWindow *w = m_window;
    ViewerWidget *viewer = w->m_viewer;

    // The switches: the zoom, the marks and the loop.
    if (w->m_commands) {
        connect(w->m_commands, &CommandRegistry::toggled, this, [this](const QString &id, bool checked) {
            if (id == QStringLiteral("playback.loop")) setLoop(checked);
        });
        w->m_commandHandlers.insert(QStringLiteral("viewer.zoom"), [this] { showZoomMenu(); });
        w->m_commandHandlers.insert(QStringLiteral("mark.in"), [this] { markIn(); });
        w->m_commandHandlers.insert(QStringLiteral("mark.out"), [this] { markOut(); });
        w->m_commandHandlers.insert(QStringLiteral("mark.clear"), [this] { clearMarks(); });
    }

    // The engine and the other regions.
    connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &json) {
        if (type == QStringLiteral("check")) {
            if (json.value(QStringLiteral("status")).toString() == QStringLiteral("done")) m_lastCheck = json;
            refreshOverlap();
        }
    });
    connect(m_bus, &EditorBus::playheadChanged, this, &StageWiring::refreshOverlap);
    connect(m_bus, &EditorBus::playheadChanged, this, &StageWiring::followPlayhead);
    connect(m_bus, &EditorBus::playingChanged, this, &StageWiring::followPlaying);

    if (const QJsonObject check = m_bus->lastEvent(QStringLiteral("check"));
        check.value(QStringLiteral("status")).toString() == QStringLiteral("done"))
        m_lastCheck = check;
    refreshOverlap();
}

QString StageWiring::zoomText(double zoom) {
    if (zoom <= 0.0) return QStringLiteral("Fit");
    return QStringLiteral("%1%").arg(std::lround(zoom * 100.0));
}

void StageWiring::setZoom(double zoom) {
    m_window->m_viewer->setZoom(zoom);
    for (const auto &[value, action] : std::as_const(m_zoomActions)) {
        const QSignalBlocker blocker(action);
        action->setChecked(value == zoom);
    }
}

double StageWiring::zoom() const { return m_window->m_viewer->zoom(); }

QMenu *StageWiring::zoomMenu() {
    if (m_zoomMenu) return m_zoomMenu;
    auto *menu = new QMenu(m_window);
    menu->setObjectName(QStringLiteral("viewerZoomMenu"));
    menu->setTitle(QStringLiteral("Viewer zoom"));
    ui::styleMenu(menu);
    const QAction *zoomCommand = m_window->m_commands ? m_window->m_commands->action(QStringLiteral("viewer.zoom")) : nullptr;
    for (const double value : {0.0, 0.5, 1.0, 2.0}) {
        QAction *action = menu->addAction(zoomText(value));
        action->setObjectName(QStringLiteral("viewerZoom.%1").arg(zoomText(value)));
        action->setCheckable(true);
        action->setChecked(value == zoom());
        if (value == 0.0 && zoomCommand) ui::setMenuKey(action, zoomCommand->shortcut().toString(QKeySequence::NativeText));
        connect(action, &QAction::triggered, this, [this, value] { setZoom(value); });
        m_zoomActions.append({value, action});
    }
    // Each time it opens: the zoom checked.
    connect(menu, &QMenu::aboutToShow, this, [this] {
        for (const auto &[value, action] : std::as_const(m_zoomActions)) action->setChecked(value == zoom());
    });
    m_zoomMenu = menu;
    return menu;
}

void StageWiring::showZoomMenu(QWidget *anchor) {
    if (!anchor) anchor = m_window->findChild<QWidget *>(QStringLiteral("appMenu"));
    QMenu *menu = zoomMenu();
    if (anchor) ui::popupMenu(menu, anchor);
    else menu->popup(m_window->mapToGlobal(m_window->rect().center()));
}

// ---- marks and loop ------------------------------------------------------------------

void StageWiring::showMarks() {
    if (m_window->m_timeline) m_window->m_timeline->setMarkedRange(m_in, m_out);
}

void StageWiring::markIn() {
    if (m_bus->duration() <= 0.0) return;
    m_in = std::clamp(m_bus->playhead(), 0.0, m_bus->duration());
    if (hasOut() && m_out <= m_in) m_out = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    showMarks();
}

void StageWiring::markOut() {
    if (m_bus->duration() <= 0.0) return;
    m_out = std::clamp(m_bus->playhead(), 0.0, m_bus->duration());
    if (hasIn() && m_in >= m_out) m_in = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    showMarks();
}

void StageWiring::clearMarks() {
    if (!hasIn() && !hasOut()) return;
    m_in = m_out = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    showMarks();
}

void StageWiring::setLoop(bool loop) {
    if (loop == m_loop) return;
    m_loop = loop;
    m_wrapped = false;
}

double StageWiring::loopStart() const { return hasIn() ? m_in : 0.0; }

double StageWiring::loopEnd() const { return hasOut() ? std::min(m_out, m_bus->duration()) : m_bus->duration(); }

void StageWiring::followPlayhead() {
    if (!m_loop || !m_bus->playing()) return;
    const double frame = 1.0 / std::max(1.0, m_bus->fps());
    const double end = loopEnd(), start = loopStart();
    if (m_bus->playhead() < end - frame / 2) {
        m_wrapped = false;
        return;
    }
    // Playing on past the out mark: back to the in mark, still playing. At
    // the program's own end playback stops, and followPlaying starts it again.
    const bool atProgramEnd = end >= m_bus->duration() - frame;
    if (m_wrapped || atProgramEnd || end - start < frame) return;
    m_wrapped = true;
    m_window->queueSeek(start);
}

void StageWiring::followPlaying(bool playing) {
    if (playing || !m_loop) return;
    const double frame = 1.0 / std::max(1.0, m_bus->fps());
    const double end = loopEnd(), start = loopStart();
    // Stopped at the loop's end (the program's end, where playback ends by
    // itself, or an out mark the playhead had reached): start again.
    const double slack = std::max(2.0 * frame, 0.1);
    if (end - start < frame || m_bus->playhead() < end - slack) return;
    m_wrapped = true;
    m_window->queueSeek(start);
    m_bus->run(QStringLiteral("playback.toggle"));
}

void StageWiring::refreshOverlap() {
    ViewerWidget *viewer = m_window->m_viewer;
    const double playhead = m_bus->playhead();
    const double slack = frameSlack(m_bus->fps());
    QJsonObject found;
    // The last finished check's findings (one running keeps them).
    const QJsonArray findings = m_lastCheck.value(QStringLiteral("findings")).toArray();
    // The first whose frames the playhead is on.
    for (const QJsonValue &value : std::as_const(findings)) {
            const QJsonObject finding = value.toObject();
            if (finding.value(QStringLiteral("code")).toString() != QStringLiteral("text.overlap")) continue;
            const QJsonObject rect = finding.value(QStringLiteral("rect")).toObject();
            if (rect.value(QStringLiteral("w")).toDouble() <= 0.0 || rect.value(QStringLiteral("h")).toDouble() <= 0.0) continue;
            // The box is about the frames it was found on.
            if (finding.contains(QStringLiteral("at"))) {
                const double at = finding.value(QStringLiteral("at")).toDouble();
                const double end = at + std::max(0.0, finding.value(QStringLiteral("duration")).toDouble());
                if (playhead < at - slack || playhead > end + slack) continue;
            }
            found = finding;
            break;
        }
    m_overlapFinding = found;
    if (found.isEmpty()) {
        viewer->setOverlap(QRectF(), QString());
        return;
    }
    const QJsonObject rect = found.value(QStringLiteral("rect")).toObject();
    QString tag;
    if (found.contains(QStringLiteral("value"))) {
        const QJsonValue value = found.value(QStringLiteral("value"));
        const QString number = value.isDouble() ? QString::number(value.toDouble(), 'g', 4) : value.toString();
        const QString unit = found.value(QStringLiteral("unit")).toString();
        tag = unit.isEmpty() ? number : QStringLiteral("%1 %2").arg(number, unit);
    }
    viewer->setOverlap(QRectF(rect.value(QStringLiteral("x")).toDouble(), rect.value(QStringLiteral("y")).toDouble(),
                              rect.value(QStringLiteral("w")).toDouble(), rect.value(QStringLiteral("h")).toDouble()),
                       tag);
}

} // namespace kerf
