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
//   Vertical (aspect < 1.2):
//     the picture over the transport's 160 tall block, 360 wide, beside the
//     timeline at the panel's full height. The timeline keeps 600 of width while
//     the picture's column can give it up, down to 240; the picture then
//     letterboxes in black.
//   One pane alone has the panel: a picture with the transport's block under
//   it (88 tall under a wide picture), or a timeline with the transport in its
//   own strip.

#include "test_friends.h"

#include <QList>
#include <QPointer>
#include <QRect>
#include <QSize>
#include <QWidget>

namespace kerf {

namespace stage {

// Which panes the window shows. The conversation always shows.
struct Panes {
    bool sidebar = true;
    bool picture = true;
    bool timeline = true;
    bool panelOpen() const { return picture || timeline; }
    bool operator==(const Panes &other) const {
        return sidebar == other.sidebar && picture == other.picture && timeline == other.timeline;
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
    QRect panel;
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
    void setPanes(const stage::Panes &panes);
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

protected:
    void resizeEvent(QResizeEvent *event) override;
    void paintEvent(QPaintEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    void place();
    // While a sheet or tray is open Esc is theirs, ahead of the window's own
    // shortcuts on Esc, outside text fields, menus and dialogs.
    void followEscape();
    bool takesEscape(QObject *watched, QEvent *event);
    bool m_filtering = false;
    QWidget *m_sidebar = nullptr;
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
