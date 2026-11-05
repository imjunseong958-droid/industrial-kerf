#pragma once

// Versions of a request, side by side: several agents make their own proposal
// at once. The picture becomes the cards: posters, a little askew, fanning out
// from where it was while the timeline waits under them. The one chosen flies
// back to take its place and the others fall away.
//
//       ╱──────────╲ ┌──────────┐ ╱──────────╲            [×]
//       │ poster   │ │ poster   │ │ poster   │
//       │ A        │ │ B        │ │ C        │
//       │ Working  │ │ 3 changes│ │ Failed   │
//       │ [Watch] [Use this]      …

#include "test_friends.h"

#include <QFrame>
#include <QHash>
#include <QImage>
#include <QList>
#include <QRect>
#include <QString>
#include <QStringList>
#include <QWidget>

#include <functional>

class QLabel;

namespace kerf {

namespace ui {
class Button;
class IconButton;
class ThinProgress;
} // namespace ui

class ComparePoster final : public QWidget {
    Q_OBJECT
public:
    explicit ComparePoster(QWidget *parent = nullptr);
    void setImage(const QImage &image);
    bool hasImage() const { return !m_image.isNull(); }
    // Nothing written on it: it is about to be the picture.
    void setPlain(bool plain);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QImage m_image;
    bool m_plain = false;
};

class CompareView final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class State { Running, Ready, Failed, Stopped };
    struct Version {
        QString key;
        QString proposalId; // empty until it has staged one
        State state = State::Running;
        QString status;     // what it is doing, or why it failed
        QStringList changes;
        QImage poster;
    };

    explicit CompareView(QWidget *parent = nullptr);
    void setVersions(const QList<Version> &versions);
    QStringList keys() const;
    QString statusText(const QString &key) const;
    ui::Button *watchButton(const QString &key) const;
    ui::Button *useButton(const QString &key) const;
    ui::IconButton *closeButton() const { return m_close; }
    // Where the cards fan out from, in this widget's coordinates: the picture.
    void setOrigin(const QRect &origin) { m_origin = origin; }
    // Upright cards for an upright picture.
    void setUpright(bool upright);
    // Where each card rests.
    QRect cardRect(int index, int count) const;
    // The card of `proposalId` flies to `to` and the others fall away; `done`
    // follows, at once under reduced motion.
    void playPick(const QString &proposalId, const QRect &to, std::function<void()> done);

signals:
    void watchRequested(const QString &proposalId);
    void useRequested(const QString &proposalId);
    void closeRequested();

protected:
    void resizeEvent(QResizeEvent *event) override;

private:
    struct Card {
        QFrame *frame = nullptr;
        ComparePoster *poster = nullptr;
        ui::ThinProgress *progress = nullptr;
        QLabel *status = nullptr;
        QLabel *changes = nullptr;
        ui::Button *watch = nullptr;
        ui::Button *use = nullptr;
        QString proposal;
    };
    Card &cardFor(const QString &key, bool *made);
    void placeCards(bool animate);

    ui::IconButton *m_close = nullptr;
    QRect m_origin;
    bool m_upright = false;
    bool m_picking = false;
    QStringList m_keys;
    QHash<QString, Card> m_byKey;
};

} // namespace kerf
