#include "stage_shell.h"

#include "components.h"
#include "design_tokens.h"

#include <QAbstractSpinBox>
#include <QApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QLineEdit>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QRegion>
#include <QResizeEvent>
#include <QTextEdit>
#include <QTimer>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace stage {

QRect fitted(const QRect &slot, double aspect) {
    if (slot.isEmpty() || aspect <= 0.0) return slot;
    int width = slot.width();
    int height = int(std::lround(width / aspect));
    if (height > slot.height()) {
        height = slot.height();
        width = std::min(slot.width(), int(std::lround(height * aspect)));
    }
    return QRect(slot.left() + (slot.width() - width) / 2, slot.top() + (slot.height() - height) / 2, width, height);
}

QRect paddedSlot(const QRect &slot) {
    using namespace tokens::layout;
    if (slot.width() <= 2 * picturePad || slot.height() <= 2 * picturePad) return slot;
    return slot.adjusted(picturePad, picturePad, -picturePad, -picturePad);
}

Rects rects(const QSize &window, double aspect, int toolbar, const Panes &panes) {
    using namespace tokens::layout;
    Rects out;
    const double shape = aspect > 0.0 ? aspect : 16.0 / 9.0;
    const int w = std::max(0, window.width());
    const int h = std::max(0, window.height() - toolbar);
    out.wide = isWide(shape);
    const int side = panes.sidebar && w >= sidebarWindowLeast ? std::clamp(panes.sidebarWidth, sidebarLeast, sidebarMost(w)) : 0;
    out.sidebar = side > 0 ? QRect(0, toolbar, side, h) : QRect();
    const int room = w - side;
    // With the panel closed the conversation has the room; with it open it
    // gives way only when it would leave the panel under its least, down to 320.
    const int conversation = panes.panelOpen() ? std::min(room, std::clamp(room - workLeast, railLeast, rail)) : room;
    out.conversation = QRect(side, toolbar, conversation, h);
    const int rest = room - conversation;
    const int left = side + conversation;
    if (panes.panelOpen()) out.panel = QRect(left, toolbar, rest, h);
    // While watching changes the stage grows over the conversation, which stays
    // where it is, dimmed.
    const int reach = panes.reel && panes.panelOpen() ? std::min(rest, reelReach) : 0;
    const int stageLeft = left - reach;
    const int stageWidth = rest + reach;
    if (reach > 0) out.panel = QRect(stageLeft, toolbar, stageWidth, h);
    // The Media band lies across the panel, under the picture and over the
    // timeline, so what is dragged from it has only a little way to go.
    const int band = panes.media && panes.panelOpen() ? std::min(mediaBand, h) : 0;
    if (panes.picture && panes.timeline) {
        // The picture over the timeline, whichever its shape. Under a wide
        // picture the timeline also has what the picture's aspect does not use.
        const bool roomy = out.wide && !panes.reel;
        const double share = roomy ? timelineWideShare : timelineUprightShare;
        const int least = roomy ? timelineWideLeast : timelineUprightLeast;
        const int timeline = std::min(h, std::max(least, int(std::lround(h * share))));
        int slotHeight = h - timeline;
        if (out.wide) slotHeight = std::min(slotHeight, int(std::lround(stageWidth / shape)));
        // The band takes the timeline's top; the picture gives way only when
        // that would leave the timeline under its least.
        if (band > 0) slotHeight = std::max(0, std::min(slotHeight, h - band - mediaBandTimelineLeast));
        out.slot = QRect(stageLeft, toolbar, stageWidth, slotHeight);
        if (band > 0) out.media = QRect(stageLeft, toolbar + slotHeight, stageWidth, band);
        out.timeline = QRect(stageLeft, toolbar + slotHeight + band, stageWidth, h - slotHeight - band);
    } else if (panes.picture) {
        // No timeline to carry the transport: the block is under the picture,
        // and the band between the two.
        const int block = std::min(h, pictureTransportLow);
        const int slotHeight = std::max(0, h - block - band);
        out.slot = QRect(stageLeft, toolbar, stageWidth, slotHeight);
        if (band > 0) out.media = QRect(stageLeft, toolbar + slotHeight, stageWidth, std::min(band, h - block));
        out.transport = QRect(stageLeft, toolbar + h - block, stageWidth, block);
    } else if (panes.timeline) {
        if (band > 0) out.media = QRect(stageLeft, toolbar, stageWidth, band);
        out.timeline = QRect(stageLeft, toolbar + band, stageWidth, h - band);
    }
    out.picture = out.slot.isEmpty() ? out.slot : fitted(paddedSlot(out.slot), shape);
    // The strip floats over the picture's top: at its corner, or centred on an
    // upright one narrower than it.
    if (panes.reel && !out.slot.isEmpty()) {
        const int strip = std::min(out.slot.width() - 2 * picturePad, 460);
        const int least = out.slot.left() + picturePad;
        const int most = out.slot.right() + 1 - picturePad - strip;
        const int left = out.picture.width() >= strip + 2 * picturePad ? out.picture.left() + picturePad
                                                                       : out.picture.center().x() - strip / 2;
        out.reel = QRect(std::clamp(left, least, std::max(least, most)), out.picture.top() + picturePad, strip, reelBar);
    }
    return out;
}

} // namespace stage

namespace {

constexpr int kSeam = tokens::layout::seam;

// Painted, not filled from the palette: the app sheet's `QWidget { background:
// transparent }` would leave a palette fill invisible.
class Seam final : public QWidget {
public:
    Seam(QWidget *parent, const char *name) : QWidget(parent) {
        setObjectName(QString::fromLatin1(name));
        setAttribute(Qt::WA_TransparentForMouseEvents);
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        QPainter p(this);
        p.fillRect(event->rect(), tokens::color(tokens::line));
    }
};

QWidget *seam(QWidget *parent, const char *name) { return new Seam(parent, name); }

constexpr int kShadowReach = 50;  // how far the shadow goes past the picture
constexpr int kShadowDrop = 18;   // and how far it falls below it
constexpr int kShadowSteps = 10;  // the layers it is built of
constexpr int kShadowCorner = kShadowReach + tokens::layout::pictureRadius; // what a corner of it takes

// The picture's shadow, wide and soft, for any size of picture: its corners and
// edges, drawn once, and laid out around the picture in each frame.
const QPixmap &shadowTile(qreal ratio) {
    static QPixmap tile;
    if (!tile.isNull() && qFuzzyCompare(tile.devicePixelRatio(), ratio)) return tile;
    const int side = 2 * kShadowCorner + 2;
    QImage image(QSize(side, side) * ratio, QImage::Format_ARGB32_Premultiplied);
    image.setDevicePixelRatio(ratio);
    image.fill(Qt::transparent);
    QPainter p(&image);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    p.setBrush(QColor(0, 0, 0, 12));
    const QRectF picture(kShadowReach, kShadowReach, side - 2 * kShadowReach, side - 2 * kShadowReach);
    for (int step = kShadowSteps; step >= 1; --step) {
        const qreal out = qreal(step * kShadowReach) / kShadowSteps;
        p.drawRoundedRect(picture.adjusted(-out, -out, out, out), tokens::layout::pictureRadius + out, tokens::layout::pictureRadius + out);
    }
    p.end();
    tile = QPixmap::fromImage(image);
    return tile;
}

// Keeps a widget that paints every pixel it is asked for marked opaque: the
// editor's style sheet clears the attribute as it polishes a widget, and Qt then
// paints what is under the widget too, in each frame, only to be drawn over.
class KeepsOpaque final : public QObject {
public:
    explicit KeepsOpaque(QWidget *widget) : QObject(widget) {
        widget->setAttribute(Qt::WA_OpaquePaintEvent);
        widget->installEventFilter(this);
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        // The style sheet clears it as it polishes, after the Polish event: set
        // again once shown, and after each restyle.
        const QEvent::Type type = event->type();
        if (type == QEvent::Show || type == QEvent::StyleChange || type == QEvent::Polish)
            static_cast<QWidget *>(watched)->setAttribute(Qt::WA_OpaquePaintEvent);
        return false;
    }
};

// A region as it was when a move began, shown in its place while it travels:
// one picture copied in each frame, not all that is in it drawn again.
class Still final : public QWidget {
public:
    // The picture is laid over `room`, what is under it, so it covers all it is
    // over: the room is not drawn beneath it, and each frame copies it rather
    // than blends it.
    Still(QWidget *parent, const QPixmap &picture, const QColor &room) : QWidget(parent) {
        setObjectName(QStringLiteral("stageStill"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        new KeepsOpaque(this);
        QImage opaque(picture.size(), QImage::Format_RGB32);
        opaque.setDevicePixelRatio(picture.devicePixelRatio());
        opaque.fill(room);
        {
            QPainter p(&opaque);
            p.drawPixmap(0, 0, picture);
        }
        m_picture = QPixmap::fromImage(std::move(opaque));
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        QPainter p(this);
        p.drawPixmap(event->rect(), m_picture, QRectF(QPointF(event->rect().topLeft()) * m_picture.devicePixelRatio(),
                                                      QSizeF(event->rect().size()) * m_picture.devicePixelRatio()).toRect());
    }

private:
    QPixmap m_picture;
};

// The room's colour over a region, as much as `level`.
class Veil final : public QWidget {
public:
    explicit Veil(QWidget *parent) : QWidget(parent) {
        setObjectName(QStringLiteral("stageVeil"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        hide();
        m_ease.setEasingCurve(tokens::easing());
        connect(&m_ease, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) { apply(value.toReal()); });
    }
    qreal level() const { return m_level; }
    void fadeTo(qreal level, int delay, int duration) {
        ++m_order;
        m_ease.stop();
        const int length = ui::motionDuration(duration);
        if (length <= 0) {
            apply(level);
            return;
        }
        const int order = m_order;
        QTimer::singleShot(delay, this, [this, level, length, order] {
            if (order != m_order) return;
            m_ease.setDuration(length);
            m_ease.setStartValue(m_level);
            m_ease.setEndValue(level);
            m_ease.start();
        });
    }
    void hold(qreal level) {
        ++m_order;
        m_ease.stop();
        apply(level);
    }

protected:
    void paintEvent(QPaintEvent *event) override {
        QPainter p(this);
        QColor room = tokens::color(tokens::chrome);
        room.setAlphaF(float(m_level));
        p.fillRect(event->rect(), room);
    }

private:
    void apply(qreal level) {
        m_level = level;
        setVisible(level > 0.0 && parentWidget());
        update();
    }
    QVariantAnimation m_ease;
    qreal m_level = 0.0;
    int m_order = 0;
};

// The sidebar's edge: dragged, it sets how wide the sidebar is; pressed, it
// puts the sidebar away; pressed twice, the width goes back to the first.
// Nothing is drawn until the pointer is over it; then a line says what would move.
class SidebarGrip final : public QWidget {
public:
    SidebarGrip(QWidget *parent, std::function<void(int)> drag, std::function<void()> press)
        : QWidget(parent), m_drag(std::move(drag)), m_press(std::move(press)) {
        setObjectName(QStringLiteral("stageSidebarGrip"));
        setCursor(Qt::SplitHCursor);
        setAccessibleName(QStringLiteral("Sidebar width"));
        setToolTip(QStringLiteral("Drag to resize · click to hide · double-click to reset"));
        // A press waits to see whether a second follows.
        m_single.setSingleShot(true);
        QObject::connect(&m_single, &QTimer::timeout, this, [this] { m_press(); });
    }

protected:
    void paintEvent(QPaintEvent *) override {
        if (!underMouse() && !m_dragging) return;
        QPainter p(this);
        constexpr int line = tokens::layout::sidebarGripLine;
        p.fillRect(QRect(width() / 2 - line / 2, 0, line, height()), tokens::color(m_dragging ? tokens::accent : tokens::lineh));
    }
    void enterEvent(QEnterEvent *) override { update(); }
    void leaveEvent(QEvent *) override { update(); }
    void mousePressEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        m_dragging = true;
        m_moved = false;
        m_from = event->globalPosition().toPoint();
        update();
    }
    void mouseMoveEvent(QMouseEvent *event) override {
        if (!m_dragging || !parentWidget()) return;
        const QPoint at = event->globalPosition().toPoint();
        if (!m_moved && (at - m_from).manhattanLength() < QApplication::startDragDistance()) return;
        m_moved = true;
        m_single.stop();
        m_drag(parentWidget()->mapFromGlobal(at).x());
    }
    void mouseReleaseEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        const bool pressed = m_dragging && !m_moved;
        m_dragging = false;
        update();
        if (pressed && !m_double) m_single.start(QApplication::doubleClickInterval());
        m_double = false;
    }
    void mouseDoubleClickEvent(QMouseEvent *event) override {
        if (event->button() != Qt::LeftButton) return;
        m_single.stop();
        m_double = true;
        m_dragging = true;
        m_moved = false;
        m_drag(tokens::layout::sidebar);
    }

private:
    std::function<void(int)> m_drag;
    std::function<void()> m_press;
    QTimer m_single;
    QPoint m_from;
    bool m_dragging = false;
    bool m_moved = false;
    bool m_double = false;
};

} // namespace

StageShell::StageShell(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageShell"));
    // It fills all it repaints with the room.
    new KeepsOpaque(this);
    m_seamSidebar = seam(this, "stageSeamSidebar");
    m_seamUpright = seam(this, "stageSeamUpright");
    m_seamAcross = seam(this, "stageSeamAcross");
    m_seamColumn = seam(this, "stageSeamColumn");
    m_veil = new Veil(this);
    m_grip = new SidebarGrip(this, [this](int x) { setSidebarWidth(x); }, [this] { emit sidebarPutAway(); });
    m_grip->hide();
}

void StageShell::setRegions(QWidget *picture, QWidget *conversation, QWidget *timeline) {
    m_picture = picture;
    m_conversation = conversation;
    m_timeline = timeline;
    for (QWidget *region : {picture, conversation, timeline})
        if (region) region->setParent(this);
    place();
}

void StageShell::setSidebar(QWidget *sidebar) {
    m_sidebar = sidebar;
    if (sidebar) sidebar->setParent(this);
    place();
}

void StageShell::setReelBar(QWidget *bar) {
    m_reelBar = bar;
    if (bar) bar->setParent(this);
    place();
}


void StageShell::setTransport(QWidget *transport) {
    m_transport = transport;
    if (transport) transport->setParent(this);
    place();
}

void StageShell::setSheets(QWidget *media) {
    m_sheets[0] = media;
    for (QWidget *sheet : m_sheets)
        if (sheet) {
            sheet->setParent(this);
            sheet->hide();
        }
    place();
}

void StageShell::setCover(QWidget *cover) {
    m_cover = cover;
    if (cover) {
        cover->setParent(this);
        cover->hide();
    }
    place();
}

void StageShell::setCovered(bool covered) {
    covered = covered && m_cover;
    if (covered == m_covered) return;
    m_covered = covered;
    place();
    followEscape();
    emit coveredChanged(covered);
}

void StageShell::setBanners(const QList<QWidget *> &banners) {
    m_banners = banners;
    for (QWidget *banner : banners) {
        banner->setParent(this);
        // Shown or hidden, or its text rewrapped, the bars stack again.
        banner->installEventFilter(this);
    }
    place();
}

QWidget *StageShell::widgetOf(Sheet sheet) const { return sheet == Sheet::None ? nullptr : m_sheets[int(sheet) - 1]; }


QString StageShell::sheetId(Sheet sheet) {
    switch (sheet) {
    case Sheet::Media: return QStringLiteral("media");
    case Sheet::None: break;
    }
    return {};
}

void StageShell::setAspect(double aspect) {
    if (aspect <= 0.0 || aspect == m_aspect) return;
    const stage::Rects was = rects();
    m_aspect = aspect;
    transition(was);
}

void StageShell::setPanes(const stage::Panes &panes, const QRect &pictureOrigin, int duration) {
    if (panes == m_panes) return;
    // Back while it was still falling away, the timeline comes out of the room
    // again, unless what opened it says otherwise after this.
    const bool timelineBack = panes.timeline && !m_panes.timeline && static_cast<Veil *>(m_veil)->level() > 0.0;
    const stage::Rects was = rects();
    const bool sheetChange = panes.media != m_panes.media;
    m_panes = panes;
    if (sheetChange) m_sheet = panes.media ? Sheet::Media : Sheet::None;
    transition(was, pictureOrigin, duration);
    if (timelineBack) veilTimeline(0.0);
    if (sheetChange) {
        followEscape();
        emit sheetChanged(m_sheet);
    }
    emit panesChanged();
}

void StageShell::setSidebarWidth(int width) {
    width = std::clamp(width, tokens::layout::sidebarLeast, tokens::layout::sidebarMost(this->width()));
    if (width == m_panes.sidebarWidth) return;
    m_panes.sidebarWidth = width;
    // Dragged: it is where the pointer is, with nothing to wait for.
    const bool cutShort = stopMoves();
    place();
    if (cutShort) settle();
    emit pictureMoved(rects().picture);
    emit sidebarWidthChanged(width);
}

void StageShell::setMotion(QWidget *motion) {
    m_motion = motion;
    if (motion) motion->setParent(this);
    place();
}

void StageShell::releaseStill() {
    if (!m_still) return;
    m_still->hide();
    m_still->deleteLater();
    m_still = nullptr;
    if (m_timeline) m_timeline->setUpdatesEnabled(true);
}

bool StageShell::stopMoves() {
    releaseStill();
    for (QWidget *sheet : m_sheets)
        if (sheet) sheet->clearMask();
    if (!m_moves) return false;
    m_moves->stop();
    m_moves->deleteLater();
    m_moves = nullptr;
    return true;
}

void StageShell::settle() {
    releaseStill();
    // The Media band is whole again, or gone.
    for (QWidget *sheet : m_sheets)
        if (sheet) sheet->clearMask();
    // Gone with what it lay over.
    if (m_timeline && !m_timeline->isVisible()) static_cast<Veil *>(m_veil)->hold(0.0);
    emit transitionFinished();
}

void StageShell::transition(const stage::Rects &was, const QRect &pictureOrigin, int length) {
    const int duration = ui::motionDuration(length);
    const bool cutShort = stopMoves();
    if (duration <= 0 || !isVisible()) {
        place();
        if (cutShort) settle();
        emit pictureMoved(rects().picture);
        return;
    }
    QWidget *const sheet = m_sheets[0];
    QWidget *const regions[] = {m_sidebar, m_picture, m_conversation, m_timeline, m_transport, m_reelBar, sheet};
    QList<QPair<QWidget *, QRect>> before;
    for (QWidget *region : regions)
        if (region) before.append({region, region->isVisible() ? region->geometry() : QRect()});
    place();
    const stage::Rects now = rects();
    // Where a picture that closes goes back to, now that the regions are where
    // they are going: what it names, else its home.
    QRect back = pictureOrigin;
    if (!back.isValid() && m_pictureHome && m_picture && !m_picture->isVisible()) back = m_pictureHome();
    // A pane comes in from the edge it belongs to, and goes out by it, its own
    // edge against the edge of what makes room: the panel from the right as the
    // conversation narrows, the timeline up from the foot under a picture that
    // gives it the height, the picture down from the top over a timeline that
    // was alone. So nothing lies over anything on the way.
    const bool panelOpens = was.panel.isEmpty() && !now.panel.isEmpty();
    const bool panelCloses = !was.panel.isEmpty() && now.panel.isEmpty();
    // The left edge of what lies right of the conversation.
    const int edgeWas = was.panel.isEmpty() ? width() : was.panel.left();
    const int edgeNow = now.panel.isEmpty() ? width() : now.panel.left();
    const auto offstage = [&](QWidget *region, const QRect &at) {
        // The strip over the picture is there or not, where it is.
        if (region == m_reelBar) return at;
        if (region == m_sidebar) return at.translated(-at.right() - 1, 0);
        if (panelOpens || panelCloses) return at.translated(width() - (panelOpens ? edgeNow : edgeWas), 0);
        if (region == m_picture) return at.translated(0, -at.bottom() - 1);
        // The timeline, and the transport's block: from the foot.
        return at.translated(0, height() - at.top());
    };
    QList<Move> moves;
    for (const auto &[region, from] : std::as_const(before)) {
        const bool shown = region->isVisible();
        const QRect to = shown ? region->geometry() : QRect();
        Move move;
        move.widget = region;
        if (from.isNull() && to.isNull()) continue;
        if (from == to) continue;
        if (region == sheet && !panelOpens && !panelCloses) {
            // The Media band keeps its height and comes out from under what lies
            // over it, seen only under that: the timeline moves down for it,
            // and up over it as it goes back.
            const QRect at = to.isNull() ? from : to;
            move.from = from.isNull() ? at.translated(0, -at.height()) : from;
            move.to = to.isNull() ? at.translated(0, -at.height()) : to;
            if (to.isNull()) region->show();
        } else if (from.isNull()) {
            // It comes from somewhere: the picture from what it grows out of.
            move.from = region == m_picture && pictureOrigin.isValid() ? pictureOrigin : offstage(region, to);
            move.to = to;
        } else if (to.isNull()) {
            move.from = from;
            // The picture goes back into what it names, if anything.
            move.to = region == m_picture && back.isValid() ? back : offstage(region, from);
            region->show();
        } else {
            move.from = from;
            move.to = to;
        }
        region->setGeometry(move.from);
        moves << move;
    }
    emit pictureMoved(rects().picture);
    if (moves.isEmpty()) {
        if (cutShort) settle();
        return;
    }
    for (QWidget *seam : {m_seamSidebar, m_seamUpright, m_seamAcross, m_seamColumn}) seam->hide();
    m_grip->hide();
    // The timeline only travels, in or out or along: shown from a picture of it
    // for the move, rather than drawn again, with all its controls, in each frame.
    for (const Move &move : std::as_const(moves)) {
        if (move.widget != m_timeline || move.from.size() != move.to.size() || move.from.isEmpty()) continue;
        const QPixmap picture = m_timeline->grab();
        if (picture.isNull()) break;
        m_still = new Still(this, picture, tokens::color(tokens::chrome));
        m_still->setGeometry(move.from);
        m_still->stackUnder(m_veil);
        m_still->show();
        // Still there, as far as anything asks: only not drawn while its picture is.
        m_timeline->setUpdatesEnabled(false);
        break;
    }
    // What of the Media band shows: all that is under the picture, or under the
    // panel's top with no picture over it.
    const auto clipSheet = [this, sheet] {
        if (!sheet || sheet->isHidden()) return;
        const int over = m_picture && m_picture->isVisible() ? m_picture->geometry().bottom() + 1 : 0;
        const int hidden = std::max(0, over - sheet->y());
        if (hidden >= sheet->height()) sheet->setMask(QRegion(-1, -1, 1, 1));
        else sheet->setMask(QRegion(0, hidden, sheet->width(), sheet->height() - hidden));
    };
    clipSheet();
    m_moves = new QVariantAnimation(this);
    m_moves->setDuration(duration);
    m_moves->setStartValue(0.0);
    m_moves->setEndValue(1.0);
    m_moves->setEasingCurve(tokens::easing());
    connect(m_moves, &QVariantAnimation::valueChanged, this, [this, moves, clipSheet](const QVariant &value) {
        const qreal at = value.toReal();
        for (const Move &move : moves) {
            if (!move.widget) continue;
            const auto mix = [at](int a, int b) { return int(std::lround(a + (b - a) * at)); };
            // An edge is mixed, not a width, so two regions that share one
            // keep sharing it in every frame.
            const int left = mix(move.from.left(), move.to.left());
            const int top = mix(move.from.top(), move.to.top());
            const int right = mix(move.from.left() + move.from.width(), move.to.left() + move.to.width());
            const int bottom = mix(move.from.top() + move.from.height(), move.to.top() + move.to.height());
            move.widget->setGeometry(left, top, right - left, bottom - top);
            if (move.widget == m_timeline) {
                m_veil->setGeometry(m_timeline->geometry());
                if (m_still) m_still->setGeometry(m_timeline->geometry());
            }
        }
        clipSheet();
        updateRoom();
    });
    connect(m_moves, &QAbstractAnimation::finished, this, [this] {
        if (m_moves) m_moves->deleteLater();
        m_moves = nullptr;
        place();
        settle();
    });
    emit transitionStarted();
    m_moves->start();
}

int StageShell::barsHeight(int barWidth) const {
    int bars = 0;
    for (const QWidget *banner : std::as_const(m_banners))
        if (!banner->isHidden()) bars += banner->hasHeightForWidth() ? banner->heightForWidth(barWidth) : banner->sizeHint().height();
    return bars;
}

stage::Rects StageShell::rects() const {
    constexpr int toolbar = tokens::layout::toolbar;
    stage::Rects out = stage::rects(QSize(width(), height() + toolbar), m_aspect, toolbar, m_panes);
    // The bars lie over the content's top. A Media band there starts under them,
    // so that its tabs and its close are not hidden while one shows.
    const int bars = std::min(barsHeight(width() - out.sidebar.width()), std::max(0, out.media.height() / 2));
    if (bars > 0 && !out.media.isNull() && out.media.top() == toolbar) out.media.adjust(0, bars, 0, 0);
    for (QRect *rect : {&out.sidebar, &out.panel, &out.reel, &out.slot, &out.picture, &out.transport, &out.conversation, &out.timeline, &out.media})
        if (!rect->isNull()) rect->translate(0, -toolbar);
    return out;
}

void StageShell::setSheet(Sheet sheet) {
    if (sheet == m_sheet) return;
    // The conversation and the panel part for it, and close after it.
    stage::Panes panes = m_panes;
    panes.media = sheet == Sheet::Media;
    setPanes(panes, QRect(), tokens::motion::panel);
}

void StageShell::toggleSheet(Sheet sheet) { setSheet(m_sheet == sheet ? Sheet::None : sheet); }

bool StageShell::dismiss() {
    if (m_covered) {
        setCovered(false);
        return true;
    }
    if (m_sheet != Sheet::None) {
        setSheet(Sheet::None);
        return true;
    }
    return false;
}

void StageShell::followEscape() {
    const bool want = m_sheet != Sheet::None || m_covered;
    if (want == m_filtering || !qApp) return;
    m_filtering = want;
    if (want) qApp->installEventFilter(this);
    else qApp->removeEventFilter(this);
}

bool StageShell::takesEscape(QObject *watched, QEvent *event) {
    if (event->type() != QEvent::ShortcutOverride && event->type() != QEvent::KeyPress) return false;
    auto *key = static_cast<QKeyEvent *>(event);
    if (key->key() != Qt::Key_Escape || key->modifiers() != Qt::NoModifier) return false;
    auto *target = qobject_cast<QWidget *>(watched);
    if (!target || target->window() != window() || QApplication::activePopupWidget() || QApplication::activeModalWidget()) return false;
    // A text field and the picture (a drawing, a mask tool) answer first; what
    // they leave reaches the window, which closes it too.
    // On the page that has the room, Esc is the way back wherever the keys are.
    const bool onCover = m_covered && m_cover && (target == m_cover || m_cover->isAncestorOf(target));
    if (!onCover && (qobject_cast<QLineEdit *>(target) || qobject_cast<QTextEdit *>(target) || qobject_cast<QPlainTextEdit *>(target) ||
                     qobject_cast<QAbstractSpinBox *>(target) || target == m_picture))
        return false;
    if (event->type() == QEvent::ShortcutOverride) {
        event->accept();
        return true;
    }
    dismiss();
    return true;
}

bool StageShell::holdsOpen(const QWidget *widget) const {
    const QWidget *open = widgetOf(m_sheet);
    return open && (open == widget || open->isAncestorOf(widget));
}

void StageShell::place() {
    const stage::Rects r = rects();
    const auto placeRegion = [](QWidget *region, const QRect &rect) {
        if (!region) return;
        if (!rect.isEmpty()) region->setGeometry(rect);
        region->setVisible(!rect.isEmpty());
    };
    placeRegion(m_sidebar, r.sidebar);
    // Its edge takes a drag, from just inside it to just outside.
    const int grip = tokens::layout::sidebarGrip;
    if (!r.sidebar.isEmpty()) m_grip->setGeometry(r.sidebar.right() + 1 - grip / 2, r.sidebar.top(), grip, r.sidebar.height());
    m_grip->setVisible(!r.sidebar.isEmpty());
    placeRegion(m_picture, r.slot);
    placeRegion(m_conversation, r.conversation);
    placeRegion(m_timeline, r.timeline);
    placeRegion(m_transport, r.transport);
    placeRegion(m_reelBar, r.reel);
    // The stage lies over the conversation while it reaches across it.
    for (QWidget *region : {m_picture, m_timeline, m_transport, m_reelBar})
        if (region) region->raise();
    if (m_timeline) m_veil->setGeometry(m_timeline->geometry());
    m_veil->raise();
    // One hairline between the regions, drawn inside the edge of what lies
    // right of or under it.
    const auto placeSeam = [](QWidget *seam, const QRect &rect) {
        if (!rect.isEmpty()) seam->setGeometry(rect);
        seam->setVisible(!rect.isEmpty());
        seam->raise();
    };
    // The sidebar draws the line it ends on itself, in its own last column,
    // so the line goes where it goes and is never put away while regions move.
    placeSeam(m_seamSidebar, QRect());
    // No line between the conversation and the stage: they share the room.
    placeSeam(m_seamUpright, QRect());
    QRect across;
    if (!r.transport.isEmpty()) across = QRect(r.transport.left(), r.transport.top(), r.transport.width(), kSeam);
    else if (!r.timeline.isEmpty() && (!r.slot.isEmpty() || !r.media.isEmpty()))
        across = QRect(r.timeline.left(), r.timeline.top(), r.timeline.width(), kSeam);
    placeSeam(m_seamAcross, across);
    placeSeam(m_seamColumn, QRect());

    // The Media band, under the picture and over the timeline. It draws its own
    // line at its top.
    const QRect sheetRects[1] = {r.media};
    for (int index = 0; index < 1; ++index) {
        QWidget *sheet = m_sheets[index];
        if (!sheet) continue;
        const bool shown = int(m_sheet) == index + 1 && !sheetRects[index].isEmpty();
        if (shown) {
            sheet->setGeometry(sheetRects[index]);
            sheet->raise();
        }
        sheet->setVisible(shown);
    }
    m_grip->raise();
    if (m_motion) {
        m_motion->setGeometry(rect());
        m_motion->raise();
    }
    // The bars, stacked at the top of the content, over everything but the sidebar.
    const int barLeft = r.sidebar.width();
    const int barWidth = width() - barLeft;
    int top = 0;
    for (QWidget *banner : std::as_const(m_banners)) {
        if (banner->isHidden()) continue;
        const int height = banner->hasHeightForWidth() ? banner->heightForWidth(barWidth) : banner->sizeHint().height();
        banner->setGeometry(barLeft, top, barWidth, height);
        banner->raise();
        top += height;
    }
    // What one goes into, over all of the window: its scrim dims the rest.
    if (m_cover) {
        if (m_covered) {
            m_cover->setGeometry(rect());
            m_cover->raise();
        }
        m_cover->setVisible(m_covered);
    }
    if (m_motion && m_covered) m_motion->raise();
    updateRoom();
}

void StageShell::resizeEvent(QResizeEvent *) {
    const bool cutShort = stopMoves();
    place();
    if (cutShort) settle();
    emit pictureMoved(rects().picture);
}

void StageShell::veilTimeline(qreal level, int delay, int duration) {
    auto *veil = static_cast<Veil *>(m_veil);
    if (delay == 0 && duration == 0) veil->hold(level);
    else veil->fadeTo(level, delay, duration);
}

qreal StageShell::timelineVeil() const { return static_cast<Veil *>(m_veil)->level(); }

QRect StageShell::shadowed() const {
    if (!m_picture || m_picture->isHidden()) return {};
    const QRect picture = stage::fitted(stage::paddedSlot(m_picture->geometry()), m_aspect);
    return picture.isEmpty() ? QRect() : picture.adjusted(-kShadowReach, -kShadowReach + kShadowDrop, kShadowReach, kShadowReach + kShadowDrop);
}

void StageShell::updateRoom() {
    const QRect falling = shadowed();
    QRegion dirty(falling);
    dirty += m_shadowed;
    m_shadowed = falling;
    if (!dirty.isEmpty()) update(dirty);
}

void StageShell::paintEvent(QPaintEvent *event) {
    // One room: the conversation and the picture's region are clear over it, so the
    // picture floats. The room stays neutral, whatever the picture shows, as colour
    // around a picture changes how its colours look.
    QPainter p(this);
    // Not where the picture paints every pixel over it: as it plays, that is all
    // there is to repaint.
    QRegion room = event->region();
    if (m_pictureCovers) room -= m_pictureCovers();
    for (const QRect &rect : room) p.fillRect(rect, tokens::color(tokens::chrome));
    m_shadowed = shadowed();
    if (m_shadowed.isEmpty()) return;
    // The picture covers what is under its middle: only its corners show any.
    const QRect shown = stage::fitted(stage::paddedSlot(m_picture->geometry()), m_aspect);
    const int corner = tokens::layout::pictureRadius;
    // Its shadow, wide and soft under it: each piece of the tile drawn only where
    // it shows, its corners copied and its edges and middle drawn out between
    // them. One clip for each piece costs less than one for all of them.
    const QPixmap &tile = shadowTile(std::max(1.0, devicePixelRatioF()));
    const QRegion falls = (QRegion(m_shadowed) - shown.adjusted(corner, corner, -corner, -corner)) & room;
    const int c = kShadowCorner;
    const int cp = qRound(c * tile.devicePixelRatio());
    const int mp = tile.width() - 2 * cp;
    const QRect &r = m_shadowed;
    const int w = std::max(0, r.width() - 2 * c);
    const int h = std::max(0, r.height() - 2 * c);
    const auto piece = [&](const QRect &to, const QRect &from) {
        const QRegion part = falls & to;
        if (part.isEmpty()) return;
        p.setClipRegion(part);
        p.drawPixmap(to, tile, from);
    };
    piece(QRect(r.left(), r.top(), c, c), QRect(0, 0, cp, cp));
    piece(QRect(r.left() + c + w, r.top(), c, c), QRect(cp + mp, 0, cp, cp));
    piece(QRect(r.left(), r.top() + c + h, c, c), QRect(0, cp + mp, cp, cp));
    piece(QRect(r.left() + c + w, r.top() + c + h, c, c), QRect(cp + mp, cp + mp, cp, cp));
    piece(QRect(r.left() + c, r.top(), w, c), QRect(cp, 0, mp, cp));
    piece(QRect(r.left() + c, r.top() + c + h, w, c), QRect(cp, cp + mp, mp, cp));
    piece(QRect(r.left(), r.top() + c, c, h), QRect(0, cp, cp, mp));
    piece(QRect(r.left() + c + w, r.top() + c, c, h), QRect(cp + mp, cp, cp, mp));
    piece(QRect(r.left() + c, r.top() + c, w, h), QRect(cp, cp, mp, mp));
}

bool StageShell::eventFilter(QObject *watched, QEvent *event) {
    if (watched->isWidgetType() && m_banners.contains(static_cast<QWidget *>(watched)) &&
        (event->type() == QEvent::ShowToParent || event->type() == QEvent::HideToParent || event->type() == QEvent::LayoutRequest))
        place();
    if (takesEscape(watched, event)) return true;
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
