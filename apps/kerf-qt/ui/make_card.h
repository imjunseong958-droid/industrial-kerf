#pragma once

// A making Kerf asked for, in the conversation: what the recipe would run
// (a guide rendered from the project, then a model's program) and the take
// it would write, with Make and Not now, as a coding agent asks before it
// runs a command. Make runs it as the bin's Make from Recipe… does; from then
// on the card follows the making: what its steps say, Stop, and once its take
// is made, Look at it, which asks Kerf to look at the take.
//
// The card says only what it is told: ChatPanel tells it the media's name,
// whether the take is made already, and how a making of it goes.

#include "making_ask.h"

#include <QFrame>
#include <QString>

class QLabel;

namespace kerf {

namespace ui {
class Button;
class GhostButton;
} // namespace ui

class MakeCard final : public QFrame {
    Q_OBJECT
public:
    using Ask = MakingAsk;
    enum class State { Asked, Making, Made, Stopped, Failed, Declined };

    explicit MakeCard(const Ask &ask, QWidget *parent = nullptr);

    const Ask &ask() const { return m_ask; }
    // The media as the bin names it.
    void setMediaName(const QString &name);
    // How the making goes. `detail` is what its steps last said, or why it
    // failed; `take` the take it made.
    void setState(State state, const QString &detail = {}, int take = 0);
    State state() const { return m_state; }
    // Whether the person can make it now (connected, nothing else making it).
    void setCanMake(bool can);

    // The plan without the line telling a person where else to make it.
    static QString planOf(const QString &text);

    ui::Button *makeButton() const { return m_make; }
    ui::Button *notNowButton() const { return m_notNow; }
    ui::Button *stopButton() const { return m_stop; }
    ui::Button *lookButton() const { return m_look; }
    QLabel *statusLabel() const { return m_status; }
    QLabel *planLabel() const { return m_plan; }

signals:
    void makeRequested(const QString &media, int rolls);
    void stopRequested(const QString &media);
    void lookRequested(const QString &media, int take);
    void declined(const QString &key);

protected:
    void paintEvent(QPaintEvent *) override;

private:
    void refresh();
    QString takesText() const;

    Ask m_ask;
    QString m_name;
    State m_state = State::Asked;
    QString m_detail;
    int m_made = 0;
    bool m_canMake = true;
    QLabel *m_title = nullptr;
    QLabel *m_takes = nullptr;
    QLabel *m_plan = nullptr;
    ui::GhostButton *m_planToggle = nullptr;
    QLabel *m_status = nullptr;
    ui::Button *m_notNow = nullptr;
    ui::Button *m_make = nullptr;
    ui::Button *m_stop = nullptr;
    ui::Button *m_look = nullptr;
};

} // namespace kerf
