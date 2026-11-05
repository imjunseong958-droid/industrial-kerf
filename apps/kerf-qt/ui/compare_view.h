#pragma once

// Versions of a request, side by side: several agents make their own proposal
// at once, and this lies over the timeline while they do, so the picture stays
// free to play any of them.
//
//   Versions                                            [×]
//   ┌──────────┐ ┌──────────┐ ┌──────────┐
//   │ poster   │ │ poster   │ │ poster   │
//   │ A        │ │ B        │ │ C        │
//   │ Working  │ │ 3 changes│ │ Failed   │
//   │ [Watch] [Use this]      …

#include "test_friends.h"

#include <QFrame>
#include <QHash>
#include <QImage>
#include <QList>
#include <QString>
#include <QStringList>
#include <QWidget>

class QHBoxLayout;
class QLabel;

namespace kerf {

namespace ui {
class Button;
class IconButton;
} // namespace ui

class ComparePoster final : public QWidget {
    Q_OBJECT
public:
    explicit ComparePoster(QWidget *parent = nullptr);
    void setImage(const QImage &image);
    bool hasImage() const { return !m_image.isNull(); }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QImage m_image;
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

signals:
    void watchRequested(const QString &proposalId);
    void useRequested(const QString &proposalId);
    void closeRequested();

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    struct Card {
        QFrame *frame = nullptr;
        ComparePoster *poster = nullptr;
        QLabel *status = nullptr;
        QLabel *changes = nullptr;
        ui::Button *watch = nullptr;
        ui::Button *use = nullptr;
        QString proposal;
    };
    Card &cardFor(const QString &key);

    QHBoxLayout *m_cards = nullptr;
    ui::IconButton *m_close = nullptr;
    QStringList m_keys;
    QHash<QString, Card> m_byKey;
};

} // namespace kerf
