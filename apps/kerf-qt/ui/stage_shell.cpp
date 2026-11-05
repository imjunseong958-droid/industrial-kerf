#include "stage_shell.h"

#include "components.h"
#include "design_tokens.h"

#include <QAbstractSpinBox>
#include <QApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QLineEdit>
#include <QPainter>
#include <QPixmap>
#include <QPlainTextEdit>
#include <QRegion>
#include <QResizeEvent>
#include <QTextEdit>
#include <QTimer>
#include <QVariantAnimation>

#include <qdrawutil.h>

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
    const int side = panes.sidebar && w >= sidebarWindowLeast ? sidebar : 0;
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
    if (panes.picture && panes.timeline) {
        // The picture over the timeline, whichever its shape. Under a wide
        // picture the timeline also has what the picture's aspect does not use.
        const bool roomy = out.wide && !panes.reel;
        const double share = roomy ? timelineWideShare : timelineUprightShare;
        const int least = roomy ? timelineWideLeast : timelineUprightLeast;
        const int timeline = std::min(h, std::max(least, int(std::lround(h * share))));
        int slotHeight = h - timeline;
        if (out.wide) slotHeight = std::min(slotHeight, int(std::lround(stageWidth / shape)));
        out.slot = QRect(stageLeft, toolbar, stageWidth, slotHeight);
        out.timeline = QRect(stageLeft, toolbar + slotHeight, stageWidth, h - slotHeight);
    } else if (panes.picture) {
        // No timeline to carry the transport: the block is under the picture.
        const int block = std::min(h, pictureTransportLow);
        out.slot = QRect(stageLeft, toolbar, stageWidth, h - block);
        out.transport = QRect(stageLeft, toolbar + h - block, stageWidth, block);
    } else if (panes.timeline) {
        out.timeline = QRect(stageLeft, toolbar, stageWidth, h);
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
    // A sheet lies over the conversation alone, at its panel side, so a drag
    // reaches the timeline.
    const auto sheet = [&](int width) {
        const int shown = std::min(width, out.conversation.width());
        return QRect(out.conversation.right() + 1 - shown, out.conversation.top(), shown, out.conversation.height());
    };
    out.media = sheet(bin);
    out.browser = sheet(browser);
    out.inspector = sheet(inspector);
    const int tray = std::min(h, band);
    out.tray = QRect(side, toolbar + h - tray, room, tray);
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

} // namespace

StageShell::StageShell(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageShell"));
    setAttribute(Qt::WA_OpaquePaintEvent);
    m_seamSidebar = seam(this, "stageSeamSidebar");
    m_seamUpright = seam(this, "stageSeamUpright");
    m_seamAcross = seam(this, "stageSeamAcross");
    m_seamColumn = seam(this, "stageSeamColumn");
    m_seamSheet = seam(this, "stageSeamSheet");
    m_seamTray = seam(this, "stageSeamTray");
    m_seamSheet->hide();
    m_seamTray->hide();
    m_veil = new Veil(this);
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

void StageShell::setCompare(QWidget *compare) {
    m_compare = compare;
    if (compare) compare->setParent(this);
    place();
}

void StageShell::setCompareShown(bool shown) {
    if (shown == m_compareShown) return;
    m_compareShown = shown;
    place();
    update();
}

void StageShell::setTransport(QWidget *transport) {
    m_transport = transport;
    if (transport) transport->setParent(this);
    place();
}

void StageShell::setSheets(QWidget *media, QWidget *browser, QWidget *inspector) {
    m_sheets[0] = media;
    m_sheets[1] = browser;
    m_sheets[2] = inspector;
    for (QWidget *sheet : m_sheets)
        if (sheet) {
            sheet->setParent(this);
            sheet->hide();
        }
    place();
}

void StageShell::setTrays(QWidget *finish, QWidget *deliver) {
    m_trays[0] = finish;
    m_trays[1] = deliver;
    for (QWidget *tray : m_trays)
        if (tray) {
            tray->setParent(this);
            tray->hide();
        }
    place();
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

QWidget *StageShell::widgetOf(Tray tray) const { return tray == Tray::None ? nullptr : m_trays[int(tray) - 1]; }

QString StageShell::sheetId(Sheet sheet) {
    switch (sheet) {
    case Sheet::Media: return QStringLiteral("media");
    case Sheet::Browser: return QStringLiteral("browser");
    case Sheet::Inspector: return QStringLiteral("inspector");
    case Sheet::None: break;
    }
    return {};
}

void StageShell::setAspect(double aspect) {
    if (aspect <= 0.0 || aspect == m_aspect) return;
    m_aspect = aspect;
    transition();
}

void StageShell::setPanes(const stage::Panes &panes, const QRect &pictureOrigin) {
    if (panes == m_panes) return;
    // Back while it was still falling away, the timeline comes out of the room
    // again, unless what opened it says otherwise after this.
    const bool timelineBack = panes.timeline && !m_panes.timeline && static_cast<Veil *>(m_veil)->level() > 0.0;
    m_panes = panes;
    transition(pictureOrigin);
    if (timelineBack) veilTimeline(0.0);
    emit panesChanged();
}

void StageShell::setMotion(QWidget *motion) {
    m_motion = motion;
    if (motion) motion->setParent(this);
    place();
}

bool StageShell::stopMoves() {
    if (!m_moves) return false;
    m_moves->stop();
    m_moves->deleteLater();
    m_moves = nullptr;
    return true;
}

void StageShell::settle() {
    // Gone with what it lay over.
    if (m_timeline && !m_timeline->isVisible()) static_cast<Veil *>(m_veil)->hold(0.0);
    emit transitionFinished();
}

void StageShell::transition(const QRect &pictureOrigin) {
    const int duration = ui::motionDuration(tokens::motion::stage);
    const bool cutShort = stopMoves();
    if (duration <= 0 || !isVisible()) {
        place();
        if (cutShort) settle();
        emit pictureMoved(rects().picture);
        return;
    }
    QWidget *const regions[] = {m_sidebar, m_picture, m_conversation, m_timeline, m_transport, m_reelBar};
    QList<QPair<QWidget *, QRect>> before;
    for (QWidget *region : regions)
        if (region) before.append({region, region->isVisible() ? region->geometry() : QRect()});
    place();
    QList<Move> moves;
    for (const auto &[region, from] : std::as_const(before)) {
        const bool shown = region->isVisible();
        const QRect to = shown ? region->geometry() : QRect();
        Move move;
        move.widget = region;
        if (from.isNull() && to.isNull()) continue;
        if (from == to) continue;
        if (from.isNull()) {
            // It comes from somewhere: the picture from what it grows out of.
            move.from = region == m_picture && pictureOrigin.isValid() ? pictureOrigin
                        : region == m_sidebar                          ? to.translated(-to.width(), 0)
                        : region == m_timeline                         ? to.translated(0, 48)
                        : region == m_picture                          ? to.translated(48, 0)
                                                                       : to;
            move.to = to;
        } else if (to.isNull()) {
            move.from = from;
            // The picture goes back into what it names, if anything.
            move.to = region == m_sidebar                          ? from.translated(-from.width(), 0)
                      : region == m_timeline                       ? from.translated(0, 48)
                      : region == m_picture && pictureOrigin.isValid() ? pictureOrigin
                      : region == m_picture                        ? from.translated(48, 0)
                                                                   : from;
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
    m_moves = new QVariantAnimation(this);
    m_moves->setDuration(duration);
    m_moves->setStartValue(0.0);
    m_moves->setEndValue(1.0);
    m_moves->setEasingCurve(tokens::easing());
    connect(m_moves, &QVariantAnimation::valueChanged, this, [this, moves](const QVariant &value) {
        const qreal at = value.toReal();
        for (const Move &move : moves) {
            if (!move.widget) continue;
            const auto mix = [at](int a, int b) { return int(std::lround(a + (b - a) * at)); };
            move.widget->setGeometry(mix(move.from.left(), move.to.left()), mix(move.from.top(), move.to.top()),
                                     mix(move.from.width(), move.to.width()), mix(move.from.height(), move.to.height()));
            if (move.widget == m_timeline) m_veil->setGeometry(m_timeline->geometry());
        }
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

stage::Rects StageShell::rects() const {
    constexpr int toolbar = tokens::layout::toolbar;
    stage::Rects out = stage::rects(QSize(width(), height() + toolbar), m_aspect, toolbar, m_panes);
    for (QRect *rect : {&out.sidebar, &out.panel, &out.reel, &out.slot, &out.picture, &out.transport, &out.conversation, &out.timeline, &out.media, &out.browser, &out.inspector, &out.tray})
        if (!rect->isNull()) rect->translate(0, -toolbar);
    return out;
}

void StageShell::setSheet(Sheet sheet) {
    if (sheet == m_sheet) return;
    m_sheet = sheet;
    place();
    followEscape();
    emit sheetChanged(sheet);
}

void StageShell::toggleSheet(Sheet sheet) { setSheet(m_sheet == sheet ? Sheet::None : sheet); }

void StageShell::setTray(Tray tray) {
    if (tray == m_tray) return;
    m_tray = tray;
    place();
    followEscape();
    emit trayChanged(tray);
}

bool StageShell::dismiss() {
    if (m_sheet != Sheet::None) {
        setSheet(Sheet::None);
        return true;
    }
    if (m_tray != Tray::None) {
        setTray(Tray::None);
        return true;
    }
    return false;
}

void StageShell::followEscape() {
    const bool want = m_sheet != Sheet::None || m_tray != Tray::None;
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
    if (qobject_cast<QLineEdit *>(target) || qobject_cast<QTextEdit *>(target) || qobject_cast<QPlainTextEdit *>(target) ||
        qobject_cast<QAbstractSpinBox *>(target) || target == m_picture)
        return false;
    if (event->type() == QEvent::ShortcutOverride) {
        event->accept();
        return true;
    }
    dismiss();
    return true;
}

bool StageShell::holdsOpen(const QWidget *widget) const {
    for (const QWidget *open : {widgetOf(m_sheet), widgetOf(m_tray)})
        if (open && (open == widget || open->isAncestorOf(widget))) return true;
    return false;
}

void StageShell::place() {
    const stage::Rects r = rects();
    const auto placeRegion = [](QWidget *region, const QRect &rect) {
        if (!region) return;
        if (!rect.isEmpty()) region->setGeometry(rect);
        region->setVisible(!rect.isEmpty());
    };
    placeRegion(m_sidebar, r.sidebar);
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
    if (m_compare && m_compareShown) m_compare->raise();
    // The cards take the picture's place, with the timeline still under them.
    placeRegion(m_compare, m_compareShown ? r.slot : QRect());
    // One hairline between the regions, drawn inside the edge of what lies
    // right of or under it.
    const auto placeSeam = [](QWidget *seam, const QRect &rect) {
        if (!rect.isEmpty()) seam->setGeometry(rect);
        seam->setVisible(!rect.isEmpty());
        seam->raise();
    };
    placeSeam(m_seamSidebar, r.sidebar.isEmpty() ? QRect() : QRect(r.conversation.left(), r.conversation.top(), kSeam, r.conversation.height()));
    // No line between the conversation and the stage: they share the room.
    placeSeam(m_seamUpright, QRect());
    QRect across;
    if (!r.transport.isEmpty()) across = QRect(r.transport.left(), r.transport.top(), r.transport.width(), kSeam);
    else if (!r.slot.isEmpty() && !r.timeline.isEmpty()) across = QRect(r.timeline.left(), r.timeline.top(), r.timeline.width(), kSeam);
    placeSeam(m_seamAcross, across);
    placeSeam(m_seamColumn, QRect());

    // The tray, over the regions; a sheet over the tray.
    for (int index = 0; index < 2; ++index) {
        QWidget *tray = m_trays[index];
        if (!tray) continue;
        const bool shown = int(m_tray) == index + 1;
        if (shown) {
            tray->setGeometry(r.tray);
            tray->raise();
        }
        tray->setVisible(shown);
    }
    m_seamTray->setVisible(m_tray != Tray::None);
    if (m_tray != Tray::None) {
        m_seamTray->setGeometry(r.tray.left(), r.tray.top(), r.tray.width(), kSeam);
        m_seamTray->raise();
    }
    const QRect sheetRects[3] = {r.media, r.browser, r.inspector};
    for (int index = 0; index < 3; ++index) {
        QWidget *sheet = m_sheets[index];
        if (!sheet) continue;
        const bool shown = int(m_sheet) == index + 1;
        if (shown) {
            sheet->setGeometry(sheetRects[index]);
            sheet->raise();
        }
        sheet->setVisible(shown);
    }
    m_seamSheet->setVisible(m_sheet != Sheet::None);
    if (m_sheet != Sheet::None) {
        const QRect sheet = sheetRects[int(m_sheet) - 1];
        m_seamSheet->setGeometry(sheet.left() - kSeam, sheet.top(), kSeam, sheet.height());
        m_seamSheet->raise();
    }
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
    if (!m_picture || m_picture->isHidden() || m_compareShown) return {};
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
    p.fillRect(event->rect(), tokens::color(tokens::chrome));
    m_shadowed = shadowed();
    if (m_shadowed.isEmpty()) return;
    // The picture covers what is under its middle: only its corners show any.
    const QRect shown = stage::fitted(stage::paddedSlot(m_picture->geometry()), m_aspect);
    const int corner = tokens::layout::pictureRadius;
    p.setClipRegion(QRegion(m_shadowed) - shown.adjusted(corner, corner, -corner, -corner));
    // Its shadow, wide and soft under it.
    const QPixmap &tile = shadowTile(std::max(1.0, devicePixelRatioF()));
    const QMargins edges(kShadowCorner, kShadowCorner, kShadowCorner, kShadowCorner);
    const int cornerPixels = qRound(kShadowCorner * tile.devicePixelRatio());
    qDrawBorderPixmap(&p, m_shadowed, edges, tile, tile.rect(), QMargins(cornerPixels, cornerPixels, cornerPixels, cornerPixels));
}

bool StageShell::eventFilter(QObject *watched, QEvent *event) {
    if (watched->isWidgetType() && m_banners.contains(static_cast<QWidget *>(watched)) &&
        (event->type() == QEvent::ShowToParent || event->type() == QEvent::HideToParent || event->type() == QEvent::LayoutRequest))
        place();
    if (takesEscape(watched, event)) return true;
    return QWidget::eventFilter(watched, event);
}

} // namespace kerf
