#pragma once

#include <QObject>
#include <QString>

#include <array>
#include <cstddef>

namespace kerf {

// The Program picture's player state: the transport itself is the scrub
// strip under the picture (ui/scrub_strip, mockup-spec 3.4), and nothing is
// drawn over the picture. What stays is what the window keeps for it: the
// playhead's time, length and rate, whether it plays, and the play and step
// commands' enabled state and tips (MainWindow::updateTransport). A click on
// the picture plays or pauses while Play is enabled (ViewerWidget emits
// controlTriggered).
class PlayerOverlay final : public QObject {
    Q_OBJECT
public:
    enum class Control { Play, StepBack, StepForward };

    explicit PlayerOverlay(QObject *parent = nullptr) : QObject(parent) {}

    void setTime(double time, double duration, double fps);
    double time() const { return m_time; }
    double duration() const { return m_duration; }
    double fps() const { return m_fps; }
    void setPlaying(bool playing) { m_playing = playing; }
    bool playing() const { return m_playing; }
    void setControlEnabled(Control control, bool enabled) { m_enabled[std::size_t(control)] = enabled; }
    bool controlEnabled(Control control) const { return m_enabled[std::size_t(control)]; }
    void setControlTip(Control control, const QString &tip) { m_tips[std::size_t(control)] = tip; }
    QString controlTip(Control control) const { return m_tips[std::size_t(control)]; }

signals:
    void controlTriggered(Control control);

private:
    double m_time = 0.0;
    double m_duration = 0.0;
    double m_fps = 24.0;
    bool m_playing = false;
    std::array<bool, 3> m_enabled{true, true, true};
    std::array<QString, 3> m_tips;
};

} // namespace kerf
