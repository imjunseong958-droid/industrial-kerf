#pragma once

// The window's continuity. Things on screen become the next thing rather than
// giving way to it: a thumbnail grows into the picture, a clip's poster flies
// from the conversation to where it lands on the timeline. StageMotion is a
// clear layer over the content that draws those flights; it takes no clicks.
//
// All of it stands aside under reduced motion: a flight then never starts and
// the end of it is simply there.

#include "design_tokens.h"
#include "test_friends.h"

#include <QElapsedTimer>
#include <QImage>
#include <QList>
#include <QPixmap>
#include <QRectF>
#include <QVariantAnimation>
#include <QWidget>

namespace kerf {

class StageMotion final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    struct Flight {
        QImage image;
        QRectF from;
        QRectF to;
        int delay = 0;
        int duration = tokens::motion::rise;
        qreal radius = 6;
        qreal fromOpacity = 1.0;
        qreal toOpacity = 1.0;
        // Overshoots a little as it lands, as a thing thrown does.
        bool lively = true;
        // Stays where it lands until released.
        bool stays = false;
        // Shown whole on black, as the picture shows its footage, not filling its box.
        bool fit = false;
        // Once landed it holds this long and fades, over what it became.
        int tail = 0;
        // The clip's name, on it, and a ring of light as it lands.
        QString label;
        bool ring = false;
    };

    explicit StageMotion(QWidget *parent = nullptr);

    // False under reduced motion, when nothing flies.
    bool fly(const Flight &flight);
    // Those that stay fade where they are; one still on its way lands first.
    void releaseStaying();
    // The picture has moved: what stands in for it goes with it. With no picture
    // there is nothing to stand in for.
    void followPicture(const QRect &picture);
    bool isFlying() const { return !m_flights.isEmpty(); }
    int flightCount() const { return int(m_flights.size()); }
    // Whether it still draws frames: not when all that is left holds still.
    bool isTicking() const { return m_ticker.state() == QAbstractAnimation::Running; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    struct Running {
        Flight flight;
        qint64 started = 0;
        // What it last drew over, and whether it has been drawn at rest.
        QRect drawn;
        bool atRest = false;
    };
    void tick();
    void wake();
    QRectF boxOf(const Running &running) const;
    qreal opacityOf(const Running &running) const;
    QElapsedTimer m_clock;
    // Frames come from the clock all animations share, so a flight and the regions
    // under it move in the same frames. One time for a frame: what is marked to
    // be drawn is what is drawn.
    QVariantAnimation m_ticker;
    qint64 m_now = 0;
    QList<Running> m_flights;
};

struct FootageTile {
    QString id;
    QString label;
    QImage poster;
};

// The footage in the conversation: named tiles that pop in, one after another.
// As Kerf uses one it goes on from its tile to become the picture or a clip; its
// place stays, and the sheet closes when they have all gone.
class FootageSheet final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit FootageSheet(QWidget *parent = nullptr);
    void setTiles(const QList<FootageTile> &tiles);
    QStringList ids() const;
    // Where `id` is here; empty when it is not.
    QRect tileRect(const QString &id) const;
    // It has gone on from its tile.
    void take(const QString &id);
    bool isTaken(const QString &id) const { return m_taken.contains(id); }

    int heightForWidth(int width) const override;
    QSize sizeHint() const override { return QSize(320, heightForWidth(320)); }

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    QList<FootageTile> m_tiles;
    QStringList m_taken;
    QVariantAnimation m_pop;
};

// A chip over the picture: a pulsing dot and what Kerf is doing.
class WorkChip final : public QWidget {
    Q_OBJECT
public:
    explicit WorkChip(QWidget *parent = nullptr);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    QVariantAnimation m_pulse;
    int m_pulseStep = 0;
};

namespace moves {

// Fades `widget` in from nothing, whenever it is first shown here.
void fadeIn(QWidget *widget, int delay = 0, int duration = tokens::motion::appear);
// Moves `widget` up into place as it fades in. It is drawn there, not put there,
// so a layout that moves it meanwhile is not undone.
void rise(QWidget *widget, int delay = 0, int distance = 14, int duration = tokens::motion::rise);
// Eases `widget` to `level` of its opacity, from wherever it has got to; 1 takes
// the effect away again.
void dim(QWidget *widget, qreal level, int duration = tokens::motion::settle);

} // namespace moves

} // namespace kerf
