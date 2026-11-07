#include "editor_bus.h"

namespace kerf {

EditorBus::EditorBus(QObject *parent) : QObject(parent) {
    setObjectName(QStringLiteral("editorBus"));
}

bool EditorBus::send(const QJsonObject &command) {
    return m_sender && !command.isEmpty() && m_sender(command);
}

bool EditorBus::run(const QString &commandId, const QJsonObject &params) {
    return m_runner && !commandId.isEmpty() && m_runner(commandId, params);
}

void EditorBus::setReview(ReviewModel *review) {
    if (review == m_review) return;
    m_review = review;
    emit reviewChanged();
}

void EditorBus::setViewAs(const QString &person) {
    if (person == m_viewAs) return;
    m_viewAs = person;
    emit viewAsChanged(person);
}

void EditorBus::dispatch(const QJsonObject &event) {
    const QString type = event.value(QStringLiteral("type")).toString();
    if (type.isEmpty()) return;
    m_lastEvents.insert(type, event);
    if (type == QStringLiteral("collab")) {
        const QJsonObject user = event.value(QStringLiteral("status")).toObject().value(QStringLiteral("user")).toObject();
        if (!user.isEmpty() && user != m_identity) {
            m_identity = user;
            emit identityChanged(m_identity);
        }
    }
    emit this->event(type, event);
}

void EditorBus::setSnapshot(const QJsonObject &snapshot, const QJsonObject &timeline, double fps, const QSize &frameSize) {
    m_snapshot = snapshot;
    m_timeline = timeline;
    m_revision = static_cast<quint64>(snapshot.value(QStringLiteral("revision")).toDouble());
    m_fps = fps > 0 ? fps : 24.0;
    m_frameSize = frameSize;
    emit snapshotChanged();
}

void EditorBus::setPlayhead(double seconds) {
    if (seconds == m_playhead) return;
    m_playhead = seconds;
    emit playheadChanged(seconds);
}

void EditorBus::setDuration(double seconds) {
    m_duration = seconds;
}

void EditorBus::setPlaying(bool playing) {
    if (playing == m_playing) return;
    m_playing = playing;
    emit playingChanged(playing);
}

void EditorBus::setSelection(const QStringList &clips) {
    if (clips == m_selection) return;
    m_selection = clips;
    emit selectionChanged(clips);
}

void EditorBus::setWorkspace(const QString &name, bool reapplied) {
    if (name == m_workspace && !reapplied) return;
    m_workspace = name;
    emit workspaceChanged(name);
}

void EditorBus::setProgramFrame(const QImage &frame) {
    m_programFrame = frame;
    emit programFrameChanged(frame);
}

void EditorBus::setProgramSignal(const QImage &frame, int curve) {
    emit programSignalChanged(frame, curve);
}

} // namespace kerf
