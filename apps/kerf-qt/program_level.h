#pragma once

#include <QElapsedTimer>
#include <QJsonObject>
#include <QTimer>
#include <QWidget>

namespace kerf {

class MediaArtwork;

// The program's sound just before `seconds` on `timeline` (the snapshot's
// timeline, as the editor holds it): the largest peak, as a linear sample
// size, of the sound clips there on enabled, unmuted sound tracks, each with
// its own gain (props.gain, in dB). Silence, or sound whose peaks have not
// arrived, is 0.
double programPeak(const QJsonObject &timeline, const MediaArtwork *artwork, double seconds);

// Where a linear peak puts a VU meter's needle, 0 (the left stop) to 1:
// -20 VU to +3 VU, 0 VU at -18 dBFS and three quarters of the way across
// where the red begins, spaced by voltage as a VU meter's printed scale is.
double vuNeedle(double peak);

// An analog VU meter's window: the needle follows the level it is given
// with a VU meter's ballistics, reaching a steady reading in about 300 ms
// on the way up and the way down. It animates only while the program plays
// or its needle is still settling.
class VuMeter final : public QWidget {
    Q_OBJECT
public:
    // A VU meter's needle reaches 99% of a step in 300 ms.
    static constexpr double kSettleSeconds = 0.3;
    explicit VuMeter(QWidget *parent = nullptr);
    // The reading the needle travels to, 0 to 1 (see vuNeedle).
    void setTarget(double needle);
    double target() const { return m_target; }
    // Where the needle is now.
    double needle() const { return m_needle; }
    // While playing the meter keeps moving with each new reading.
    void setPlaying(bool playing);
    bool animating() const { return m_tick.isActive(); }
    // Moves the needle `seconds` of the way towards its target.
    void advance(double seconds);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;

private:
    void step();
    void updateTick();
    double m_target = 0.0;
    double m_needle = 0.0;
    bool m_playing = false;
    QTimer m_tick;
    QElapsedTimer m_clock;
};

} // namespace kerf
