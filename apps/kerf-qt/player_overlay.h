#pragma once

#include "test_friends.h"

#include <QFont>
#include <QObject>
#include <QPointF>
#include <QRectF>
#include <QString>
#include <QTimer>

#include <array>

class QPainter;
class QVariantAnimation;
class QWidget;

namespace kerf {

// What the Program picture keeps of a player: skimming. The transport moved
// off the picture into the scrub strip under it (ui/scrub_strip, mockup-spec
// 3.4), so this draws nothing over the picture and never takes the pointer
// from the viewer, which skims across the picture itself (see ViewerWidget).
//
// It still carries the transport's state for the window: the playhead's time
// and length (timecodeText), whether it plays, and the play and step
// commands' enabled state and tips (updateTransport keeps them, and a click
// on the picture plays only while Play is enabled). The geometry functions
// describe the picture's foot, where nothing is drawn any more. The drawing
// and pointer parts are deprecated, removed in W3.
class PlayerOverlay final : public QObject {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Control { Play, StepBack, StepForward };
    static constexpr int kRowHeight = 44;      // The control row.
    static constexpr int kHitSize = 40;        // Each control's target...
    static constexpr int kGlyphSize = 24;      // ...and its symbol.
    static constexpr int kCompactHitSize = 36; // On a portrait card's cluster.
    static constexpr int kBarHeight = 3;       // The bar, at rest...
    static constexpr int kBarHoverHeight = 5;  // ...and under the pointer...
    static constexpr int kScrubber = 12;       // ...with its scrubber.
    static constexpr int kBarReach = 8;        // How near the bar the pointer takes it.
    static constexpr int kIdleTimeout = 2500;
    static constexpr int kFadeDuration = 200;

    explicit PlayerOverlay(QWidget *host);

    // The picture it lies on, and whether to use the portrait arrangement.
    void setRect(const QRectF &picture, bool compact);
    QRectF rect() const { return m_rect; }
    bool compact() const { return m_compact; }
    void setTime(double time, double duration, double fps);
    void setPlaying(bool playing);
    bool playing() const { return m_playing; }
    void setControlEnabled(Control control, bool enabled);
    bool controlEnabled(Control control) const { return m_enabled[std::size_t(control)]; }
    void setControlTip(Control control, const QString &tip);
    QString controlTip(Control control) const { return m_tips[std::size_t(control)]; }
    // For tests: how long it waits after the pointer's last move, and fades.
    void setIdleTimeout(int milliseconds) { m_idle.setInterval(milliseconds); }
    void setFadeDuration(int milliseconds) { m_fadeDuration = milliseconds; }

    // Whether it is showing or on its way to, and how opaque it is now.
    bool shown() const { return m_target; }
    qreal opacity() const { return m_opacity; }

    QRectF controlRect(Control control) const;
    // The bar as drawn, and the band over it the pointer takes.
    QRectF barRect() const;
    QRectF barReach() const;
    QRectF timecodeRect() const;
    QString timecodeText() const;
    // Every control, the bar and the time: what must stay on the picture.
    QRectF controlsBounds() const;
    // The top of what it shows along the picture's foot (the bar, the time,
    // and the control row where it is there), with the bar at its hovered
    // height, so a caption can keep clear of it.
    double footTop() const;
    // While suppressed (safe zones are shown), only the bar shows until the
    // pointer comes over the picture.
    void setControlsSuppressed(bool suppressed);
    bool controlsSuppressed() const { return m_suppressed; }
    // Whether the controls and the time show now, not only the bar.
    bool controlsShown() const { return !m_suppressed || m_inside; }

    void paint(QPainter &painter) const;
    // The viewer's pointer, in its coordinates; true when this took it.
    bool mousePress(const QPointF &position);
    bool mouseMove(const QPointF &position, bool leftDown);
    bool mouseRelease(const QPointF &position);
    void leave();
    QString toolTipAt(const QPointF &position) const;

signals:
    void controlTriggered(Control control);
    // A click or drag on the bar, as a fraction of the length.
    void seekRequested(double fraction);

private:
    void updateShown();
    void fadeTo(bool shown);
    double fractionAt(double x) const;
    QFont timeFont(int pixelSize) const;
    int timeSize() const { return m_compact ? 11 : 12; }

    QWidget *m_host = nullptr;
    QRectF m_rect;
    bool m_compact = false;
    double m_time = 0.0;
    double m_duration = 0.0;
    double m_fps = 24.0;
    bool m_playing = false;
    std::array<bool, 3> m_enabled{true, true, true};
    std::array<QString, 3> m_tips;
    // Where the pointer is over the picture, and whether it drags the bar.
    bool m_inside = false;
    bool m_suppressed = false;
    bool m_overBar = false;
    bool m_scrubbing = false;
    bool m_pressedControl = false;
    QPointF m_pointer;
    QTimer m_idle;
    bool m_idleExpired = false;
    bool m_target = true;
    qreal m_opacity = 1.0;
    int m_fadeDuration = kFadeDuration;
    QVariantAnimation *m_fade = nullptr;
};

} // namespace kerf
