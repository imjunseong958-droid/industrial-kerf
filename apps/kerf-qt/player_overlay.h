#pragma once

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

// The player's controls laid on the picture, as a video site's player lays
// them: a gradient to black along the picture's foot, the progress bar
// across it (thin, thicker under the pointer, its played part in the
// accent, a round scrubber and the time under the pointer while hovered),
// and one row of white controls: play or pause, a frame back, a frame on,
// and the time over the length. On a portrait card it takes a short-video
// player's arrangement instead: the bar along the very foot, the controls
// in a compact cluster at the top left, the time small over the bar.
//
// Shown while paused, while the pointer is over the picture, and for a
// moment after its last move while playing; then it fades. The keyboard
// never depends on it. It draws and takes the pointer for the viewer that
// owns it; the transport itself (playing, stepping, seeking) stays with the
// editor, which hears what is asked through its signals. Swapping this for
// a strip of buttons would leave the transport as it is.
class PlayerOverlay final : public QObject {
    Q_OBJECT
    friend class EditorTest;
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
    void paintGlyph(QPainter &painter, Control control, const QRectF &area) const;

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
