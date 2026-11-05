#pragma once

// The window's content. It starts as a conversation: the folders and
// conversations at the left, the conversation beside them. The picture and the
// timeline are the panel at the right, opened when there is something to see
// and closed at will. Everything else opens on demand: the three sheets over
// the conversation, one at a time, and the Finish and Deliver trays over the
// bottom. Review is the base state.
//
// stage::rects() is the geometry, a pure function of the window's size, the
// picture's aspect and which panes are open; StageShell places its children by
// it. A format change lays out again at once, with no animation.
//
//   The sidebar is 240 wide, and gives way in a window under 1100.
//   With the panel closed the conversation has the rest. With it open the
//   conversation is 400 wide, giving way down to 320 only when it would leave
//   the panel under 600. At 1600 x 1000, the panel is 960 x 960 at (640, 40):
//   Wide (aspect >= 1.2):
//     picture slot 960 x 420 at the top, timeline 960 x 540 under it. The
//     timeline has 56% of the height, at least 260, and whatever the picture's
//     aspect leaves; the picture pillarboxes in black in its slot.
//   One pane alone has the panel: a picture with the transport's block 88 tall
//   under it, or a timeline with the transport in its own strip.
//   Vertical (aspect < 1.2), the picture is over the timeline too, whose share
//   is 36% of the height, at least 220; the picture fits inside the slot.
//   The picture keeps 12 points inside its slot, with rounded corners.
//
// Regions move to a new arrangement rather than jump to it, over 620 ms: the
// picture from what it grows out of (setPanes' origin), the timeline up from
// below. Under reduced motion, and while hidden, they are simply there.

#include "design_tokens.h"
#include "test_friends.h"

#include <QList>
#include <QPointer>
#include <QRect>
#include <QSize>
#include <QColor>
#include <QWidget>

class QVariantAnimation;

namespace kerf {

namespace stage {

// Which panes the window shows. The conversation always shows.
struct Panes {
    bool sidebar = true;
    bool picture = true;
    bool timeline = true;
    // Watching a proposal's changes: the stage grows over the conversation,
    // with a strip floating over the picture.
    bool reel = false;
    bool panelOpen() const { return picture || timeline; }
    bool operator==(const Panes &other) const {
        return sidebar == other.sidebar && picture == other.picture && timeline == other.timeline && reel == other.reel;
    }
};

// The sidebar, the panel (empty when closed), the picture's slot (`picture` is
// the frame inside it, in black bars when the slot is not of its shape), the
// transport's block under the picture (empty when the timeline's strip carries
// the transport), the conversation and the timeline, the three sheets and the
// tray. A pane that is closed has an empty rect.
struct Rects {
    bool wide = true;
    QRect sidebar;
    QRect panel; // what lies right of the conversation
    QRect reel;
    QRect slot;
    QRect picture;
    QRect transport;
    QRect conversation;
    QRect timeline;
    QRect media;
    QRect browser;
    QRect inspector;
    QRect tray;
};

// The part of a slot the picture fits inside.
QRect paddedSlot(const QRect &slot);
// The geometry in window coordinates; `toolbar` is the height over the content.
Rects rects(const QSize &window, double aspect, int toolbar = 40, const Panes &panes = {});
// The largest rect of `aspect` inside `slot`, centred.
QRect fitted(const QRect &slot, double aspect);
inline bool isWide(double aspect) { return aspect >= 1.2; }

} // namespace stage

class StageShell final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    enum class Sheet { None, Media, Browser, Inspector };
    enum class Tray { None, Finish, Deliver };

    explicit StageShell(QWidget *parent = nullptr);

    void setRegions(QWidget *picture, QWidget *conversation, QWidget *timeline);
    void setSidebar(QWidget *sidebar);
    // The strip over the picture while watching a proposal's changes.
    void setReelBar(QWidget *bar);
    // The versions of a request, over the timeline while they show.
    void setCompare(QWidget *compare);
    void setCompareShown(bool shown);
    bool compareShown() const { return m_compareShown; }
    // The block under the picture; hidden while the timeline's strip carries
    // the transport.
    void setTransport(QWidget *transport);
    void setSheets(QWidget *media, QWidget *browser, QWidget *inspector);
    void setTrays(QWidget *finish, QWidget *deliver);
    // Bars that take over the top of the content while they show (the
    // connection, an error, project.kerf changed on disk), over the regions:
    // the three regions keep their rects.
    void setBanners(const QList<QWidget *> &banners);
    QWidget *widgetOf(Sheet sheet) const;
    QWidget *widgetOf(Tray tray) const;

    void setAspect(double aspect);
    double aspect() const { return m_aspect; }
    // `pictureOrigin`, in this widget's coordinates: where the picture grows
    // from, when it opens.
    void setPanes(const stage::Panes &panes, const QRect &pictureOrigin = QRect());
    // The layer flights are drawn on, over everything but the bars.
    void setMotion(QWidget *motion);
    // How much of the room lies over the timeline, 0 to 1, eased there after
    // `delay` ms: it comes in out of the room, and falls back under other things.
    void veilTimeline(qreal level, int delay = 0, int duration = tokens::motion::settle);
    qreal timelineVeil() const;
    bool isMoving() const { return m_moves != nullptr; }
    const stage::Panes &panes() const { return m_panes; }
    // The geometry as this widget places it, in its own coordinates.
    stage::Rects rects() const;

    Sheet sheet() const { return m_sheet; }
    // One at a time; None closes the one open.
    void setSheet(Sheet sheet);
    // The same command opens and closes.
    void toggleSheet(Sheet sheet);
    Tray tray() const { return m_tray; }
    void setTray(Tray tray);
    // Whether `widget` is in the open sheet or tray.
    bool holdsOpen(const QWidget *widget) const;
    // Esc: closes the sheet, else the tray. False when neither is open.
    bool dismiss();

    static QString sheetId(Sheet sheet);

signals:
    void sheetChanged(StageShell::Sheet sheet);
    void trayChanged(StageShell::Tray tray);
    void panesChanged();
    void transitionStarted();
    // Every start has an end, also when the move is cut short.
    void transitionFinished();
    // Where the picture is going has changed; empty when there is none.
    void pictureMoved(const QRect &picture);

protected:
    void resizeEvent(QResizeEvent *event) override;
    void paintEvent(QPaintEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    struct Move {
        QPointer<QWidget> widget;
        QRect from;
        QRect to;
    };
    void place();
    void transition(const QRect &pictureOrigin = QRect());
    // Stops the move under way, if there is one, and says so.
    bool stopMoves();
    // What a move leaves behind it, however it ended.
    void settle();
    // What the picture's shadow falls on; empty when it is not painted.
    QRect shadowed() const;
    // Marks the room where the shadow was and is, not all of it: all that lies clear
    // over it is drawn again with it.
    void updateRoom();
    // While a sheet or tray is open Esc is theirs, ahead of the window's own
    // shortcuts on Esc, outside text fields, menus and dialogs.
    void followEscape();
    bool takesEscape(QObject *watched, QEvent *event);
    bool m_filtering = false;
    QWidget *m_sidebar = nullptr;
    QWidget *m_reelBar = nullptr;
    QWidget *m_motion = nullptr;
    QRect m_shadowed;
    QWidget *m_veil = nullptr;
    QVariantAnimation *m_moves = nullptr;
    QWidget *m_compare = nullptr;
    bool m_compareShown = false;
    QWidget *m_picture = nullptr;
    QWidget *m_conversation = nullptr;
    QWidget *m_timeline = nullptr;
    QWidget *m_transport = nullptr;
    QWidget *m_sheets[3] = {};
    QWidget *m_trays[2] = {};
    QList<QWidget *> m_banners;
    // The hairlines between the regions and at a sheet's left and a tray's top.
    QWidget *m_seamSidebar = nullptr;
    QWidget *m_seamUpright = nullptr;
    QWidget *m_seamAcross = nullptr;
    QWidget *m_seamColumn = nullptr;
    QWidget *m_seamSheet = nullptr;
    QWidget *m_seamTray = nullptr;
    double m_aspect = 16.0 / 9.0;
    stage::Panes m_panes;
    Sheet m_sheet = Sheet::None;
    Tray m_tray = Tray::None;
};

} // namespace kerf
