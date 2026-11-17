#pragma once
// Small parts of the conversation: its text helpers (timecodes, lengths,
// ages, Kerf's Markdown as rich text), the fold link, the suggestion row, and
// the drafting dots and skeleton.

#include <QAbstractButton>
#include <QDateTime>
#include <QString>
#include <QStringList>
#include <QWidget>

class QTimer;
class QVariantAnimation;

namespace kerf {

namespace chat {

// "00:00:17:08" at `fps` (whole frames, non-drop).
QString timecode(double seconds, double fps);
// A length as seconds and frames: "2s12f", "1s02f", "12f".
QString lengthText(double seconds, double fps);
// "00:00:17:08 · 2s12f" for a range, "00:00:17:08" for a point.
QString markText(double at, double duration, double fps);
// "+17f", "−3f" (a true minus), "0f".
QString signedFrames(qint64 frames);
// The age a row shows: "now", "9 min", "2 h", "Mon", then "3 Nov".
QString age(const QDateTime &then, const QDateTime &now);
// Its tooltip: "rev 21 · 14:20, 3 Nov" (without the revision when unknown).
QString ageTip(qint64 revision, const QDateTime &created);
// The seconds of "HH:MM:SS:FF" (or ;FF) at `fps`; NaN when it is not one.
double parseTimecode(const QString &text, double fps);
// The text as rich text: escaped, inline timecodes as tc:<seconds> links
// (mono 12, accent ink), line breaks kept.
QString richBody(const QString &body, double fps);
// The text escaped as rich text, a word too long for any line (a file's
// name, a link) free to break anywhere: a QLabel breaks only at spaces.
QString breakingLongWords(const QString &text);
// The text's first line, as a title: without the Markdown marks richBody shows it without.
QString titleLine(const QString &body);

} // namespace chat

// .lk: a fold link ("3 steps", "2 changes"), UI 12/16 (11/16
// in the tags row) ink2, hover ink, with a 14 pt chevron that turns 180°
// in 160 ms when open. Checkable: checked is open.
class ChatExpander final : public QAbstractButton {
    Q_OBJECT
public:
    explicit ChatExpander(QWidget *parent = nullptr);
    // 11 pt in a tags row (16 tall), else 12 pt (20 tall).
    void setSmall(bool small);
    bool isSmall() const { return m_small; }
    // A link with no chevron (Ask again, Copy).
    void setChevron(bool chevron);
    // The chevron's turn, 0 (closed) to 1 (open), as drawn now.
    qreal turn() const { return m_turn; }
    // Open or closed at once, without toggled() (a rebuild restoring it).
    void setOpenQuietly(bool open);
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    bool m_small = false;
    bool m_chevron = true;
    qreal m_turn = 0;
    QVariantAnimation *m_rotate = nullptr;
};

// A request to try, set as the rows of the card over it are: 34 tall,
// radius 6, an 18 pt g-cdr in ink3 12 in, the words UI 15/20 ink (timecodes
// in mono ink2); a raised wash under the pointer.
class SuggestionButton final : public QAbstractButton {
    Q_OBJECT
public:
    explicit SuggestionButton(const QString &text, QWidget *parent = nullptr);
    // What it asks for when chosen (the label by default).
    QString request() const { return m_request; }
    void setRequest(const QString &request) { m_request = request; }
    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return QSize(80, sizeHint().height()); }
    // Where its words go.
    QRect textRect() const;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }

private:
    QString m_request;
};

// The three 3 pt drafting dots (ink3, gap 3), pulsing in turn unless motion
// is reduced.
class DraftDots final : public QWidget {
    Q_OBJECT
public:
    explicit DraftDots(QWidget *parent = nullptr);
    QSize sizeHint() const override { return QSize(2 + 3 * 3 + 2 * 3, 20); }

protected:
    void paintEvent(QPaintEvent *) override;
    void showEvent(QShowEvent *) override;
    void hideEvent(QHideEvent *) override;

private:
    QTimer *m_timer = nullptr;
    int m_phase = 0;
};

// The drafting skeleton: 8 pt lines, radius 4, raised, 8 apart, at 88% and 56%.
class DraftSkeleton final : public QWidget {
    Q_OBJECT
public:
    explicit DraftSkeleton(QWidget *parent = nullptr);
    QSize sizeHint() const override { return QSize(120, 8 + 8 + 8); }

protected:
    void paintEvent(QPaintEvent *) override;
};

} // namespace kerf
