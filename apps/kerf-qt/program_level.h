#pragma once

// The program's level, as the stage rail's meter shows it (mockup-spec 3.5).
//
// ProgramMeter is the model: it is fed only by the bridge's `meter` events
// (plan §3: peak and RMS per channel in dBFS, momentary and short-term
// loudness in LUFS, all null once playback stops) and keeps the peak-hold
// ballistics. With no reading it holds nothing, so the meter shows its empty
// track and never an estimate.
//
// VuMeter, vuNeedle and programPeak are the old analogue meter and the level
// it guessed from the waveform peaks: deprecated, removed in W3 (only
// MainWindow::updateProgramLevel still calls them, and the rail no longer
// shows the VuMeter it hands out).

#include <QElapsedTimer>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QRgb>
#include <QString>
#include <QTimer>
#include <QWidget>

#include <array>

namespace kerf {

class MediaArtwork;

// ---------------------------------------------------------------- ProgramMeter

class ProgramMeter final : public QObject {
    Q_OBJECT
public:
    // The scale runs from 0 dBFS at the top to -60 at the foot.
    static constexpr double kFloorDb = -60.0;
    // The segments' edges: -60...-18 track2, -18...-6 warn, -6...0 neg.
    static constexpr double kWarnDb = -18.0;
    static constexpr double kHotDb = -6.0;
    // A peak is held 1.5 s, then falls at 20 dB a second until it meets the
    // peak playing now (or leaves the scale).
    static constexpr double kHoldSeconds = 1.5;
    static constexpr double kFallDbPerSecond = 20.0;
    static constexpr int kChannels = 2;
    // The design's meter height, which Y() takes by default.
    static constexpr double kHeight = 350.0;

    explicit ProgramMeter(QObject *parent = nullptr);

    // A `meter` event ({peak:[l,r], rms:[l,r], momentary, shortTerm, at});
    // nulls clear the readings they stand for. Returns false for an event
    // of another type.
    bool apply(const QJsonObject &event);
    // Forgets every reading and hold.
    void clear();
    // Ages the holds by `seconds`: what the meter's own timer does while a
    // hold is up. Tests call it directly.
    void advance(double seconds);
    // The timer that ages the holds (on by default); off, only advance()
    // moves them.
    void setAutoAdvance(bool on);
    bool autoAdvance() const { return m_auto; }
    bool ticking() const { return m_tick.isActive(); }

    // A channel's reading in dBFS: the bar shows the RMS level, the hold line
    // the held peak. Each is valid only when has*() says so.
    bool hasLevel(int channel) const;
    double level(int channel) const;
    bool hasPeak(int channel) const;
    double peak(int channel) const;
    bool hasHold(int channel) const;
    double hold(int channel) const;
    // Any channel has a reading.
    bool hasData() const;

    bool hasMomentary() const { return m_hasMomentary; }
    double momentary() const { return m_momentary; }
    bool hasShortTerm() const { return m_hasShortTerm; }
    double shortTerm() const { return m_shortTerm; }
    // The momentary loudness to one decimal with a true minus ("−17.8"), or
    // "—" when there is none.
    QString momentaryText() const;
    // The program time of the latest reading, or -1.
    double at() const { return m_at; }

    // Where `db` sits on a meter `height` tall: -db/60 of it, clamped to the
    // scale and rounded to a whole point (the mockup's Math.round).
    static double y(double db, double height = kHeight);
    // A filled run of the bar, top to bottom in points, and its colour.
    struct Segment {
        double top = 0.0;
        double bottom = 0.0;
        QRgb color = 0;
    };
    // The bar for a level of `db`: up to three runs (track2, warn, neg),
    // bottom first; empty at or under the floor.
    static QList<Segment> segments(double db, double height = kHeight);
    // The scale's labels, in dB under full scale: 0 6 12 18 24 40 60.
    static QList<int> scaleMarks();

signals:
    void changed();

private:
    struct Channel {
        bool hasLevel = false;
        double level = kFloorDb;
        bool hasPeak = false;
        double peak = kFloorDb;
        bool hasHold = false;
        double hold = kFloorDb;
        double holdAge = 0.0; // seconds since the hold was last raised
    };
    void updateTick();
    void step();
    std::array<Channel, kChannels> m_channels;
    bool m_hasMomentary = false;
    double m_momentary = 0.0;
    bool m_hasShortTerm = false;
    double m_shortTerm = 0.0;
    double m_at = -1.0;
    bool m_auto = true;
    QTimer m_tick;
    QElapsedTimer m_clock;
};

// ---------------------------------------------------------------- deprecated

// Deprecated, removed in W3: the level guessed from the waveform peaks under
// the playhead, for the old VU meter. The rail's meter reads `meter` events.
// The program's sound just before `seconds` on `timeline`: the largest peak,
// as a linear sample size, of the sound clips there on enabled, unmuted sound
// tracks, each with its own gain (props.gain, in dB).
double programPeak(const QJsonObject &timeline, const MediaArtwork *artwork, double seconds);

// Deprecated, removed in W3: where a linear peak puts a VU meter's needle, 0
// to 1 (-20 VU to +3 VU, 0 VU at -18 dBFS three quarters of the way across).
double vuNeedle(double peak);

// Deprecated, removed in W3: the old analogue meter. StageRail::meter() still
// hands one out so its old callers compile, but it is never shown; it keeps
// what it is told and its needle stands at its target.
class VuMeter final : public QWidget {
    Q_OBJECT
public:
    static constexpr double kSettleSeconds = 0.3;
    explicit VuMeter(QWidget *parent = nullptr);
    void setTarget(double needle);
    double target() const { return m_target; }
    double needle() const { return m_target; }
    void setPlaying(bool playing) { m_playing = playing; }
    bool animating() const { return false; }
    void advance(double) {}
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;

private:
    double m_target = 0.0;
    bool m_playing = false;
};

} // namespace kerf
