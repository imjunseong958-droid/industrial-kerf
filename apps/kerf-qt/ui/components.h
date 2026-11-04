#pragma once

// The shared flat components of the review-thread design (mockup-spec 0.5).
// Every one paints itself from design_tokens.h and the icon set, with no
// style sheet, so the app's QSS cannot change how it looks:
//
//   IconButton       24/28/20 square icon buttons with hover, on and disabled
//   Separator        the 1 x 16 vertical hairline
//   SegmentedControl text or icon segments with a sliding fill and a menu tail
//   GhostButton      text button with no outline, plain or mono, with a chevron
//   Button           primary or secondary push button, min width 64
//   CheckBox         the 14 pt box with the check path
//   Slider           16 tall, 2 pt track, 10 pt knob, bipolar tick
//   Chip             filter chip with a count
//   Badge            the count badge
//   Avatar           person (circle), agent or QC (radius 6), Kerf (g-kerf)
//   Dot              6 pt status dot
//   ThinProgress     the 4 pt progress bar
//   FadeScrollArea   hidden scroll bar, 12 pt top fade, 32 pt bottom fade
//   Popover          card with the pop shadow, placed and flipped as menus are
//   styleMenu()      spec 0.5 menus on a QMenu (header, check column, keys)
//   TooltipManager   the app-wide name-and-shortcut tooltip, and setTip()
//
// Animations last the design's times (tokens::motion) on its ease, and none
// run when reduced motion is on (reducedMotion()).

#include "design_tokens.h"

#include <QAbstractButton>
#include <QColor>
#include <QElapsedTimer>
#include <QFont>
#include <QKeySequence>
#include <QList>
#include <QMargins>
#include <QPointer>
#include <QRectF>
#include <QScrollArea>
#include <QString>
#include <QTimer>
#include <QVariant>
#include <QWidget>

#include <functional>

class QAction;
class QFontMetricsF;
class QMenu;
class QPainter;
class QVariantAnimation;

namespace kerf::ui {

// ---------------------------------------------------------------- motion

// Whether animations are skipped: the override when one is set, else
// KERF_REDUCE_MOTION=1/0 in the environment, else the system's Reduce motion
// setting (macOS), else off. The system setting is read again at most once a
// second, so turning it on or off applies while the app runs. GUI thread only.
bool reducedMotion();
// -1 follows the environment and the system; 0 forces motion on, 1 off.
void setReducedMotionOverride(int state);
// `ms`, or 0 under reduced motion.
int motionDuration(int ms);

// ---------------------------------------------------------------- painting helpers

// A colour between `from` and `to`, t in 0..1 (alpha included).
QColor mix(const QColor &from, const QColor &to, qreal t);
// ceil(horizontalAdvance): the design snaps text-sized widths to whole points.
int snapWidth(const QFontMetricsF &metrics, const QString &text);
int snapWidth(const tokens::TypeStyle &style, const QString &text);
// box-shadow: inset 0 0 0 <width>px <color> on a rounded rect.
void paintInsetLine(QPainter &painter, const QRectF &rect, qreal radius, const QColor &color, qreal width = 1);
// box-shadow: inset 0 1px 0 <color> (a top highlight inside a rounded rect).
void paintTopHighlight(QPainter &painter, const QRectF &rect, qreal radius, const QColor &color);
// The 2 pt accent focus ring, 1 pt outside `rect`.
void paintFocusRing(QPainter &painter, const QRectF &rect, qreal radius);
// A blurred box shadow (CSS box-shadow) for a rounded rect, painted outside it.
void paintShadow(QPainter &painter, const QRectF &rect, qreal radius, const tokens::Shadow &shadow = tokens::pop);
// How far the pop shadow reaches outside its card: left, top, right, bottom.
QMargins shadowMargins(const tokens::Shadow &shadow = tokens::pop);

// ---------------------------------------------------------------- tooltips

// Gives `widget` the design's tooltip: its name and, when not empty, its
// shortcut (mockup-spec 15). An empty name removes it. The TooltipManager
// shows it; the widget's own QToolTip is not used. Children without a tip of
// their own show their closest tipped ancestor's (as closest('[data-tip]')
// does); a child with its own setToolTip() keeps its Qt tooltip. Disabled
// widgets show their tip too, so a disabled control can say why.
void setTip(QWidget *widget, const QString &name, const QString &shortcut = {});
QString tipName(const QWidget *widget);
QString tipShortcut(const QWidget *widget);
// `keys` as a tip or menu shows them on this platform: ⌘↩ on a Mac, Ctrl+Return
// elsewhere.
QString keyText(const QKeySequence &keys);

class TooltipManager final : public QObject {
    Q_OBJECT

public:
    static constexpr int delayMs = 400;       // before a tip shows
    static constexpr int quickMs = 300;       // a tip hidden this recently makes the next one instant
    static constexpr int gap = 6;             // between the control and the tip
    static constexpr int tipHeight = 24;
    static constexpr int flipBelow = 540;     // a control whose bottom is past this ...
    static constexpr int flipAbove = 100;     // ... and whose top is past this gets its tip above
    static constexpr int edge = 8;            // tips keep this far from the window's sides

    // The one manager, filtering the application's events (made on first use).
    static TooltipManager *instance();

    // Milliseconds from any fixed start; the default is a monotonic clock.
    // Tests give a fake one and call tick() after moving it.
    void setClock(std::function<qint64()> clock);
    // Shows the pending tip when its delay has passed by the clock.
    void tick();

    // Where a tip of `tip` size goes for a control at `control`, both in the
    // control's window coordinates, in a window of `window` size: centred
    // under the control (6 below), or 6 above when the control's bottom is
    // past 540 and its top past 100; x kept within 8 ... W - 8.
    static QRect placement(const QRectF &control, QSize tip, QSize window);

    bool isShowing() const { return m_showing; }
    QWidget *target() const { return m_target; }
    QString shownName() const;
    QString shownShortcut() const;
    QWidget *tipWindow() const;

    // Starts the tip for `widget` (or the closest ancestor with one) as if
    // the pointer had entered it; hide() as if it had left.
    void hover(QWidget *widget);
    void hide();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    TooltipManager();
    void show(QWidget *widget);
    qint64 now() const;

    std::function<qint64()> m_clock;
    QElapsedTimer m_elapsed;
    QTimer m_timer;
    QPointer<QWidget> m_target;
    QPointer<QWidget> m_window;
    qint64 m_pendingSince = -1;
    qint64 m_lastHidden = -1'000'000;
    bool m_showing = false;
};

// ---------------------------------------------------------------- base button

// What the kit's buttons share: a 120 ms hover fade, the keyboard-only focus
// ring, and setTip().
class KitButton : public QAbstractButton {
    Q_OBJECT

public:
    explicit KitButton(QWidget *parent = nullptr);
    void setTip(const QString &name, const QString &shortcut = {});
    // 0 at rest, 1 hovered, between while it fades.
    qreal hoverProgress() const { return m_hover; }
    bool isHovered() const { return m_hovered; }
    // Whether it shows the focus ring now (keyboard focus from Tab).
    bool focusVisible() const { return m_focusVisible && hasFocus(); }

protected:
    void enterEvent(QEnterEvent *event) override;
    void leaveEvent(QEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void changeEvent(QEvent *event) override;

private:
    void fadeTo(qreal target);
    QVariantAnimation *m_fade = nullptr;
    qreal m_hover = 0;
    bool m_hovered = false;
    bool m_focusVisible = false;
};

// ---------------------------------------------------------------- IconButton

// .ib: radius 6, a 16 pt icon in ink2. Hover: raised + ink. On: on + ink +
// inset line2. Disabled: dis, no events. 28 in the toolbar, 20 in the pin
// bar, the reply-to bar and track toggles (with a 14 pt icon).
class IconButton final : public KitButton {
    Q_OBJECT

public:
    enum class Size { S20 = 20, S24 = 24, S28 = 28 };
    explicit IconButton(const QString &icon = {}, Size size = Size::S24, QWidget *parent = nullptr);

    void setIconName(const QString &icon);
    QString iconName() const { return m_icon; }
    void setButtonSize(Size size);
    Size buttonSize() const { return m_size; }
    // The icon's size: 16, or 14 on a 20 button, unless set.
    void setIconPx(qreal px);
    qreal iconPx() const;
    // The on state (.ib.on): a checked, checkable button.
    void setOn(bool on);
    bool isOn() const { return isChecked(); }
    // Ink instead of ink2 at rest, for the one control that leads a row.
    void setInkAtRest(bool ink);
    // The colour the icon is drawn in now.
    QColor iconColor() const;

    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_icon;
    Size m_size;
    bool m_inkAtRest = false;
    qreal m_iconPx = -1;
};

// ---------------------------------------------------------------- Separator

// .vsep: a 1 x 16 line with `margin` on each side (4, or 8 in the toolbar).
class Separator final : public QWidget {
    Q_OBJECT

public:
    explicit Separator(int margin = 4, QWidget *parent = nullptr);
    void setMargin(int margin);
    int margin() const { return m_margin; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    int m_margin;
};

// ---------------------------------------------------------------- SegmentedControl

// .seg: a raised track (24 tall, 28 in the toolbar) with 2 pt padding and
// radius 6; segments 20 (24) tall, radius 4, pad 0 8, UI 12/16 ink2, hover
// ink, on ink 500. Each text segment is as wide as its label at weight 500,
// so turning a segment on never shifts the layout. One sliding fill (on,
// inset line2, a 6% top highlight) moves to the current segment in 200 ms.
// Icon segments are 24 wide (or 28/20) with no padding. An optional 20 pt
// tail with a 14 pt chevron asks for a menu.
class SegmentedControl final : public QWidget {
    Q_OBJECT

public:
    explicit SegmentedControl(QWidget *parent = nullptr);

    // Adds a segment and returns its index.
    int addSegment(const QString &text, const QString &tip = {}, const QString &shortcut = {});
    int addIconSegment(const QString &icon, const QString &tip = {}, const QString &shortcut = {});
    int count() const;
    void setSegmentEnabled(int index, bool enabled);
    void setSegmentTip(int index, const QString &tip, const QString &shortcut = {});
    void setSegmentData(int index, const QVariant &data);
    QVariant segmentData(int index) const;
    QString segmentText(int index) const;

    // 24 (the default) or 28 (the toolbar).
    void setTrackHeight(int height);
    int trackHeight() const { return m_trackHeight; }
    // Width of icon segments: 24 (the default), 28 or 20.
    void setIconSegmentWidth(int width);
    // One width for every text segment (0, the default: each its label's
    // width at weight 500 plus 8 each side), as the toolbar's and the
    // timeline's heights want.
    void setTextSegmentWidth(int width);
    int textSegmentWidth() const { return m_textWidth; }
    // A count after a text segment's label (" 12"), UI 12/16 at weight 400
    // in ink3 whether on or not; a negative count removes it.
    void setSegmentCount(int index, int count);
    int segmentCount(int index) const;
    // The menu-only tail (20 wide, 14 pt chevron). It emits menuRequested.
    void setMenuTail(bool on, const QString &tip = {}, const QString &shortcut = {});
    bool hasMenuTail() const;
    QWidget *menuTail() const;

    int currentIndex() const { return m_current; }
    // Moves the fill to `index`; it slides (200 ms) when `animate` and the
    // control is visible and motion is allowed. -1 chooses none: no fill,
    // every segment off (currentChanged(-1)).
    void setCurrentIndex(int index, bool animate = true);

    // In this widget's coordinates: a segment's rect, and the fill as drawn now.
    QRectF segmentRect(int index) const;
    QRectF fillRect() const { return m_fill; }
    QWidget *segment(int index) const;

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override { return sizeHint(); }

signals:
    // The current segment changed (by click, keyboard or setCurrentIndex).
    void currentChanged(int index);
    // The user picked a segment, even the current one.
    void activated(int index);
    void menuRequested();

protected:
    void paintEvent(QPaintEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void focusInEvent(QFocusEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void relayout();
    QRectF targetFill() const;
    QList<KitButton *> m_segments;
    KitButton *m_tail = nullptr;
    QVariantAnimation *m_slide = nullptr;
    QRectF m_fill;
    int segmentWidth(int index) const;
    int m_current = -1;
    int m_trackHeight = 24;
    int m_iconWidth = 24;
    int m_textWidth = 0;
    bool m_focusVisible = false;
};

// ---------------------------------------------------------------- GhostButton

// .ghost-b: 24 tall (28 in the toolbar; 20 in small rows), radius 6, pad 0 8,
// gap 4, UI 12/16 ink2; hover raised + ink. Mono (.m) uses mono 11/16. An
// optional leading 16 pt icon and a trailing 14 pt chevron.
class GhostButton final : public KitButton {
    Q_OBJECT

public:
    explicit GhostButton(const QString &text = {}, QWidget *parent = nullptr);

    void setMono(bool mono);
    bool isMono() const { return m_mono; }
    // The type style; setMono picks monoSmall or control. Small rows use 11 pt UI.
    void setTypeStyle(const tokens::TypeStyle &style);
    tokens::TypeStyle typeStyle() const { return m_style; }
    void setChevron(bool chevron);
    bool hasChevron() const { return m_chevron; }
    void setIconName(const QString &icon);
    void setControlHeight(int height);
    int controlHeight() const { return m_height; }
    void setHorizontalPadding(int padding);
    // The meta look of small row actions ("Edit roles…"): UI 11/16 in ink3,
    // ink on hover, 20 tall.
    void setMeta(bool meta);
    bool isMeta() const { return m_meta; }
    QColor textColor() const;

    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    tokens::TypeStyle m_style = tokens::control;
    QString m_icon;
    int m_height = 24;
    int m_padding = 8;
    bool m_mono = false;
    bool m_chevron = false;
    bool m_meta = false;
};

// ---------------------------------------------------------------- Button

// .btn: 24 tall, min width 64, radius 6, pad 0 10, gap 8, UI 500 12/16.
// Primary: accent, white text; hover accent-hover. Secondary: ink with an
// inset line2; hover raised + inset lineh. Disabled: dis text, inset line,
// no events.
class Button final : public KitButton {
    Q_OBJECT

public:
    enum class Kind { Primary, Secondary };
    explicit Button(const QString &text = {}, Kind kind = Kind::Secondary, QWidget *parent = nullptr);

    void setKind(Kind kind);
    Kind kind() const { return m_kind; }
    void setIconName(const QString &icon);
    QColor background() const;
    QColor textColor() const;

    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    Kind m_kind;
    QString m_icon;
};

// ---------------------------------------------------------------- CheckBox

// .cbx: 14 x 14, radius 4, inset 1 pt white 16%. On: the on fill and the
// check path (#a8a8a8, 1.75, drawn at 14 pt). An optional label (UI 12/16
// ink) follows 8 pt after it.
class CheckBox final : public KitButton {
    Q_OBJECT

public:
    explicit CheckBox(const QString &text = {}, QWidget *parent = nullptr);
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;
    bool hitButton(const QPoint &pos) const override;
};

// ---------------------------------------------------------------- Slider

// .sl: 16 tall. A 2 pt track at y 7, radius 1, in track; the fill in track2
// (from the left, or from the centre when bipolar); a 10 pt ink knob with
// the knob shadow, centred on the value; bipolar sliders show a 1 x 6 track2
// tick at the centre. The track runs between 5 pt insets so the knob is
// never clipped: give the slider 10 more than the track's width.
class Slider final : public QWidget {
    Q_OBJECT

public:
    static constexpr int knob = 10;
    static constexpr int inset = knob / 2;

    explicit Slider(QWidget *parent = nullptr);

    void setRange(double minimum, double maximum);
    double minimum() const { return m_min; }
    double maximum() const { return m_max; }
    void setValue(double value);
    double value() const { return m_value; }
    // A keyboard step (arrows); Shift takes ten.
    void setSingleStep(double step);
    void setBipolar(bool bipolar);
    bool isBipolar() const { return m_bipolar; }
    // 0..1 along the track.
    double position() const;
    // The knob's centre and the track, in this widget's coordinates.
    QPointF knobCentre() const;
    QRectF trackRect() const;
    QRectF fillRect() const;
    void setTip(const QString &name, const QString &shortcut = {});

    QSize sizeHint() const override;
    QSize minimumSizeHint() const override;

signals:
    void valueChanged(double value);
    // Only from the user's drags and keys, not setValue().
    void sliderMoved(double value);
    void sliderPressed();
    void sliderReleased();

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;

private:
    void setFromX(qreal x);
    double m_min = 0;
    double m_max = 1;
    double m_value = 0;
    double m_step = 0.01;
    bool m_bipolar = false;
    bool m_dragging = false;
};

// ---------------------------------------------------------------- Chip

// .chip: 24 tall, radius 6, pad 0 8, gap 4, UI 12/16 ink2, count in ink3;
// hover raised. On: the on fill, ink 500, inset line2 and the top highlight,
// count in ink2. The width is that of the on (500) label, so it never shifts.
class Chip final : public KitButton {
    Q_OBJECT

public:
    explicit Chip(const QString &text = {}, QWidget *parent = nullptr);
    // -1 hides the count.
    void setCount(int count);
    int count() const { return m_count; }
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    int m_count = -1;
};

// ---------------------------------------------------------------- Badge

// The count badge (.rbtn .bdg): min 16 x 16, pad 0 4, radius 8, ink2 fill,
// #0e0e0e mono 500 11/16 text, a 2 pt ring in the colour behind it. The widget
// includes the ring, so it is 4 larger each way than the badge.
class Badge final : public QWidget {
    Q_OBJECT

public:
    static constexpr int ring = 2;
    explicit Badge(const QString &text = {}, QWidget *parent = nullptr);
    void setText(const QString &text);
    QString text() const { return m_text; }
    void setCount(int count);
    // The ring's colour (the surface the badge sits on); invalid draws none.
    void setRingColor(const QColor &color);
    QRectF badgeRect() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QString m_text;
    QColor m_ring = tokens::color(tokens::surround);
};

// ---------------------------------------------------------------- Avatar

// .av: 24 x 24 on the on fill, UI 500 11/16 ink2 initials. A person is a
// circle (g-user at 14 pt when there are no initials); an agent is a
// radius-6 square; Kerf shows g-kerf (16 pt) and QC g-gauge (14 pt), both
// radius 6. Selected: hover2 and an inset line2.
class Avatar final : public QWidget {
    Q_OBJECT

public:
    enum class Kind { Person, Agent, Kerf, Qc };
    explicit Avatar(Kind kind = Kind::Person, const QString &initials = {}, QWidget *parent = nullptr);

    void setKind(Kind kind);
    Kind kind() const { return m_kind; }
    void setInitials(const QString &initials);
    QString initials() const { return m_initials; }
    void setSelected(bool selected);
    bool isSelected() const { return m_selected; }
    void setDiameter(int diameter);
    bool isRound() const { return m_kind == Kind::Person; }
    // The bridge's author kinds ("person", "mcp", "kerf", "qc") as an avatar kind.
    static Kind kindFor(const QString &authorKind);
    // Two letters for a name: the first letters of its first two words
    // ("Mina Park" -> "MP", "Claude Code" -> "CC"), else its first two letters.
    static QString initialsFor(const QString &name);

    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    Kind m_kind;
    QString m_initials;
    int m_diameter = 24;
    bool m_selected = false;
};

// ---------------------------------------------------------------- Dot

// .dot6: a 6 pt circle.
class Dot final : public QWidget {
    Q_OBJECT

public:
    explicit Dot(const QColor &color = tokens::color(tokens::ink2), QWidget *parent = nullptr);
    void setColor(const QColor &color);
    QColor color() const { return m_color; }
    QSize sizeHint() const override { return {6, 6}; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    QColor m_color;
};

// ---------------------------------------------------------------- ThinProgress

// .pb: 4 tall, radius 2, the track colour, filled in accent to the value.
class ThinProgress final : public QWidget {
    Q_OBJECT

public:
    explicit ThinProgress(QWidget *parent = nullptr);
    // 0..1 (clamped).
    void setValue(double value);
    double value() const { return m_value; }
    QRectF fillRect() const;
    QSize sizeHint() const override { return {120, 4}; }

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    double m_value = 0;
};

// ---------------------------------------------------------------- FadeScrollArea

// A scroll area with its scroll bars hidden (it still scrolls by wheel,
// trackpad and keys). A 12 pt fade covers the top only when a child of the
// content is cut by the top edge, and a 32 pt fade the bottom while there is
// more below (the mockup's #list.ft / .fb masks), in the colour behind it.
class FadeScrollArea final : public QScrollArea {
    Q_OBJECT

public:
    static constexpr int topFade = 12;
    static constexpr int bottomFade = 32;

    explicit FadeScrollArea(QWidget *parent = nullptr);
    // The surface colour the fades blend into (card by default).
    void setFadeColor(const QColor &color);
    QColor fadeColor() const { return m_fadeColor; }
    bool topFadeShown() const { return m_top; }
    bool bottomFadeShown() const { return m_bottom; }
    // Recomputes the fades (called on scroll and resize; call after the
    // content changes in a way the area cannot see).
    void updateFades();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void scrollContentsBy(int dx, int dy) override;

private:
    QWidget *m_overlay = nullptr;
    QColor m_fadeColor = tokens::color(tokens::card);
    bool m_top = false;
    bool m_bottom = false;
};

// ---------------------------------------------------------------- Popover

// Where menus and popovers open (mockup placeMenu): 4 below the anchor, left
// aligned with it; right-aligned with it when that would pass W - 8; never
// left of 8; above it (4 over its top) when the bottom would pass H - 32.
// Side::Right opens 8 right of the anchor at its top, and falls back below
// the anchor when there is no room on the right. All in `bounds`
// coordinates (the window), returning the popover's rect.
enum class Side { Below, Right };
QRect popoverPlacement(const QRectF &anchor, QSize size, const QRect &bounds, Side side = Side::Below);

// A card (radius 8, card fill, pop shadow, inset line2, pad 4) that floats
// over the window as a popup: a click outside or Esc closes it. It opens
// with the design's 160 ms fade and 4 pt rise.
class Popover final : public QWidget {
    Q_OBJECT

public:
    explicit Popover(QWidget *parent = nullptr);
    // The content, laid out inside the 4 pt padding. The popover owns it.
    void setContent(QWidget *content);
    QWidget *content() const { return m_content; }
    void setMinimumCardWidth(int width);
    // Opens at popoverPlacement() of `anchor` within its window.
    void openAt(QWidget *anchor, Side side = Side::Below);
    // The card's rect in global coordinates (the window is larger by the shadow).
    QRect cardGeometry() const;

signals:
    void closed();

protected:
    void paintEvent(QPaintEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    // A press in the shadow halo (inside the window, outside the card) is
    // a click outside: it closes the popover.
    void mousePressEvent(QMouseEvent *event) override;
    void hideEvent(QHideEvent *event) override;

private:
    QWidget *m_content = nullptr;
    int m_minWidth = 0;
};

// ---------------------------------------------------------------- menus

// Gives `menu` the spec 0.5 menu look, drawn by the kit whatever the app's
// style sheet says (the menu gets a small style sheet of its own that pins
// its metrics, and the kit paints it): min width 220, card, radius 8, pop shadow and inset
// line2, pad 4; items 28 tall, radius 6, pad 0 8, gap 8, UI 12/16 ink, hover
// raised; a 14 pt check column (the check icon in ink) when any item is
// checkable or has a dot; the shortcut (or setMenuKey text) right-aligned in
// mono 11/16 ink3 with 16 before it; headers UI 11/16 ink3 with pad 8 8 4;
// separators 1 pt line with 4 above and below; disabled items in dis with no
// hover; warn items in neg; an accent action in UI 12 accent-ink. A press in
// the shadow halo around the card closes the menu, as a click outside does.
void styleMenu(QMenu *menu);
// Opens a styled menu at popoverPlacement() of `anchor` (with the 160 ms fade).
void popupMenu(QMenu *menu, QWidget *anchor, Side side = Side::Below);
// The least width of `menu`'s card (220 by default), for menus the design
// draws wider (the Workspaces and Sequence menus); 0 goes back to 220.
void setMenuMinimumCardWidth(QMenu *menu, int width);
int menuMinimumCardWidth(const QMenu *menu);

enum class MenuRole { Normal, Header, Warn };
// A non-interactive header row (.mhd): a disabled row the kit paints as a
// header. (QMenu::addSection rows get a separator's height in a kit menu.)
QAction *addMenuHeader(QMenu *menu, const QString &text);
void setMenuRole(QAction *action, MenuRole role);
MenuRole menuRole(const QAction *action);
// The right column's text, instead of the action's shortcut. `uiFont` uses
// the UI face (.k.ui) instead of mono.
void setMenuKey(QAction *action, const QString &key, bool uiFont = false);
QString menuKey(const QAction *action);
// An icon (ink3, 14 pt) in the right column.
void setMenuKeyIcon(QAction *action, const QString &icon);
// A 6 pt dot in the check column (the preflight rows).
void setMenuDot(QAction *action, const QColor &color);
// Accent text in the right column (.ak), such as "Ask Kerf".
void setMenuAccent(QAction *action, const QString &text);

} // namespace kerf::ui
