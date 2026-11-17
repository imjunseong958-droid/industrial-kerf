#pragma once

// The messages of one conversation, as the conversation panel lists them in
// the order they were said:
//
//   YouMessage   what the person asked: a bubble at the right, raised, radius
//                8, reading 14/22 ink, at most 82% of the column; the playhead
//                it was asked at as a timecode chip over it (mono 11, accent
//                ink), which seeks.
//   KerfMessage  Kerf's answer: its work log, then what it said (Markdown as
//                rich text, reading 14/22 ink, flush left with no bubble, as a
//                coding agent answers), then the cards of what it proposed,
//                then Ask again when it failed or was stopped.
//   WorkLog      what Kerf is doing or did: live, "Working" with a light
//                passing over it, the time so far and the step it is on, the
//                last steps under it on a hairline; done, "Worked for 1m 12s ·
//                6 steps", folded, unfolding to every step.

#include "test_friends.h"

#include <QElapsedTimer>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QWidget>

class QLabel;
class QTimer;
class QVBoxLayout;
class QHBoxLayout;

namespace kerf {

class ChatExpander;
class DraftDots;
class MakeCard;

// "0:42", "12:05": the time a turn has worked so far.
QString workClock(double seconds);
// "12s", "1m 12s", "1h 4m": how long it worked, once done.
QString workLength(double seconds);

class YouMessage final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kPadX = 12;
    static constexpr int kPadY = 8;
    static constexpr double kShare = 0.82;

    // `timecode` is NaN when it was not asked at a time.
    YouMessage(const QString &text, double timecode, double fps, QWidget *parent = nullptr);
    QString text() const { return m_text; }
    double timecode() const { return m_timecode; }
    QLabel *body() const { return m_body; }
    // The timecode chip; null when it was not asked at a time.
    QWidget *chip() const { return m_chip; }
    QRect bubbleRect() const { return bubbleFor(width()); }

    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return QSize(80, heightForWidth(320)); }

signals:
    void seekRequested(double seconds);

protected:
    void paintEvent(QPaintEvent *) override;
    void resizeEvent(QResizeEvent *) override;

private:
    QRect bubbleFor(int width) const;
    int chipHeight() const;

    QString m_text;
    double m_timecode;
    QLabel *m_body = nullptr;
    QWidget *m_chip = nullptr;
};

class WorkLog final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    static constexpr int kRow = 20;
    // Steps shown under a live log: the latest of them.
    static constexpr int kLiveSteps = 3;

    explicit WorkLog(QWidget *parent = nullptr);

    // Kerf at work since `elapsed` started; the step it is on now.
    void start();
    bool isLive() const { return m_live; }
    void setStatus(const QString &status);
    QString status() const { return m_status; }
    void addStep(const QString &step);
    // A tool call's row, by its id: added as it starts, and its line put in
    // the same place once it ends. A failed one is said in neg.
    void setStep(const QString &id, const QString &step, bool failed = false);
    void setSteps(const QStringList &steps);
    QStringList steps() const { return m_steps; }
    // Done, after `seconds`: folded to its headline.
    void finish(double seconds);
    double seconds() const;
    // "Working · 0:42" while live, "Worked for 1m 12s · 6 steps" once done.
    QString headline() const;
    bool isOpen() const { return m_open; }
    void setOpen(bool open);
    ChatExpander *toggle() const { return m_toggle; }
    // The light over "Working", 0 to 1 across it (for tests).
    qreal shimmer() const { return m_shimmer; }

    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override;
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return QSize(120, kRow); }

protected:
    void paintEvent(QPaintEvent *) override;
    void showEvent(QShowEvent *) override;
    void hideEvent(QHideEvent *) override;

private:
    QStringList shownSteps() const;
    void updateToggle();
    void updateTimer();

    bool m_live = false;
    bool m_open = false;
    QString m_status;
    QStringList m_steps;
    // The tool call each row is, where it is one (parallel to m_steps).
    QStringList m_stepIds;
    QStringList m_failedIds;
    double m_seconds = 0;
    QElapsedTimer m_elapsed;
    qreal m_shimmer = 0;
    QTimer *m_frame = nullptr;
    ChatExpander *m_toggle = nullptr;
};

class KerfMessage final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Outcome { Answer, Failed, Stopped };

    explicit KerfMessage(double fps, QWidget *parent = nullptr);

    WorkLog *work() const { return m_work; }
    // What Kerf said, as Markdown.
    void setText(const QString &text);
    void appendText(const QString &delta);
    QString text() const { return m_text; }
    QLabel *body() const { return m_bodyLabel; }
    // Nothing said yet while Kerf works: the drafting dots.
    void setDrafting(bool drafting);
    bool isDrafting() const;
    void setOutcome(Outcome outcome, const QString &askAgain = {});
    Outcome outcome() const { return m_outcome; }
    // Ask again, for a failed or stopped turn; null otherwise.
    ChatExpander *askAgainButton() const { return m_askAgain; }
    // A card of what it proposed, under what it said. The message takes it.
    void addCard(QWidget *card);
    QList<QWidget *> cards() const;
    void clearCards();
    // The makings it asked the person to run, each a card under what it said.
    void addAsk(MakeCard *card);
    QList<MakeCard *> asks() const;
    // The proposal its turn staged, if any.
    void setProposal(const QString &proposalId) { m_proposal = proposalId; }
    QString proposal() const { return m_proposal; }

signals:
    void askAgain(const QString &text);
    void seekRequested(double seconds);

private:
    void updateBody();

    double m_fps;
    QString m_text;
    QString m_request;
    QString m_proposal;
    Outcome m_outcome = Outcome::Answer;
    QVBoxLayout *m_layout = nullptr;
    WorkLog *m_work = nullptr;
    QLabel *m_state = nullptr;
    QLabel *m_bodyLabel = nullptr;
    DraftDots *m_dots = nullptr;
    QWidget *m_cardBox = nullptr;
    QVBoxLayout *m_cardLayout = nullptr;
    QWidget *m_askBox = nullptr;
    QVBoxLayout *m_askLayout = nullptr;
    ChatExpander *m_askAgain = nullptr;
};

} // namespace kerf
