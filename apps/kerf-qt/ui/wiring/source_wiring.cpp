#include "source_wiring.h"

#include "command_registry.h"
#include "editor_bus.h"
#include "main_window.h"
#include "source_monitor_panel.h"
#include "source_state.h"
#include "timeline_widget.h"


#include <QAction>
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
    if (w->m_sourceMonitor) connect(w->m_sourceMonitor, &SourceMonitorPanel::revealRequested, this, &SourceWiring::reveal);
    connect(m_state, &SourceState::opened, this, &SourceWiring::updateReveal);
    if (m_bus) connect(m_bus, &EditorBus::snapshotChanged, this, &SourceWiring::onSnapshot);
}

void SourceWiring::attachCommands() {
    MainWindow *w = m_window;
    if (!w->m_commands) return;
    w->m_commandHandlers.insert(QStringLiteral("source.play"), [this] { togglePlay(); });
    w->m_commandHandlers.insert(QStringLiteral("source.reveal"), [this] { reveal(); });
    QHash<QString, QAction *> actions;
    for (const char *id : {"source.play", "source.reveal"}) {
        QAction *action = w->m_commands->action(QString::fromLatin1(id));
        if (!action) continue;
        // A key given to them acts while a Source viewer has the focus, as
        // the registry's source-scoped commands do.
        action->setShortcutContext(Qt::WidgetWithChildrenShortcut);
        actions.insert(QString::fromLatin1(id), action);
    }
    if (!actions.isEmpty()) m_state->addActions(actions);
}

bool SourceWiring::togglePlay() {
    if (!m_state->canPlay()) return false;
    m_state->togglePlay();
    return true;
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
    }
    updateReveal();
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
    w->selectClip(target.clip);
    w->queueSeek(target.at);
    if (w->m_timeline) w->m_timeline->setFocus(Qt::OtherFocusReason);
    return true;
}

void SourceWiring::updateReveal() {
    m_state->setRevealClip(revealTarget().clip);
}

} // namespace kerf
