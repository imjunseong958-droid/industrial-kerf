#pragma once

// A proposal card of the Comments panel (mockup-spec 4.7): a pending change
// from the state snapshot's proposals, under the comment that carries it.
//
//   [pin 12] Pinned proposal · blocking first        (the first open proposal)
//   Move logo reveal to the snare                     (UI 500 13/20; click opens)
//   [2 changes ⌄]                             [v2 ⌄]  (versions menu)
//   Logo_reveal · V4        → 00:00:26:12      +17f   (open: the changes)
//   [72 vectorscope] B003_C011 · Hue vs hue          (a colour change on one clip)
//   - … / + …                                         (Script: the diff)
//   [□ Preview] [Script]          [Reject] [Accept]   (open: the footer)
//
// Settled, it collapses to one line: "Accepted in rev 23 · You · now". In
// guest mode the buttons give way to "Waiting for the editor".

#include "scope_math.h"
#include "test_friends.h"

#include <QAbstractButton>
#include <QFrame>
#include <QImage>
#include <QList>
#include <QString>
#include <QStringList>

class QHBoxLayout;
class QLabel;
class QVBoxLayout;

namespace kerf {

class ChatExpander;

namespace ui {
class Button;
class CheckBox;
class GhostButton;
} // namespace ui

// The 72 pt vectorscope chip (.vs): the well, the r 32 graticule in track2
// with the six 4 x 4 75% targets, the proposal's before frame as a grey
// cloud (#828282 at 50%) and its after frame in ink, dots r 0.8.
class VectorscopeChip final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kSize = 72;
    static constexpr double kRadius = 32;
    explicit VectorscopeChip(QWidget *parent = nullptr);
    // Either may be null (not rendered yet).
    void setFrames(const QImage &before, const QImage &after);
    bool hasBefore() const { return !m_before.isEmpty(); }
    bool hasAfter() const { return !m_after.isEmpty(); }
    const scope::Density &before() const { return m_before; }
    const scope::Density &after() const { return m_after; }
    // The graticule the chip bins with: centre (36, 36), radius 32.
    static scope::VectorscopeGeometry geometry();
    // A frame binned as the chip draws it.
    static scope::Density bin(const QImage &frame);
    QSize sizeHint() const override { return QSize(kSize, kSize); }

protected:
    void paintEvent(QPaintEvent *) override;

private:
    scope::Density m_before;
    scope::Density m_after;
};

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
    // A colour change on one clip, previewed in the chip.
    struct Colour {
        QString clip;   // the clip's id, for effect_previews
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
    void setColourFrames(const QImage &before, const QImage &after);
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
    bool isSettled() const { return m_settled; }

    QWidget *pinHeader() const { return m_pinHeader; }
    QLabel *pinHeaderText() const { return m_pinText; }
    QLabel *titleLabel() const { return m_title; }
    ChatExpander *changesLink() const { return m_changesLink; }
    VersionButton *versionButton() const { return m_version; }
    QWidget *changesBox() const { return m_changes; }
    QList<QWidget *> changeRows() const { return m_changeRows; }
    VectorscopeChip *chip() const { return m_chip; }
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
    void versionChosen(const QString &proposalId);
    void acceptRequested(const QString &proposalId);
    void rejectRequested(const QString &proposalId);

protected:
    void paintEvent(QPaintEvent *) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    bool m_compact = false;
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
    QLabel *m_title = nullptr;
    QWidget *m_second = nullptr;
    ChatExpander *m_changesLink = nullptr;
    VersionButton *m_version = nullptr;
    QWidget *m_changes = nullptr;
    QVBoxLayout *m_changesLayout = nullptr;
    QList<QWidget *> m_changeRows;
    QWidget *m_colourBox = nullptr;
    VectorscopeChip *m_chip = nullptr;
    QLabel *m_colourTitle = nullptr;
    QLabel *m_colourParams = nullptr;
    QLabel *m_colourLegend = nullptr;
    QLabel *m_diff = nullptr;
    QWidget *m_footer = nullptr;
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
