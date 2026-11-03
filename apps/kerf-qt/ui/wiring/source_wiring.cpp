#include "source_wiring.h"

#include "editor_bus.h"
#include "main_window.h"
#include "source_monitor_panel.h"
#include "source_state.h"
#include "timeline_widget.h"
#include "workspace.h"

#include <DockWidget.h>

#include <QDir>
#include <QJsonArray>

#include <algorithm>
#include <limits>

namespace kerf {

SourceWiring::SourceWiring(MainWindow *window, EditorBus *bus) : QObject(nullptr), m_window(window), m_bus(bus) {
    setObjectName(QStringLiteral("sourceWiring"));
    m_state = new SourceState(this);
}

SourceWiring::~SourceWiring() = default;

void SourceWiring::attach() {
    MainWindow *w = m_window;
    for (SourceMonitorPanel *panel : {w->m_sourceMonitor, w->m_stageSource})
        if (panel) connect(panel, &SourceMonitorPanel::revealRequested, this, &SourceWiring::reveal);
    connect(m_state, &SourceState::opened, this, [this](const QString &media) {
        if (!media.isEmpty()) requestIndex();
        updateReveal();
    });
    if (m_bus) {
        connect(m_bus, &EditorBus::event, this, &SourceWiring::onEvent);
        connect(m_bus, &EditorBus::snapshotChanged, this, &SourceWiring::onSnapshot);
    }
}

void SourceWiring::onEvent(const QString &type, const QJsonObject &event) {
    if (type == QStringLiteral("media_index")) {
        m_state->applyIndex(event);
    } else if (type == QStringLiteral("error") && event.value(QStringLiteral("command")).toString() == QStringLiteral("media_index")) {
        m_state->applyIndexError(event);
    } else if (type == QStringLiteral("connection")) {
        // A new connection answers nothing asked of the last one.
        m_askAgain = true;
    }
}

void SourceWiring::onSnapshot() {
    if (m_state->isOpen()) {
        // The open media's entry, and its file when it was replaced.
        for (const QJsonValue &value : m_window->m_project.value(QStringLiteral("sources")).toArray()) {
            const QJsonObject source = value.toObject();
            if (source.value(QStringLiteral("id")).toString() != m_state->mediaId()) continue;
            m_state->refresh(source, QDir(m_window->m_projectDirectory).absoluteFilePath(source.value(QStringLiteral("path")).toString()));
            break;
        }
        const SourceState::IndexStatus status = m_state->indexStatus();
        if (m_askAgain || status == SourceState::IndexStatus::None || status == SourceState::IndexStatus::Unavailable)
            requestIndex(true);
    }
    updateReveal();
}

void SourceWiring::requestIndex(bool again) {
    if (!m_state->isOpen()) return;
    const QString media = m_state->mediaId();
    if (m_state->hasIndex(media)) return;
    if (!again && m_state->indexStatus() == SourceState::IndexStatus::Pending) return;
    // Only a live bridge is asked: without one the window would report the
    // runtime missing for a request nobody made by hand.
    if (!m_window->m_bridge || !m_window->m_connected || !m_bus) {
        m_state->setIndexUnavailable();
        return;
    }
    if (m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("media_index")}, {QStringLiteral("media"), media}})) {
        m_askAgain = false;
        m_state->setIndexPending();
    } else {
        m_state->setIndexUnavailable();
    }
}

SourceWiring::Target SourceWiring::revealTarget() const {
    Target target;
    if (!m_state->isOpen()) return target;
    const QString media = m_state->mediaId();
    const double position = std::max(0.0, m_state->position());
    double firstStart = std::numeric_limits<double>::max();
    for (const QJsonValue &trackValue : m_window->m_project.value(QStringLiteral("tracks")).toArray()) {
        for (const QJsonValue &clipValue : trackValue.toObject().value(QStringLiteral("clips")).toArray()) {
            const QJsonObject clip = clipValue.toObject();
            if (clip.value(QStringLiteral("sourceId")).toString() != media) continue;
            const QString id = clip.value(QStringLiteral("id")).toString();
            const double start = clip.value(QStringLiteral("timelineStart")).toDouble();
            const double duration = clip.value(QStringLiteral("duration")).toDouble();
            const double speed = std::max(1e-6, std::abs(clip.value(QStringLiteral("speed")).toDouble(1.0)));
            const double in = clip.value(QStringLiteral("in")).toDouble();
            const double out = in + duration * speed;
            if (position >= in && position < out) {
                // The clip shows the frame on screen: go to that frame of it.
                target.clip = id;
                target.at = start + (position - in) / speed;
                return target;
            }
            if (start < firstStart) {
                firstStart = start;
                target.clip = id;
                target.at = start;
            }
        }
    }
    return target;
}

bool SourceWiring::reveal() {
    const Target target = revealTarget();
    if (target.clip.isEmpty()) return false;
    MainWindow *w = m_window;
    if (ads::CDockWidget *timeline = w->m_workspace ? w->m_workspace->panel(QStringLiteral("timeline")) : nullptr)
        if (timeline->isClosed()) timeline->toggleView(true);
    w->selectClip(target.clip);
    w->queueSeek(target.at);
    if (w->m_timeline) w->m_timeline->setFocus(Qt::OtherFocusReason);
    return true;
}

void SourceWiring::updateReveal() {
    m_state->setRevealClip(revealTarget().clip);
}

} // namespace kerf
