#pragma once

// What the editor is doing, shown where it is done (MOTION_UI.md: Stage at work,
// Making a shot). Kerf at work is blue; the computer making pictures, a render or
// a model making a shot, is amber. Two hues and no more.
//
// The pure parts (what a making's line says, the clock, what the stage says) are
// here beside the two things that draw it: the light round the picture
// (EdgeLight) and the stripes over what is being made (paintStripes).

#include "design_tokens.h"
#include "test_friends.h"

#include <QElapsedTimer>
#include <QHash>
#include <QList>
#include <QRectF>
#include <QRegion>
#include <QString>
#include <QWidget>

class QPainter;
class QTimer;

namespace kerf::activity {

// Which light is on: none, Kerf's, or the computer's.
enum class Hue { None, Kerf, Gpu };
QRgb hueColor(Hue hue);

// What a step of a making last said, as kerf-comfy and the recipe's steps say it:
//   ""                                       Starting
//   "asked the server for 33 frames…: job …" Asked
//   "waiting, 1 ahead, 4 s"                  Waiting (ahead 1, seconds 4)
//   "running, 12 s"                          Running (seconds 12)
//   "rendering bridge: 62%"                  Rendering (percent 62)
//   "made in 13 s"                           Made (seconds 13)
//   anything else                            Other
struct MakingLine {
    enum class Phase { Starting, Asked, Waiting, Running, Rendering, Made, Other };
    Phase phase = Phase::Starting;
    int ahead = -1;
    int seconds = -1;
    int percent = -1;
    // Waiting its turn on the server: queued, not yet making.
    bool queued() const { return phase == Phase::Asked || phase == Phase::Waiting; }
};
MakingLine parseMakingLine(const QString &line);

// A making under way, as the window keeps it.
struct Making {
    int take = 0;
    QString line;
    // When it began, on clock().
    qint64 since = 0;
};

// The time every activity keeps, in ms, so a clip, a tile and the chip agree.
qint64 clock();
// How long it has gone: "0:07", "1:12", "1:02:03".
QString elapsedText(qint64 ms);

// What a media is doing, for its clips and its tile.
struct MediaActivity {
    enum class State { Ready, Making, Queued, NotMade, Offline };
    State state = State::Ready;
    qint64 since = 0; // Making and Queued: when the making began, on clock()
    int take = 0;
    int ahead = -1;   // Queued: how many are ahead of it, when said
    int percent = -1; // Making: its share done, when said
};
// What a clip of the media says: "Making 0:12", "Queued", "Not made",
// "Offline"; empty when it is ready.
QString clipLabel(const MediaActivity &activity, qint64 now);

// What is under way, for the stage to say.
struct Work {
    bool agentBusy = false;
    // Kerf is building a cut it proposes.
    bool cutting = false;
    // Makings, each with its media's name.
    struct Named {
        QString name;
        Making making;
    };
    QList<Named> makings;
    // A render running: "Rendering preview", "Master export", with its share done.
    QString render;
    double fraction = 0.0;
};
struct Stage {
    Hue hue = Hue::None;
    // What the chip on the picture says, empty for none.
    QString chip;
};
// Amber over blue: what the computer makes is the slower thing to wait on.
Stage stageFor(const Work &work, qint64 now);

// Diagonal stripes across `rect`, one each tokens::motion::stripeSpace, moved
// `phase` (0 to 1) of a stripe on.
void paintStripes(QPainter &painter, const QRectF &rect, qreal phase, const QColor &color);
// Where stripes are at `now`; 0 under Reduce Motion.
qreal stripePhase(qint64 now);

// The light round the picture while something works: a head running round its
// edge with a tail, over a quiet ring, just outside the picture. It comes and
// goes over tokens::motion::edgeFade, and under Reduce Motion it is simply
// there, a still ring. It takes no clicks, and while it holds still it draws no
// frames: only where the light was and is is drawn again.
class EdgeLight final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit EdgeLight(QWidget *parent = nullptr);
    // The picture's rect and corners, in this widget.
    void setFrame(const QRectF &picture, qreal radius);
    QRectF frame() const { return m_frame; }
    // None lets it go out.
    void setHue(Hue hue);
    Hue hue() const { return m_hue; }
    // Lit, or still going out.
    bool isLit() const;
    bool isTicking() const;
    // How lit it is now, 0 to 1.
    qreal level() const;

protected:
    void paintEvent(QPaintEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    void tick();
    void follow();
    // The ring's band, in four strips round the picture.
    QRegion band() const;
    // Where the head and its tail are at `turn`, as runs of the ring.
    QList<QPolygonF> comet(qreal turn) const;
    const QList<QPointF> &ring() const;
    QRectF m_frame;
    qreal m_radius = 0;
    Hue m_hue = Hue::None;
    // The hue it shows: the last lit one while it goes out.
    Hue m_shown = Hue::None;
    QElapsedTimer m_clock;
    qint64 m_changed = -1;
    qreal m_from = 0.0;
    qreal m_to = 0.0;
    QTimer *m_ticker = nullptr;
    QRegion m_lit;
    mutable QList<QPointF> m_ring;
    mutable QRectF m_ringFor;
};

} // namespace kerf::activity
