#include "stage_rail.h"
#include "editor_theme.h"
#include "program_level.h"

#include <QBrush>
#include <QEvent>
#include <QLinearGradient>
#include <QPainter>
#include <QPainterPath>
#include <QPolygonF>
#include <QRadialGradient>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

// ---- RailButton -----------------------------------------------------------

namespace {

// A rail button's disc as the hardware finish's raised key, round: its ring
// over a 1px drop, a face shaded from the top with a line of light inside
// its top edge; held in, it sits down where its drop was, its face darker
// with the rim's shadow in it. Lit, a 10x2 amber LED glows at the face's
// top. Returns the face, and what fills it, so a glyph's gap can be cut in
// the same colour.
struct RailDisc {
    QRectF face;
    QBrush fill;
};

// `rect` with its edges on device pixels' edges, so the LED's straight
// sides land crisp at 1.25x as the keys' do.
QRectF snapToDevicePixels(const QPainter &painter, const QRectF &rect) {
    const QTransform t = painter.deviceTransform();
    bool invertible = false;
    const QTransform back = t.inverted(&invertible);
    if (t.type() > QTransform::TxScale || !invertible) return rect;
    const QRectF device = t.mapRect(rect);
    return back.mapRect(QRectF(QPointF(std::round(device.left()), std::round(device.top())),
                               QPointF(std::round(device.right()), std::round(device.bottom()))));
}

RailDisc paintRailDisc(QPainter &painter, const QRectF &disc, bool latched, bool hovered, bool lit) {
    const EditorTheme &t = editorTheme();
    // One device pixel at 1x and 1.25x, two at 2x, as the keys' lines are.
    const QTransform device = painter.deviceTransform();
    const qreal scale = std::hypot(device.m11(), device.m12());
    const qreal line = scale > 0 ? std::max<qreal>(1.0, std::round(scale)) / scale : 1.0;
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    // The ring is a circle a line narrower than the disc, so its drop (or,
    // held in, the ring itself) fits under it.
    QRectF ring(disc.left() + line / 2, disc.top(), disc.width() - line, disc.height() - line);
    if (latched) {
        ring.translate(0, line);
    } else {
        painter.setBrush(t.keyDrop);
        painter.drawEllipse(ring.translated(0, line));
    }
    painter.setBrush(t.keyRing);
    painter.drawEllipse(ring);
    RailDisc out;
    out.face = ring.adjusted(line, line, -line, -line);
    QPainterPath face;
    face.addEllipse(out.face);
    if (latched) {
        out.fill = t.keyLatched;
        painter.fillPath(face, out.fill);
        QColor clear = t.keyLatchedShadow;
        clear.setAlpha(0);
        QLinearGradient shade(out.face.topLeft(), out.face.topLeft() + QPointF(0, 5));
        shade.setColorAt(0, t.keyLatchedShadow);
        shade.setColorAt(1, clear);
        painter.fillPath(face, shade);
    } else {
        QLinearGradient bevel(out.face.topLeft(), out.face.bottomLeft());
        bevel.setColorAt(0, t.keyTop);
        bevel.setColorAt(1, t.keyBottom);
        out.fill = bevel;
        painter.fillPath(face, out.fill);
        painter.fillPath(face.subtracted(face.translated(0, line)), t.keyHighlight);
    }
    if (hovered) painter.fillPath(face, t.hover);
    if (lit) {
        const QRectF led = snapToDevicePixels(painter, QRectF(out.face.center().x() - 5, out.face.top() + 4, 10, 2));
        QColor halo = t.keyLed;
        halo.setAlphaF(0.45);
        QColor gone = halo;
        gone.setAlpha(0);
        QRadialGradient glow(led.center(), 8);
        glow.setColorAt(0, halo);
        glow.setColorAt(1, gone);
        painter.fillPath(face, glow);
        painter.setBrush(t.keyLed);
        painter.drawRoundedRect(led, 1, 1);
    }
    painter.restore();
    return out;
}

} // namespace

RailButton::RailButton(Glyph glyph, QWidget *parent) : QAbstractButton(parent), m_glyph(glyph) {
    setObjectName(QStringLiteral("railButton"));
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setProperty("bezelRadius", kDisc / 2);
    QFont small = font();
    small.setPixelSize(12);
    small.setWeight(QFont::Medium);
    setFont(small);
}

void RailButton::setGlyph(Glyph glyph) {
    if (glyph == m_glyph) return;
    m_glyph = glyph;
    update();
}

void RailButton::setLabel(const QString &label) {
    if (label == m_label) return;
    m_label = label;
    update();
}

QRect RailButton::discRect() const { return QRect((width() - kDisc) / 2, 0, kDisc, kDisc); }

QSize RailButton::sizeHint() const { return {StageRail::kWidth, kDisc + 4 + 16}; }

void RailButton::paintEvent(QPaintEvent *) {
    const EditorTheme &theme = editorTheme();
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    // On reads from across the stage: held in, with its LED lit. A press
    // holds it in while the button is down.
    const bool hovered = underMouse() && !isDown();
    const RailDisc disc = paintRailDisc(painter, discRect(), isChecked() || isDown(), hovered, isChecked());
    // A device switched off (the plain card) keeps its name and its
    // glyph, in secondary ink with a slash through it, as the menu says
    // "Laptop · Off".
    const bool device = m_glyph == Glyph::Phone || m_glyph == Glyph::Laptop;
    const bool off = device && isCheckable() && !isChecked();
    const QColor ink = off ? theme.textSecondary : theme.railGlyph;
    // The glyph sits on the face, so it goes down with the key.
    const QPointF origin = disc.face.center() - QPointF(12, 12);
    painter.save();
    painter.translate(origin);
    painter.setPen(QPen(ink, 1.7, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
    painter.setBrush(Qt::NoBrush);
    switch (m_glyph) {
    case Glyph::Chat: {
        // The speech bubble on a 24-point grid: a rounded bubble, its tail
        // at the lower left; filled while the chat is open.
        QPainterPath path;
        path.moveTo(7.5, 4.5); path.lineTo(16.5, 4.5);
        path.quadTo(20.5, 4.5, 20.5, 8.5); path.lineTo(20.5, 12.5);
        path.quadTo(20.5, 16.5, 16.5, 16.5); path.lineTo(11.5, 16.5);
        path.lineTo(6.5, 20); path.lineTo(7.3, 16.5);
        path.quadTo(3.5, 16.3, 3.5, 12.5); path.lineTo(3.5, 8.5);
        path.quadTo(3.5, 4.5, 7.5, 4.5);
        path.closeSubpath();
        painter.setBrush(isChecked() ? QBrush(ink) : QBrush(Qt::NoBrush));
        painter.drawPath(path);
        break;
    }
    case Glyph::Phone:
        // A phone's outline and its camera.
        painter.drawRoundedRect(QRectF(7, 3, 10, 18), 2.5, 2.5);
        painter.drawLine(QPointF(10.8, 5.8), QPointF(13.2, 5.8));
        break;
    case Glyph::Laptop:
        // A laptop's lid, and its wider base under it.
        painter.drawRoundedRect(QRectF(5, 5.5, 14, 10), 1.5, 1.5);
        painter.drawLine(QPointF(3, 18.5), QPointF(21, 18.5));
        break;
    case Glyph::SafeZones: {
        // A frame's corners, and the dashed line inside them.
        const auto corner = [&painter](QPointF at, double dx, double dy) {
            painter.drawPolyline(QPolygonF{at + QPointF(0, 4 * dy), at, at + QPointF(4 * dx, 0)});
        };
        corner({4, 4}, 1, 1);
        corner({20, 4}, -1, 1);
        corner({4, 20}, 1, -1);
        corner({20, 20}, -1, -1);
        QPen dashed(ink, 1.4, Qt::CustomDashLine, Qt::FlatCap);
        dashed.setDashPattern({2.0, 2.0});
        painter.setPen(dashed);
        painter.drawRect(QRectF(8, 8, 8, 8));
        break;
    }
    }
    if (off) {
        // The slash, over a gap cut in the face's own shading (and its
        // hover) so it reads across the glyph.
        QBrush gap = disc.fill;
        gap.setTransform(QTransform::fromTranslate(-origin.x(), -origin.y()));
        const QLineF slash(QPointF(4.5, 19.5), QPointF(19.5, 4.5));
        painter.setPen(QPen(gap, 3.5, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(slash);
        if (hovered) {
            painter.setPen(QPen(theme.hover, 3.5, Qt::SolidLine, Qt::RoundCap));
            painter.drawLine(slash);
        }
        painter.setPen(QPen(ink, 1.5, Qt::SolidLine, Qt::RoundCap));
        painter.drawLine(slash);
    }
    painter.restore();
    if (!m_label.isEmpty()) {
        painter.setPen(theme.railLabel);
        painter.setFont(font());
        painter.drawText(QRectF(0, discRect().bottom() + 4, width(), 16), Qt::AlignHCenter | Qt::AlignTop, m_label);
    }
}

// ---- StageRail ------------------------------------------------------------

StageRail::StageRail(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("stageRail"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(12);
    m_meter = new VuMeter;
    m_meter->setFixedSize(kWidth, kMeterHeight);
    layout->addWidget(m_meter, 0, Qt::AlignHCenter);
    layout->addStretch(1);
    m_device = new RailButton(RailButton::Glyph::Phone);
    m_device->setObjectName(QStringLiteral("railDevice"));
    m_device->setCheckable(true);
    m_device->setCheckedByOwner(true);
    m_device->setLabel(QStringLiteral("Phone"));
    m_safeZones = new RailButton(RailButton::Glyph::SafeZones);
    m_safeZones->setObjectName(QStringLiteral("railSafeZones"));
    m_safeZones->setCheckable(true);
    m_safeZones->setLabel(QStringLiteral("Safe"));
    m_chat = new RailButton(RailButton::Glyph::Chat);
    m_chat->setCheckable(true);
    for (RailButton *button : {m_device, m_safeZones, m_chat}) {
        button->setFixedSize(button->sizeHint());
        layout->addWidget(button, 0, Qt::AlignHCenter);
    }
}

bool StageRail::event(QEvent *event) {
    const bool handled = QWidget::event(event);
    if (event->type() == QEvent::Resize || event->type() == QEvent::LayoutRequest) fitMeter();
    return handled;
}

void StageRail::fitMeter() {
    // The buttons shown, stacked with the layout's spacing; the meter only
    // where it fits over them with kMeterGap to spare.
    int buttons = 0;
    int count = 0;
    for (const RailButton *button : {m_device, m_safeZones, m_chat}) {
        if (button->isHidden()) continue;
        buttons += button->height();
        ++count;
    }
    const int stack = buttons + std::max(0, count - 1) * layout()->spacing();
    m_meter->setVisible(height() >= stack + kMeterGap + kMeterHeight);
}

} // namespace kerf
