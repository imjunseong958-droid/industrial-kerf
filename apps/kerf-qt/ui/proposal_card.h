#pragma once

// A proposal card of the Comments panel (mockup-spec 4.7): a pending change
// from the state snapshot's proposals, under the comment that carries it.
//
//   [pin 12] Pinned proposal · blocking first        (the first open proposal)
//   Move logo reveal to the snare                     (UI 500 13/20; click opens)
//   [2 changes ⌄]                             [v2 ⌄]  (versions menu)
//   Logo_reveal · V4        → 00:00:26:12      +17f   (open: the changes)
//   B003_C011 · Hue vs hue                           (a colour change on one clip)
//   - … / + …                                         (Script: the diff)
//   [□ Preview] [Script]          [Reject] [Accept]   (open: the footer)
//
// A first cut Kerf made has its picture at the top: each clip's poster, the
// length, and a play mark. Pressing it opens the stage, which grows out of it.
// With a picture the card keeps to its title, beside Reject and Accept under
// the picture, and its versions when it has more than one.
//
// Settled, it collapses to one line: "Accepted in rev 23 · You · now". In
// guest mode the buttons give way to "Waiting for the editor".

#include "test_friends.h"

#include <QAbstractButton>
#include <QHash>
#include <QFrame>
#include <QImage>
#include <QList>
#include <QString>
#include <QStringList>

class QHBoxLayout;
class QTimer;
class QLabel;
class QVBoxLayout;

namespace kerf {

class ChatExpander;

namespace ui {
class Button;
class CheckBox;
class GhostButton;
} // namespace ui

// The version button (.vbtn): 20 tall, radius 4, mono 11 ink3 "v2" and a
// 14 pt chevron; hover hover2 and ink.
class VersionButton final : public QAbstractButton {
    Q_OBJECT
public:
    explicit VersionButton(QWidget *parent = nullptr);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
};

// What a cut looks like, for its card: the poster of each clip from where it
// starts, how long it is, and its shape.
struct CutPicture {
    struct Frame {
        double at = 0.0;
        QImage poster;
    };
    QList<Frame> frames; // in time order
    double duration = 0.0;
    double aspect = 16.0 / 9.0;
    bool isEmpty() const { return frames.isEmpty() || duration <= 0.0; }
};

// A cut's picture on its card. Moving over it runs through the cut's clips;
// pressing it asks for the stage. While the stage shows it, it says so.
//
// While Kerf makes the cut, light runs round it, the picture is a little dim and
// becomes each clip as it is placed, and a line of light runs under it. Done, the
// ring lights once and goes out, the picture lightens and it says Done.
class CardPicture final : public QAbstractButton {
    Q_OBJECT
public:
    explicit CardPicture(QWidget *parent = nullptr);
    void setPicture(const CutPicture &picture);
    // Kerf is making it.
    void setWorking(bool working);
    bool isWorking() const { return m_working; }
    // Kerf finished it `ago` ms ago: it shows that it is done, from there on.
    void setFinished(qint64 ago);
    bool isFinishing() const;
    // The time every card's light keeps, so one made again goes on where the last was.
    static qint64 clock();
    const CutPicture &picture() const { return m_picture; }
    void setOnStage(bool on);
    bool onStage() const { return m_onStage; }
    // Where the picture is drawn, in this widget: what the stage grows out of.
    QRect pictureRect() const;
    // The poster it shows now.
    QImage poster() const;
    bool hasHeightForWidth() const override { return true; }
    int heightForWidth(int width) const override;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override;
    void showEvent(QShowEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    QRect boxFor(int width) const;
    void followTicks();
    void tick();
    // A poster as it is drawn at `size`: filled, cropped, ready to draw again and again.
    QImage readyAt(const QImage &poster, const QSize &size) const;
    CutPicture m_picture;
    bool m_onStage = false;
    bool m_working = false;
    qint64 m_finishedAt = -1;
    // What it showed before the newest clip, and when that began to show.
    QImage m_before;
    qint64 m_turnedAt = -1;
    QTimer *m_ticker = nullptr;
    mutable QHash<qint64, QImage> m_ready;
    // How far through the cut the pointer is, 0 to 1; below 0 when it is not over it.
    qreal m_scrub = -1.0;
};

class ProposalCard final : public QFrame {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    struct Change {
        QString name;   // "Logo_reveal · V4"
        QString target; // "→ 00:00:26:12"
        QString shift;  // "+17f"
    };
    struct VersionItem {
        int version = 1;
        QString proposalId;
        QString label; // "v1 00:00:26:14 · +19 fr"
    };
    // A colour change on one clip, said in a line of its own.
    struct Colour {
        QString clip;   // the clip's id
        QString fx;     // the effect's id
        QString title;  // "B003_C011 · Hue vs hue"
        QString params; // "Hue −9°, Sat −6%"
    };

    explicit ProposalCard(const QString &proposalId, QWidget *parent = nullptr);

    // The proposal (its current id) and the version shown, which Accept,
    // Reject and Preview act on.
    QString proposalId() const { return m_id; }
    QString shownId() const { return m_shown.isEmpty() ? m_id : m_shown; }

    void setPinned(bool pinned, bool blocking);
    bool isPinned() const { return m_pinned; }
    void setTitle(const QString &title);
    void setChanges(const QList<Change> &changes);
    void setVersions(const QList<VersionItem> &versions, const QString &shownId);
    QList<VersionItem> versions() const { return m_versions; }
    void setColour(const Colour &colour);
    void clearColour();
    bool hasColour() const { return !m_colour.clip.isEmpty(); }
    Colour colour() const { return m_colour; }
    void setDiff(const QStringList &lines);
    void setOpen(bool open);
    bool isOpen() const { return m_open; }
    void setScriptShown(bool shown);
    bool scriptShown() const;
    // How long its changes take to watch, in seconds; none hides the button.
    void setWatchLength(double seconds);
    ui::Button *watchButton() const { return m_watch; }
    void setPreview(bool on);
    bool previewOn() const;
    // Accept is primary for the pinned or blocking proposal, else secondary.
    void setPrimary(bool primary);
    void setAcceptTip(const QString &tip);
    // The key Accept's tip names: proposal.accept's, ⌘↩ until told otherwise.
    void setAcceptKey(const QString &key);
    // Guest mode: the buttons give way to "Waiting for the editor".
    void setGuest(bool guest);
    // Accept refused (stale, offline): disabled with the reason as its tip.
    void setAcceptBlocked(const QString &reason);
    void setActionsEnabled(bool enabled);
    // Settled: one line, accent ink for an accept, ink2 for a reject. Empty
    // text opens the card again.
    void setSettled(const QString &text, bool accepted);
    // Small: what to do with it stays when its changes are folded, and watching
    // is the first thing it offers.
    void setCompact(bool compact);
    // The cut's picture; an empty one takes it away.
    void setPicture(const CutPicture &picture);
    // Kerf is making the cut: its picture is at work, and it offers nothing to do yet.
    void setBuilding(bool building);
    bool isBuilding() const { return m_building; }
    CardPicture *picture() const { return m_picture; }
    bool isSettled() const { return m_settled; }

    QWidget *pinHeader() const { return m_pinHeader; }
    QLabel *pinHeaderText() const { return m_pinText; }
    QLabel *titleLabel() const { return m_title; }
    // The title on the actions' line, which a card with a picture has instead.
    QLabel *lineTitle() const { return m_lineTitle; }
    ChatExpander *changesLink() const { return m_changesLink; }
    VersionButton *versionButton() const { return m_version; }
    QWidget *changesBox() const { return m_changes; }
    QList<QWidget *> changeRows() const { return m_changeRows; }
    QWidget *colourBox() const { return m_colourBox; }
    QLabel *colourTitle() const { return m_colourTitle; }
    QLabel *colourParams() const { return m_colourParams; }
    QLabel *diffView() const { return m_diff; }
    QStringList diffLines() const { return m_diffLines; }
    QWidget *footer() const { return m_footer; }
    ui::CheckBox *previewBox() const { return m_preview; }
    ui::GhostButton *scriptButton() const { return m_script; }
    ui::Button *acceptButton() const { return m_accept; }
    ui::Button *rejectButton() const { return m_reject; }
    QLabel *waitChip() const { return m_wait; }
    QLabel *settledLabel() const { return m_settledLabel; }
    // The versions menu's labels, checked first (for tests and menus).
    void showVersionMenu();

signals:
    void openToggled(bool open);
    void scriptToggled(bool shown);
    void previewToggled(bool on);
    void watchRequested();
    // The picture was pressed.
    void pictureRequested();
    void versionChosen(const QString &proposalId);
    void acceptRequested(const QString &proposalId);
    void rejectRequested(const QString &proposalId);

protected:
    void paintEvent(QPaintEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    bool m_compact = false;
    bool m_building = false;
    void updateParts();

    QString m_id;
    QString m_shown;
    bool m_pinned = false;
    bool m_open = false;
    bool m_settled = false;
    bool m_guest = false;
    QList<VersionItem> m_versions;
    Colour m_colour;
    QStringList m_diffLines;

    QVBoxLayout *m_layout = nullptr;
    QWidget *m_pinHeader = nullptr;
    QLabel *m_pinText = nullptr;
    CardPicture *m_picture = nullptr;
    QLabel *m_title = nullptr;
    QWidget *m_second = nullptr;
    ChatExpander *m_changesLink = nullptr;
    VersionButton *m_version = nullptr;
    QWidget *m_changes = nullptr;
    QVBoxLayout *m_changesLayout = nullptr;
    QList<QWidget *> m_changeRows;
    QWidget *m_colourBox = nullptr;
    QLabel *m_colourTitle = nullptr;
    QLabel *m_colourParams = nullptr;
    QLabel *m_diff = nullptr;
    QWidget *m_footer = nullptr;
    QLabel *m_lineTitle = nullptr;
    ui::Button *m_watch = nullptr;
    QWidget *m_watchBox = nullptr;
    bool m_hasWatch = false;
    ui::CheckBox *m_preview = nullptr;
    ui::GhostButton *m_script = nullptr;
    QLabel *m_wait = nullptr;
    ui::Button *m_reject = nullptr;
    ui::Button *m_accept = nullptr;
    QString m_acceptTip = QStringLiteral("Accept");
    QString m_acceptKey;
    bool m_acceptBlocked = false;
    QLabel *m_settledLabel = nullptr;
};

} // namespace kerf
