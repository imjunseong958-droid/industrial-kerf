#pragma once

// The window's content. It starts as a conversation: the folders and
// conversations at the left, the conversation beside them. The picture and the
// timeline are the panel at the right, opened when there is something to see
// and closed at will. Everything else opens on demand: the Media band, across
// the panel between the picture and the timeline.
//
// stage::rects() is the geometry, a pure function of the window's size, the
// picture's aspect and which panes are open; StageShell places its children by
// it. A format change lays out again at once, with no animation.
//
//   The sidebar is 320 wide until its edge is dragged (from 232 to all but 640
//   of the window), and gives way in a window under 1100. With the panel closed
//   the conversation has the rest. With it open the conversation is 448 wide,
//   giving way down to 320 only when it would leave the panel under 600. At
//   1600 x 1000, the panel is 832 x 960 at (768, 40):
//   Wide (aspect >= 1.2):
//     picture slot 832 x 420 at the top, timeline 832 x 540 under it. The
//     timeline has 56% of the height, at least 260, and whatever the picture's
//     aspect leaves; the picture pillarboxes in black in its slot.
//   One pane alone has the panel: a picture with the transport's block 88 tall
//   under it, or a timeline with the transport in its own strip.
//   Vertical (aspect < 1.2), the picture is over the timeline too, whose share
//   is 36% of the height, at least 220; the picture fits inside the slot.
//   The picture keeps 12 points inside its slot, with rounded corners.
//   The Media band, open, is 152 tall under the picture: the timeline gives it
//   its top, and the picture its foot only when the timeline would be left
//   under 200. Under a picture alone it is over the transport's block; over a
//   timeline alone, at the panel's top.
//
// Regions move to a new arrangement rather than jump to it. A pane that opens
// comes in from the window's edge it belongs to, its edge against the edge of
// what makes room for it, so nothing lies over anything on the way: the
// sidebar from the left, the panel from the right as the conversation narrows,
// the timeline up from the foot, the Media band down from under the picture as
// the timeline moves down for it. The picture alone can come from somewhere
// else: what it grows out of (setPanes' origin). Under reduced motion, and
// while hidden, they are simply there.

#include "design_tokens.h"
#include "test_friends.h"

#include <QList>
#include <QPointer>
#include <QRect>
#include <QRegion>
#include <QSize>
#include <QColor>
#include <QWidget>

#include <functional>

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
    // The Media band: across the panel, under the picture and over the
    // timeline a drag from it goes to. It covers nothing, and takes nothing
    // from what is said.
    bool media = false;
    // How wide the sidebar is, as its edge was last dragged.
    int sidebarWidth = tokens::layout::sidebar;
    bool panelOpen() const { return picture || timeline; }
    bool operator==(const Panes &other) const {
        return sidebar == other.sidebar && picture == other.picture && timeline == other.timeline && reel == other.reel &&
               media == other.media && sidebarWidth == other.sidebarWidth;
    }
};

// The sidebar, the panel (empty when closed), the picture's slot (`picture` is
// the frame inside it, in black bars when the slot is not of its shape), the
// transport's block under the picture (empty when the timeline's strip carries
// the transport), the conversation and the timeline, and the Media band.
// A pane that is closed has an empty rect.
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
    QRect media; // the Media band; empty while it is closed or the panel is
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
    enum class Sheet { None, Media };

    explicit StageShell(QWidget *parent = nullptr);

    void setRegions(QWidget *picture, QWidget *conversation, QWidget *timeline);
    void setSidebar(QWidget *sidebar);
    // The strip over the picture while watching a proposal's changes.
    void setReelBar(QWidget *bar);
    // The block under the picture; hidden while the timeline's strip carries
    // the transport.
    void setTransport(QWidget *transport);
    void setSheets(QWidget *media);
    // A page that takes the room while it shows (Settings): everything right
    // of the sidebar, over the regions and a sheet, under the bars. Esc
    // uncovers.
    // One at a time: another widget given while one covers takes its place,
    // and the window is uncovered first.
    void setCover(QWidget *cover);
    QWidget *cover() const { return m_cover; }
    void setCovered(bool covered);
    bool covered() const { return m_covered; }
    // Bars that take over the top of the content while they show (the
    // connection, an error, project.kerf changed on disk), over the regions:
    // the three regions keep their rects.
    void setBanners(const QList<QWidget *> &banners);
    QWidget *widgetOf(Sheet sheet) const;

    void setAspect(double aspect);
    double aspect() const { return m_aspect; }
    // `pictureOrigin`, in this widget's coordinates: where the picture grows
    // from, when it opens. `duration` is how long the regions take to get
    // there: the stage's own time, or a pane's when one is opened by hand.
    void setPanes(const stage::Panes &panes, const QRect &pictureOrigin = QRect(), int duration = tokens::motion::stage);
    // The sidebar's width, as its edge is dragged: at once, with no motion,
    // and kept to what the window leaves it (tokens::layout::sidebarMost).
    void setSidebarWidth(int width);
    // What takes that drag, and a press, over the sidebar's edge.
    QWidget *sidebarGrip() const { return m_grip; }
    // Where the picture goes back into as it closes with no origin given, asked
    // once the regions have their new places (empty: it slides away).
    void setPictureHome(std::function<QRect()> home) { m_pictureHome = std::move(home); }
    // What of the room the picture's region paints over, every pixel of it, in
    // this widget's coordinates: the room is not filled there first.
    void setPictureCovers(std::function<QRegion()> covers) { m_pictureCovers = std::move(covers); }
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
    // How tall the bars that show are, laid out barWidth wide.
    int barsHeight(int barWidth) const;

    Sheet sheet() const { return m_sheet; }
    // One at a time; None closes the one open.
    void setSheet(Sheet sheet);
    // The same command opens and closes.
    void toggleSheet(Sheet sheet);
    // Whether `widget` is in the open sheet.
    bool holdsOpen(const QWidget *widget) const;
    // Esc: uncovers, else closes the sheet. False when there was neither.
    bool dismiss();

    static QString sheetId(Sheet sheet);

signals:
    // The sidebar's edge was dragged to a new width.
    void sidebarWidthChanged(int width);
    // Its edge was pressed, not dragged: the window puts the sidebar away.
    void sidebarPutAway();
    void sheetChanged(StageShell::Sheet sheet);
    void coveredChanged(bool covered);
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
    // Moves the regions from where they are, laid out as `was`, to where they
    // now belong.
    void transition(const stage::Rects &was, const QRect &pictureOrigin = QRect(), int duration = tokens::motion::stage);
    // Stops the move under way, if there is one, and says so.
    bool stopMoves();
    // The timeline's picture during a move, if it is shown from one; gone with it.
    void releaseStill();
    QPointer<QWidget> m_still;
    // What a move leaves behind it, however it ended.
    void settle();
    // What the picture's shadow falls on; empty when it is not painted.
    QRect shadowed() const;
    // Marks the room where the shadow was and is, not all of it: all that lies clear
    // over it is drawn again with it.
    void updateRoom();
    // While a sheet is open Esc is its own, ahead of the window's own
    // shortcuts on Esc, outside text fields, menus and dialogs.
    void followEscape();
    bool takesEscape(QObject *watched, QEvent *event);
    bool m_filtering = false;
    QWidget *m_sidebar = nullptr;
    QWidget *m_grip = nullptr;
    QWidget *m_reelBar = nullptr;
    QWidget *m_motion = nullptr;
    QRect m_shadowed;
    QWidget *m_veil = nullptr;
    QVariantAnimation *m_moves = nullptr;
    QWidget *m_picture = nullptr;
    QWidget *m_conversation = nullptr;
    QWidget *m_timeline = nullptr;
    QWidget *m_transport = nullptr;
    QWidget *m_sheets[1] = {};
    QWidget *m_cover = nullptr;
    bool m_covered = false;
    QList<QWidget *> m_banners;
    // The hairlines between the regions.
    QWidget *m_seamSidebar = nullptr;
    QWidget *m_seamUpright = nullptr;
    QWidget *m_seamAcross = nullptr;
    QWidget *m_seamColumn = nullptr;
    double m_aspect = 16.0 / 9.0;
    stage::Panes m_panes;
    std::function<QRect()> m_pictureHome;
    std::function<QRegion()> m_pictureCovers;
    Sheet m_sheet = Sheet::None;
};

} // namespace kerf
