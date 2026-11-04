#include "components.h"

#include "icons.h"

#include <QAction>
#include <QApplication>
#include <QCache>
#include <QCursor>
#include <QEnterEvent>
#include <QFocusEvent>
#include <QFontMetricsF>
#include <QHash>
#include <QKeyEvent>
#include <QLinearGradient>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPointer>
#include <QRadialGradient>
#include <QRegularExpression>
#include <QScrollBar>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>
#include <utility>
#include <vector>

#ifdef Q_OS_MACOS
#include <CoreFoundation/CoreFoundation.h>
#endif

namespace kerf::ui {

using namespace kerf::tokens;

// ================================================================ motion

namespace {

int g_motionOverride = -1;

// The system's Reduce motion switch. It is read again when the last read is
// more than a second old: animations ask on every start, and a read is a
// preferences round trip, so this keeps it cheap while a change made in
// System Settings still applies within a second, without a restart.
bool systemReducedMotion() {
#ifdef Q_OS_MACOS
    static QElapsedTimer sinceRead;
    static bool reduced = false;
    if (!sinceRead.isValid() || sinceRead.elapsed() > 1000) {
        const CFStringRef domain = CFSTR("com.apple.universalaccess");
        CFPreferencesAppSynchronize(domain);
        bool on = false;
        if (CFPropertyListRef value = CFPreferencesCopyAppValue(CFSTR("reduceMotion"), domain)) {
            if (CFGetTypeID(value) == CFBooleanGetTypeID()) on = CFBooleanGetValue(static_cast<CFBooleanRef>(value));
            CFRelease(value);
        }
        reduced = on;
        sinceRead.start();
    }
    return reduced;
#else
    return false;
#endif
}

} // namespace

bool reducedMotion() {
    if (g_motionOverride >= 0) return g_motionOverride == 1;
    const QByteArray env = qgetenv("KERF_REDUCE_MOTION");
    if (!env.isEmpty()) return env != "0";
    return systemReducedMotion();
}

void setReducedMotionOverride(int state) { g_motionOverride = state < 0 ? -1 : (state ? 1 : 0); }

int motionDuration(int ms) { return reducedMotion() ? 0 : ms; }

// ================================================================ painting helpers

QColor mix(const QColor &from, const QColor &to, qreal t) {
    t = std::clamp<qreal>(t, 0, 1);
    auto lerp = [t](float a, float b) { return a + (b - a) * float(t); };
    QColor out;
    out.setRgbF(lerp(from.redF(), to.redF()), lerp(from.greenF(), to.greenF()), lerp(from.blueF(), to.blueF()),
                lerp(from.alphaF(), to.alphaF()));
    return out;
}

int snapWidth(const QFontMetricsF &metrics, const QString &text) {
    return int(std::ceil(metrics.horizontalAdvance(text) - 1e-6));
}

int snapWidth(const TypeStyle &style, const QString &text) { return snapWidth(QFontMetricsF(tokens::font(style)), text); }

void paintInsetLine(QPainter &painter, const QRectF &rect, qreal radius, const QColor &color, qreal width) {
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.setBrush(Qt::NoBrush);
    painter.setPen(QPen(color, width));
    const qreal h = width / 2;
    const qreal r = qMax<qreal>(0, radius - h);
    painter.drawRoundedRect(rect.adjusted(h, h, -h, -h), r, r);
    painter.restore();
}

void paintTopHighlight(QPainter &painter, const QRectF &rect, qreal radius, const QColor &color) {
    QPainterPath shape;
    shape.addRoundedRect(rect, radius, radius);
    QPainterPath shifted;
    shifted.addRoundedRect(rect.translated(0, 1), radius, radius);
    painter.save();
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.fillPath(shape.subtracted(shifted), color);
    painter.restore();
}

void paintFocusRing(QPainter &painter, const QRectF &rect, qreal radius) {
    // The design draws the 2 pt ring 1 pt outside the control. A widget cannot
    // paint outside itself, so the ring lies just inside its edge instead.
    paintInsetLine(painter, rect, radius, color(accent), focusWidth);
}

QMargins shadowMargins(const Shadow &shadow) {
    const int blur = int(std::ceil(shadow.blur));
    return QMargins(qMax(0, blur - int(shadow.dx)), qMax(0, blur - int(shadow.dy)), blur + int(shadow.dx),
                    blur + int(shadow.dy));
}

namespace {

// One box blur pass along rows (horizontal) or columns of an alpha buffer.
void boxBlur(std::vector<float> &alpha, int width, int height, int radius, bool horizontal) {
    std::vector<float> line(size_t(horizontal ? width : height));
    const int length = horizontal ? width : height;
    const int lines = horizontal ? height : width;
    const float scale = 1.0f / float(2 * radius + 1);
    for (int l = 0; l < lines; ++l) {
        auto at = [&](int i) -> float & {
            return horizontal ? alpha[size_t(l) * size_t(width) + size_t(i)] : alpha[size_t(i) * size_t(width) + size_t(l)];
        };
        float sum = 0;
        for (int i = -radius; i <= radius; ++i) sum += (i >= 0 && i < length) ? at(i) : 0.0f;
        for (int i = 0; i < length; ++i) {
            line[size_t(i)] = sum * scale;
            const int add = i + radius + 1, drop = i - radius;
            if (add < length) sum += at(add);
            if (drop >= 0) sum -= at(drop);
        }
        for (int i = 0; i < length; ++i) at(i) = line[size_t(i)];
    }
}

// The shadow of a rounded rect of `size`, blurred, in its colour, with `blur`
// of room on every side. Cached: popovers and tips ask for the same few sizes.
QImage shadowImage(QSize size, qreal radius, const Shadow &shadow) {
    static QCache<QString, QImage> cache(32);
    const QString key = QStringLiteral("%1x%2/%3/%4/%5").arg(size.width()).arg(size.height()).arg(radius).arg(shadow.blur).arg(shadow.color);
    if (const QImage *hit = cache.object(key)) return *hit;
    const int blur = int(std::ceil(shadow.blur));
    const int w = size.width() + 2 * blur, h = size.height() + 2 * blur;
    QImage mask(w, h, QImage::Format_Alpha8);
    mask.fill(0);
    {
        QPainter p(&mask);
        p.setRenderHint(QPainter::Antialiasing, true);
        p.setPen(Qt::NoPen);
        p.setBrush(Qt::black);
        p.drawRoundedRect(QRectF(blur, blur, size.width(), size.height()), radius, radius);
    }
    std::vector<float> alpha(size_t(w) * size_t(h));
    for (int y = 0; y < h; ++y) {
        const uchar *row = mask.constScanLine(y);
        for (int x = 0; x < w; ++x) alpha[size_t(y) * size_t(w) + size_t(x)] = row[x] / 255.0f;
    }
    // A CSS blur radius B is a Gaussian of sigma B/2; three box passes of
    // radius sigma come close to it.
    const int box = qMax(1, int(std::lround(shadow.blur / 2)));
    for (int pass = 0; pass < 3; ++pass) {
        boxBlur(alpha, w, h, box, true);
        boxBlur(alpha, w, h, box, false);
    }
    QImage out(w, h, QImage::Format_ARGB32_Premultiplied);
    const float a = qAlpha(shadow.color) / 255.0f;
    for (int y = 0; y < h; ++y) {
        auto *row = reinterpret_cast<QRgb *>(out.scanLine(y));
        for (int x = 0; x < w; ++x) {
            const float k = std::clamp(alpha[size_t(y) * size_t(w) + size_t(x)] * a, 0.0f, 1.0f);
            row[x] = qRgba(int(qRed(shadow.color) * k + 0.5f), int(qGreen(shadow.color) * k + 0.5f),
                           int(qBlue(shadow.color) * k + 0.5f), int(255 * k + 0.5f));
        }
    }
    cache.insert(key, new QImage(out));
    return out;
}

} // namespace

void paintShadow(QPainter &painter, const QRectF &rect, qreal radius, const Shadow &shadow) {
    const QSize size = rect.size().toSize();
    if (size.isEmpty()) return;
    const QImage image = shadowImage(size, radius, shadow);
    const int blur = int(std::ceil(shadow.blur));
    painter.drawImage(rect.topLeft() + QPointF(shadow.dx - blur, shadow.dy - blur), image);
}

// ================================================================ tooltips

namespace {

const char kTipProperty[] = "kerfTip";
const char kTipKeyProperty[] = "kerfTipKey";

// The tip itself (#tip): 24 tall, pad 0 8, radius 6, card + pop + inset
// line2, gap 8; the name in UI 12/16 ink, the shortcut in mono 11/16 ink3.
class TipWindow final : public QWidget {
public:
    TipWindow()
        : QWidget(nullptr, Qt::ToolTip | Qt::FramelessWindowHint | Qt::WindowDoesNotAcceptFocus |
                               Qt::NoDropShadowWindowHint) {
        setAttribute(Qt::WA_TranslucentBackground);
        setAttribute(Qt::WA_ShowWithoutActivating);
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setObjectName(QStringLiteral("kerfTip"));
        m_fade = new QVariantAnimation(this);
        m_fade->setEasingCurve(easing());
        connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &v) {
            setWindowOpacity(v.toReal());
        });
        connect(m_fade, &QVariantAnimation::finished, this, [this] {
            if (m_closing) QWidget::hide();
        });
    }

    void setTip(const QString &name, const QString &key) {
        m_name = name;
        m_key = key;
        update();
    }
    QString name() const { return m_name; }
    QString key() const { return m_key; }

    QSize cardSize() const {
        int w = space::s8 + snapWidth(control, m_name) + space::s8;
        if (!m_key.isEmpty()) w += space::s8 + snapWidth(monoSmall, m_key);
        return QSize(w, TooltipManager::tipHeight);
    }

    void showAt(QPoint cardTopLeft, bool fade) {
        const QMargins m = shadowMargins();
        setGeometry(QRect(cardTopLeft - QPoint(m.left(), m.top()), cardSize().grownBy(m)));
        m_closing = false;
        m_fade->stop();
        const int duration = fade ? motionDuration(motion::fade) : 0;
        if (duration > 0) {
            setWindowOpacity(0);
            QWidget::show();
            m_fade->setDuration(duration);
            m_fade->setStartValue(0.0);
            m_fade->setEndValue(1.0);
            m_fade->start();
        } else {
            setWindowOpacity(1);
            QWidget::show();
        }
    }

    void fadeOut() {
        if (!isVisible()) return;
        m_fade->stop();
        const int duration = motionDuration(motion::fade);
        if (duration <= 0) {
            QWidget::hide();
            return;
        }
        m_closing = true;
        m_fade->setDuration(duration);
        m_fade->setStartValue(windowOpacity());
        m_fade->setEndValue(0.0);
        m_fade->start();
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF card = QRectF(rect()).marginsRemoved(QMarginsF(shadowMargins()));
        paintShadow(p, card, radius::control);
        p.setPen(Qt::NoPen);
        p.setBrush(color(tokens::card));
        p.drawRoundedRect(card, radius::control, radius::control);
        paintInsetLine(p, card, radius::control, color(line2));
        qreal x = card.left() + space::s8;
        p.setFont(tokens::font(control));
        p.setPen(color(ink));
        const int nameWidth = snapWidth(control, m_name);
        p.drawText(QRectF(x, card.top(), nameWidth + 1, card.height()), Qt::AlignLeft | Qt::AlignVCenter, m_name);
        if (!m_key.isEmpty()) {
            x += nameWidth + space::s8;
            p.setFont(tokens::font(monoSmall));
            p.setPen(color(ink3));
            p.drawText(QRectF(x, card.top(), snapWidth(monoSmall, m_key) + 1, card.height()),
                       Qt::AlignLeft | Qt::AlignVCenter, m_key);
        }
    }

private:
    QString m_name;
    QString m_key;
    QVariantAnimation *m_fade = nullptr;
    bool m_closing = false;
};

QPointer<TooltipManager> g_tooltips;

QWidget *closestTipped(QWidget *widget) {
    for (QWidget *w = widget; w; w = w->parentWidget()) {
        if (!w->property(kTipProperty).toString().isEmpty()) return w;
        if (w->isWindow()) break;
    }
    return nullptr;
}

// The deepest widget under the pointer that `event` reports to `widget`, or
// `widget` itself when the event carries no point (an Enter can be sent as
// a bare QEvent).
QWidget *deepestAt(QWidget *widget, const QEvent *event) {
    if (!event->isSinglePointEvent()) return widget;
    QWidget *child = widget->childAt(static_cast<const QSinglePointEvent *>(event)->position().toPoint());
    return child ? child : widget;
}

// Whether Qt's own tooltip for `widget` gives way to a kit tip: the closest
// widget, from `widget` up, that has either kind of tip has a kit one. A
// descendant with its own setToolTip() inside a kit-tipped container keeps
// its Qt tooltip.
bool kitTipOwnsToolTip(QWidget *widget) {
    for (QWidget *w = widget; w; w = w->parentWidget()) {
        if (!w->property(kTipProperty).toString().isEmpty()) return true;
        if (!w->toolTip().isEmpty()) return false;
        if (w->isWindow()) break;
    }
    return false;
}

} // namespace

QString keyText(const QKeySequence &keys) {
    return keys.toString(QKeySequence::NativeText).replace(QChar(0x21B5), QChar(0x21A9));
}

void setTip(QWidget *widget, const QString &name, const QString &shortcut) {
    if (!widget) return;
    widget->setProperty(kTipProperty, name);
    widget->setProperty(kTipKeyProperty, shortcut);
    if (!name.isEmpty()) widget->setToolTip(QString());
    if (TooltipManager *manager = TooltipManager::instance(); manager && manager->target() == widget) {
        if (name.isEmpty()) manager->hide();
        else if (manager->isShowing()) manager->hover(widget);
    }
}

QString tipName(const QWidget *widget) { return widget ? widget->property(kTipProperty).toString() : QString(); }
QString tipShortcut(const QWidget *widget) {
    return widget ? widget->property(kTipKeyProperty).toString() : QString();
}

TooltipManager *TooltipManager::instance() {
    if (!g_tooltips && qApp) {
        g_tooltips = new TooltipManager;
        g_tooltips->setParent(qApp);
        qApp->installEventFilter(g_tooltips);
    }
    return g_tooltips;
}

TooltipManager::TooltipManager() {
    m_elapsed.start();
    m_timer.setSingleShot(true);
    connect(&m_timer, &QTimer::timeout, this, &TooltipManager::tick);
}

void TooltipManager::setClock(std::function<qint64()> clock) {
    m_clock = std::move(clock);
    m_lastHidden = now() - 1'000'000;
}

qint64 TooltipManager::now() const { return m_clock ? m_clock() : m_elapsed.elapsed(); }

QWidget *TooltipManager::tipWindow() const { return m_window; }

QString TooltipManager::shownName() const {
    auto *tip = static_cast<TipWindow *>(m_window.data());
    return m_showing && tip ? tip->name() : QString();
}

QString TooltipManager::shownShortcut() const {
    auto *tip = static_cast<TipWindow *>(m_window.data());
    return m_showing && tip ? tip->key() : QString();
}

QRect TooltipManager::placement(const QRectF &control, QSize tip, QSize window) {
    int left = int(std::lround(control.left() + control.width() / 2 - tip.width() / 2.0));
    left = qMax(edge, qMin(window.width() - edge - tip.width(), left));
    const bool above = control.bottom() > flipBelow && control.top() > flipAbove;
    const int top = int(std::lround(above ? control.top() - gap - tipHeight : control.bottom() + gap));
    return QRect(QPoint(left, top), tip);
}

void TooltipManager::hover(QWidget *widget) {
    QWidget *tipped = closestTipped(widget);
    if (!tipped) {
        hide();
        return;
    }
    if (tipped == m_target) return;
    const bool quick = m_showing || now() - m_lastHidden < quickMs;
    m_target = tipped;
    if (quick) {
        show(tipped);
        return;
    }
    if (m_window) static_cast<TipWindow *>(m_window.data())->fadeOut();
    m_pendingSince = now();
    m_timer.start(delayMs);
}

void TooltipManager::tick() {
    if (!m_target || m_pendingSince < 0) return;
    const qint64 waited = now() - m_pendingSince;
    if (waited >= delayMs) show(m_target);
    else m_timer.start(int(delayMs - waited));
}

void TooltipManager::show(QWidget *widget) {
    m_timer.stop();
    m_pendingSince = -1;
    if (!widget) return;
    if (!m_window) {
        auto *created = new TipWindow;
        m_window = created;
        connect(this, &QObject::destroyed, created, [created] { delete created; });
    }
    auto *tip = static_cast<TipWindow *>(m_window.data());
    tip->setTip(tipName(widget), tipShortcut(widget));
    QWidget *window = widget->window();
    const QRectF control(QPointF(widget->mapTo(window, QPoint(0, 0))), QSizeF(widget->size()));
    const QRect card = placement(control, tip->cardSize(), window->size());
    tip->showAt(window->mapToGlobal(card.topLeft()), !m_showing);
    m_showing = true;
}

void TooltipManager::hide() {
    m_timer.stop();
    m_pendingSince = -1;
    if (m_showing) m_lastHidden = now();
    m_showing = false;
    m_target = nullptr;
    if (m_window) static_cast<TipWindow *>(m_window.data())->fadeOut();
}

bool TooltipManager::eventFilter(QObject *watched, QEvent *event) {
    switch (event->type()) {
    case QEvent::Enter:
        // Enter reaches each widget entered, outermost first, and comes
        // again when a window under the pointer changes; what counts is the
        // deepest widget at the point, as the mockup's mouseover target is.
        if (auto *widget = qobject_cast<QWidget *>(watched); widget && widget != m_window)
            hover(deepestAt(widget, event));
        break;
    case QEvent::Leave:
        // Leaving a tipped child for its tipped parent sends the parent no
        // Enter, so look at what is under the pointer now, as the mockup's
        // mouseover re-check does: the parent's tip takes over.
        if (watched == m_target) {
            QWidget *under = QApplication::widgetAt(QCursor::pos());
            QWidget *next = under ? closestTipped(under) : nullptr;
            if (next && next != m_target && !m_target->isAncestorOf(next)) hover(next);
            else hide();
        }
        break;
    case QEvent::MouseMove:
    case QEvent::HoverMove:
        // Moving over a control re-checks it, so a tip hidden by a press
        // comes back (once the button is up) without leaving the control,
        // and a parent's tip resumes when the pointer is back on the parent.
        // Both events also reach the parents of the widget under the
        // pointer, so the check starts from the deepest widget at the point.
        if (auto *widget = qobject_cast<QWidget *>(watched); widget && widget != m_window) {
            if (!event->isSinglePointEvent() ||
                static_cast<QSinglePointEvent *>(event)->buttons() != Qt::NoButton ||
                QApplication::mouseButtons() != Qt::NoButton)
                break;
            QWidget *under = deepestAt(widget, event);
            if (closestTipped(under) != m_target) hover(under);
        }
        break;
    case QEvent::MouseButtonPress:
    case QEvent::MouseButtonDblClick:
        if (m_target || m_showing) hide();
        break;
    case QEvent::Hide:
    case QEvent::WindowDeactivate:
        if (m_target && (watched == m_target || watched == m_target->window())) hide();
        break;
    case QEvent::ToolTip:
        // The kit's tip replaces Qt's wherever it is the closest tip.
        if (auto *widget = qobject_cast<QWidget *>(watched); widget && kitTipOwnsToolTip(widget)) return true;
        break;
    default: break;
    }
    return false;
}

// ================================================================ KitButton

KitButton::KitButton(QWidget *parent) : QAbstractButton(parent) {
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
}

void KitButton::setTip(const QString &name, const QString &shortcut) { ui::setTip(this, name, shortcut); }

void KitButton::fadeTo(qreal target) {
    const int duration = isVisible() ? motionDuration(motion::hover) : 0;
    if (duration <= 0) {
        if (m_fade) m_fade->stop();
        m_hover = target;
        update();
        return;
    }
    if (!m_fade) {
        m_fade = new QVariantAnimation(this);
        m_fade->setEasingCurve(easing());
        connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &v) {
            m_hover = v.toReal();
            update();
        });
    }
    m_fade->stop();
    m_fade->setDuration(duration);
    m_fade->setStartValue(m_hover);
    m_fade->setEndValue(target);
    m_fade->start();
}

void KitButton::enterEvent(QEnterEvent *event) {
    QAbstractButton::enterEvent(event);
    if (!isEnabled()) return;
    m_hovered = true;
    fadeTo(1);
}

void KitButton::leaveEvent(QEvent *event) {
    QAbstractButton::leaveEvent(event);
    m_hovered = false;
    fadeTo(0);
}

void KitButton::focusInEvent(QFocusEvent *event) {
    QAbstractButton::focusInEvent(event);
    m_focusVisible = event->reason() == Qt::TabFocusReason || event->reason() == Qt::BacktabFocusReason;
    update();
}

void KitButton::focusOutEvent(QFocusEvent *event) {
    QAbstractButton::focusOutEvent(event);
    m_focusVisible = false;
    update();
}

void KitButton::changeEvent(QEvent *event) {
    QAbstractButton::changeEvent(event);
    if (event->type() == QEvent::EnabledChange && !isEnabled()) {
        m_hovered = false;
        if (m_fade) m_fade->stop();
        m_hover = 0;
        update();
    }
}

// ================================================================ IconButton

IconButton::IconButton(const QString &icon, Size size, QWidget *parent)
    : KitButton(parent), m_icon(icon), m_size(size) {
    setFixedSize(sizeHint());
}

void IconButton::setIconName(const QString &icon) {
    m_icon = icon;
    update();
}

void IconButton::setButtonSize(Size size) {
    m_size = size;
    setFixedSize(sizeHint());
    update();
}

void IconButton::setIconPx(qreal px) {
    m_iconPx = px;
    update();
}

qreal IconButton::iconPx() const { return m_iconPx > 0 ? m_iconPx : (m_size == Size::S20 ? 14 : 16); }

void IconButton::setOn(bool on) {
    setCheckable(true);
    setChecked(on);
    update();
}

QColor IconButton::iconColor() const {
    if (!isEnabled()) return color(dis);
    if (isChecked() || m_inkAtRest) return color(ink);
    return mix(color(ink2), color(ink), hoverProgress());
}

void IconButton::setInkAtRest(bool ink) {
    if (ink == m_inkAtRest) return;
    m_inkAtRest = ink;
    update();
}

QSize IconButton::sizeHint() const { return QSize(int(m_size), int(m_size)); }

void IconButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    if (isChecked()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(on));
        p.drawRoundedRect(r, radius::control, radius::control);
        paintInsetLine(p, r, radius::control, color(line2));
    } else if (isEnabled() && hoverProgress() > 0) {
        QColor fill = color(raised);
        fill.setAlphaF(float(hoverProgress()));
        p.setPen(Qt::NoPen);
        p.setBrush(fill);
        p.drawRoundedRect(r, radius::control, radius::control);
    }
    icons::paint(p, m_icon, r, iconColor(), iconPx());
    if (focusVisible()) paintFocusRing(p, r, radius::control);
}

// ================================================================ Separator

Separator::Separator(int margin, QWidget *parent) : QWidget(parent), m_margin(margin) {
    setFixedSize(1 + 2 * m_margin, 16);
}

void Separator::setMargin(int margin) {
    m_margin = margin;
    setFixedSize(1 + 2 * m_margin, 16);
    update();
}

void Separator::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.fillRect(QRect(m_margin, (height() - 16) / 2, 1, 16), color(line));
}

// ================================================================ SegmentedControl

namespace {

// One segment of a SegmentedControl: it paints only its label (the parent
// paints the track and the sliding fill behind it).
class SegmentButton final : public KitButton {
public:
    SegmentButton(QWidget *parent, QString text, QString icon, bool tail)
        : KitButton(parent), m_text(std::move(text)), m_icon(std::move(icon)), m_tail(tail) {
        setFocusPolicy(Qt::NoFocus);
        setText(m_text);
    }
    QString label() const { return m_text; }
    // " 12" after the label, or empty.
    QString countText() const { return m_count < 0 ? QString() : QStringLiteral(" %1").arg(m_count); }
    int count() const { return m_count; }
    void setCount(int count) {
        m_count = count;
        update();
    }
    bool isIcon() const { return !m_icon.isEmpty(); }
    bool isTail() const { return m_tail; }
    void setOn(bool on) {
        m_on = on;
        update();
    }
    bool isOn() const { return m_on; }
    QColor labelColor() const {
        if (!isEnabled()) return color(dis);
        if (m_on) return color(ink);
        return mix(color(ink2), color(ink), hoverProgress());
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QColor c = labelColor();
        if (m_tail) {
            icons::paint(p, QStringLiteral("g-chev"), rect(), c, 14);
        } else if (isIcon()) {
            icons::paint(p, m_icon, rect(), c, 16);
        } else if (m_count < 0) {
            p.setFont(tokens::font(m_on ? controlStrong : control));
            p.setPen(c);
            p.drawText(rect(), Qt::AlignCenter, m_text);
        } else {
            // The label then its count (ink3, weight 400), as one centred run.
            const QFont label = tokens::font(m_on ? controlStrong : control);
            const QFont number = tokens::font(control);
            const QString count = countText();
            const qreal nameWidth = QFontMetricsF(label).horizontalAdvance(m_text);
            const qreal countWidth = QFontMetricsF(number).horizontalAdvance(count);
            const qreal left = (width() - nameWidth - countWidth) / 2.0;
            p.setFont(label);
            p.setPen(c);
            p.drawText(QRectF(left, 0, nameWidth + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, m_text);
            p.setFont(number);
            p.setPen(isEnabled() ? color(ink3) : color(dis));
            p.drawText(QRectF(left + nameWidth, 0, countWidth + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, count);
        }
    }

private:
    QString m_text;
    QString m_icon;
    bool m_tail;
    bool m_on = false;
    int m_count = -1;
};

SegmentButton *asSegment(QWidget *w) { return static_cast<SegmentButton *>(w); }

} // namespace

SegmentedControl::SegmentedControl(QWidget *parent) : QWidget(parent) {
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    m_slide = new QVariantAnimation(this);
    m_slide->setEasingCurve(easing());
    connect(m_slide, &QVariantAnimation::valueChanged, this, [this](const QVariant &v) {
        m_fill = v.toRectF();
        update();
    });
}

int SegmentedControl::addSegment(const QString &text, const QString &tip, const QString &shortcut) {
    auto *button = new SegmentButton(this, text, {}, false);
    const int index = int(m_segments.size());
    m_segments << button;
    connect(button, &QAbstractButton::clicked, this, [this, index] {
        setCurrentIndex(index);
        emit activated(index);
    });
    if (!tip.isEmpty()) button->setTip(tip, shortcut);
    relayout();
    return index;
}

int SegmentedControl::addIconSegment(const QString &icon, const QString &tip, const QString &shortcut) {
    auto *button = new SegmentButton(this, {}, icon, false);
    const int index = int(m_segments.size());
    m_segments << button;
    connect(button, &QAbstractButton::clicked, this, [this, index] {
        setCurrentIndex(index);
        emit activated(index);
    });
    if (!tip.isEmpty()) button->setTip(tip, shortcut);
    relayout();
    return index;
}

int SegmentedControl::count() const { return int(m_segments.size()); }

void SegmentedControl::setSegmentEnabled(int index, bool enabled) {
    if (index >= 0 && index < count()) m_segments[index]->setEnabled(enabled);
}

void SegmentedControl::setSegmentTip(int index, const QString &tip, const QString &shortcut) {
    if (index >= 0 && index < count()) m_segments[index]->setTip(tip, shortcut);
}

void SegmentedControl::setSegmentData(int index, const QVariant &data) {
    if (index >= 0 && index < count()) m_segments[index]->setProperty("kerfSegmentData", data);
}

QVariant SegmentedControl::segmentData(int index) const {
    return index >= 0 && index < count() ? m_segments[index]->property("kerfSegmentData") : QVariant();
}

QString SegmentedControl::segmentText(int index) const {
    return index >= 0 && index < count() ? asSegment(m_segments[index])->label() : QString();
}

QWidget *SegmentedControl::segment(int index) const {
    return index >= 0 && index < count() ? m_segments[index] : nullptr;
}

void SegmentedControl::setTrackHeight(int height) {
    m_trackHeight = height;
    relayout();
}

void SegmentedControl::setIconSegmentWidth(int width) {
    m_iconWidth = width;
    relayout();
}

void SegmentedControl::setTextSegmentWidth(int width) {
    m_textWidth = std::max(0, width);
    relayout();
}

void SegmentedControl::setSegmentCount(int index, int count) {
    if (index < 0 || index >= this->count()) return;
    asSegment(m_segments[index])->setCount(count < 0 ? -1 : count);
    relayout();
}

int SegmentedControl::segmentCount(int index) const {
    return index >= 0 && index < count() ? asSegment(m_segments[index])->count() : -1;
}

int SegmentedControl::segmentWidth(int index) const {
    auto *s = asSegment(m_segments[index]);
    if (s->isIcon()) return m_iconWidth;
    if (m_textWidth > 0) return m_textWidth;
    // The label's width at weight 500, so the on segment never grows; a
    // count adds its own at 400.
    return space::s8 + snapWidth(controlStrong, s->label()) + (s->count() < 0 ? 0 : snapWidth(control, s->countText())) + space::s8;
}

void SegmentedControl::setMenuTail(bool on, const QString &tip, const QString &shortcut) {
    if (on && !m_tail) {
        m_tail = new SegmentButton(this, {}, {}, true);
        connect(m_tail, &QAbstractButton::clicked, this, &SegmentedControl::menuRequested);
    } else if (!on && m_tail) {
        delete m_tail;
        m_tail = nullptr;
    }
    if (m_tail) m_tail->setTip(tip, shortcut);
    relayout();
}

bool SegmentedControl::hasMenuTail() const { return m_tail != nullptr; }
QWidget *SegmentedControl::menuTail() const { return m_tail; }

void SegmentedControl::relayout() {
    const int inner = m_trackHeight - 4;
    int x = 2;
    for (int i = 0; i < count(); ++i) {
        const int w = segmentWidth(i);
        m_segments[i]->setGeometry(x, 2, w, inner);
        x += w;
    }
    if (m_tail) {
        m_tail->setGeometry(x, 2, 20, inner);
        x += 20;
    }
    setFixedSize(x + 2, m_trackHeight);
    if (m_slide->state() != QAbstractAnimation::Running) m_fill = targetFill();
    updateGeometry();
    update();
}

QSize SegmentedControl::sizeHint() const {
    int x = 2;
    for (int i = 0; i < count(); ++i) x += segmentWidth(i);
    if (m_tail) x += 20;
    return QSize(x + 2, m_trackHeight);
}

QRectF SegmentedControl::segmentRect(int index) const {
    return index >= 0 && index < count() ? QRectF(m_segments[index]->geometry()) : QRectF();
}

QRectF SegmentedControl::targetFill() const { return segmentRect(m_current); }

void SegmentedControl::setCurrentIndex(int index, bool animate) {
    if (index < -1 || index >= count() || index == m_current) return;
    if (index == -1) {
        // None chosen: no fill, every segment off.
        m_current = -1;
        for (KitButton *b : std::as_const(m_segments)) asSegment(b)->setOn(false);
        m_slide->stop();
        m_fill = QRectF();
        update();
        emit currentChanged(-1);
        return;
    }
    const int previous = m_current;
    m_current = index;
    for (int i = 0; i < count(); ++i) asSegment(m_segments[i])->setOn(i == index);
    const QRectF target = targetFill();
    const int duration = motionDuration(motion::segment);
    if (animate && previous >= 0 && !m_fill.isEmpty() && isVisible() && duration > 0) {
        m_slide->stop();
        m_slide->setDuration(duration);
        m_slide->setStartValue(m_fill);
        m_slide->setEndValue(target);
        m_slide->start();
    } else {
        m_slide->stop();
        m_fill = target;
        update();
    }
    emit currentChanged(index);
}

void SegmentedControl::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF track(0, 0, width(), m_trackHeight);
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(track, radius::control, radius::control);
    if (m_current >= 0 && !m_fill.isEmpty()) {
        p.setBrush(color(on));
        p.drawRoundedRect(m_fill, radius::small, radius::small);
        paintInsetLine(p, m_fill, radius::small, color(line2));
        paintTopHighlight(p, m_fill, radius::small, color(segmentHighlight));
    }
    if (m_focusVisible && hasFocus()) paintFocusRing(p, track, radius::control);
}

void SegmentedControl::keyPressEvent(QKeyEvent *event) {
    const int step = event->key() == Qt::Key_Left ? -1 : event->key() == Qt::Key_Right ? 1 : 0;
    if (!step) {
        QWidget::keyPressEvent(event);
        return;
    }
    for (int i = m_current + step; i >= 0 && i < count(); i += step) {
        if (!m_segments[i]->isEnabled()) continue;
        setCurrentIndex(i);
        emit activated(i);
        break;
    }
}

void SegmentedControl::focusInEvent(QFocusEvent *event) {
    QWidget::focusInEvent(event);
    m_focusVisible = event->reason() == Qt::TabFocusReason || event->reason() == Qt::BacktabFocusReason;
    update();
}

void SegmentedControl::focusOutEvent(QFocusEvent *event) {
    QWidget::focusOutEvent(event);
    m_focusVisible = false;
    update();
}

void SegmentedControl::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    if (m_slide->state() != QAbstractAnimation::Running) m_fill = targetFill();
}

// ================================================================ GhostButton

GhostButton::GhostButton(const QString &text, QWidget *parent) : KitButton(parent) {
    setText(text);
    setFixedSize(sizeHint());
}

void GhostButton::setMono(bool mono) {
    m_mono = mono;
    m_style = mono ? monoSmall : control;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setTypeStyle(const TypeStyle &style) {
    m_style = style;
    m_mono = style.mono;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setChevron(bool chevron) {
    m_chevron = chevron;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setIconName(const QString &icon) {
    m_icon = icon;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setControlHeight(int height) {
    m_height = height;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setHorizontalPadding(int padding) {
    m_padding = padding;
    setFixedSize(sizeHint());
    update();
}

void GhostButton::setMeta(bool meta) {
    m_meta = meta;
    m_style = meta ? tokens::meta : control;
    m_mono = false;
    m_height = meta ? height::h20 : 24;
    setFixedSize(sizeHint());
    update();
}

QColor GhostButton::textColor() const {
    if (!isEnabled()) return color(dis);
    return mix(color(m_meta ? ink3 : ink2), color(ink), hoverProgress());
}

QSize GhostButton::sizeHint() const {
    int w = m_padding;
    bool first = true;
    auto part = [&](int width) {
        if (!first) w += space::s4;
        w += width;
        first = false;
    };
    if (!m_icon.isEmpty()) part(16);
    if (!text().isEmpty()) part(snapWidth(m_style, text()));
    if (m_chevron) part(14);
    return QSize(w + m_padding, m_height);
}

void GhostButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    if (isEnabled() && hoverProgress() > 0) {
        QColor fill = color(raised);
        fill.setAlphaF(float(hoverProgress()));
        p.setPen(Qt::NoPen);
        p.setBrush(fill);
        p.drawRoundedRect(r, radius::control, radius::control);
    }
    const QColor c = textColor();
    qreal x = m_padding;
    if (!m_icon.isEmpty()) {
        icons::paint(p, m_icon, QRectF(x, 0, 16, height()), c, 16);
        x += 16 + space::s4;
    }
    if (!text().isEmpty()) {
        const int w = snapWidth(m_style, text());
        p.setFont(tokens::font(m_style));
        p.setPen(c);
        p.drawText(QRectF(x, 0, w + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
        x += w + space::s4;
    }
    if (m_chevron) icons::paint(p, QStringLiteral("g-chev"), QRectF(x, 0, 14, height()), c, 14);
    if (focusVisible()) paintFocusRing(p, r, radius::control);
}

// ================================================================ Button

Button::Button(const QString &text, Kind kind, QWidget *parent) : KitButton(parent), m_kind(kind) {
    setText(text);
    setFixedSize(sizeHint());
}

void Button::setKind(Kind kind) {
    m_kind = kind;
    update();
}

void Button::setIconName(const QString &icon) {
    m_icon = icon;
    setFixedSize(sizeHint());
    update();
}

QSize Button::sizeHint() const {
    int w = 10 + snapWidth(controlStrong, text()) + 10;
    if (!m_icon.isEmpty()) w += 16 + (text().isEmpty() ? 0 : space::s8);
    return QSize(qMax(64, w), height::h24);
}

QColor Button::background() const {
    if (!isEnabled()) return Qt::transparent;
    if (m_kind == Kind::Primary) return mix(color(accent), color(accentHover), hoverProgress());
    QColor fill = color(raised);
    fill.setAlphaF(float(hoverProgress()));
    return fill;
}

QColor Button::textColor() const {
    if (!isEnabled()) return color(dis);
    return m_kind == Kind::Primary ? color(accentText) : color(ink);
}

void Button::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    p.setPen(Qt::NoPen);
    p.setBrush(background());
    p.drawRoundedRect(r, radius::control, radius::control);
    if (!isEnabled()) paintInsetLine(p, r, radius::control, color(line));
    else if (m_kind == Kind::Secondary)
        paintInsetLine(p, r, radius::control, mix(color(line2), color(lineh), hoverProgress()));
    const QColor c = textColor();
    const int textWidth = text().isEmpty() ? 0 : snapWidth(controlStrong, text());
    const int content = textWidth + (m_icon.isEmpty() ? 0 : 16 + (textWidth ? space::s8 : 0));
    qreal x = (width() - content) / 2.0;
    if (!m_icon.isEmpty()) {
        icons::paint(p, m_icon, QRectF(x, 0, 16, height()), c, 16);
        x += 16 + space::s8;
    }
    if (textWidth) {
        p.setFont(tokens::font(controlStrong));
        p.setPen(c);
        p.drawText(QRectF(x, 0, textWidth + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
    }
    if (focusVisible()) paintFocusRing(p, r, radius::control);
}

// ================================================================ CheckBox

CheckBox::CheckBox(const QString &text, QWidget *parent) : KitButton(parent) {
    setCheckable(true);
    setText(text);
    setFixedSize(sizeHint());
}

QSize CheckBox::sizeHint() const {
    if (text().isEmpty()) return {14, 14};
    return QSize(14 + space::s8 + snapWidth(control, text()), control.line);
}

bool CheckBox::hitButton(const QPoint &pos) const { return rect().contains(pos); }

void CheckBox::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    if (!isEnabled()) p.setOpacity(0.5);
    const QRectF box(0, (height() - 14) / 2.0, 14, 14);
    if (isChecked()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(on));
        p.drawRoundedRect(box, radius::small, radius::small);
    }
    paintInsetLine(p, box, radius::small, color(checkboxOutline));
    if (isChecked()) icons::paint(p, QStringLiteral("checkbox-check"), box, color(ink2), 14);
    if (!text().isEmpty()) {
        p.setOpacity(1);
        p.setFont(tokens::font(control));
        p.setPen(isEnabled() ? color(ink) : color(dis));
        p.drawText(QRectF(14 + space::s8, 0, width() - 14 - space::s8 + 1, height()), Qt::AlignLeft | Qt::AlignVCenter,
                   text());
    }
    if (focusVisible()) paintFocusRing(p, box, radius::small);
}

// ================================================================ Slider

Slider::Slider(QWidget *parent) : QWidget(parent) {
    setFocusPolicy(Qt::StrongFocus);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFixedHeight(16);
}

void Slider::setRange(double minimum, double maximum) {
    m_min = minimum;
    m_max = qMax(minimum, maximum);
    setValue(m_value);
    update();
}

void Slider::setValue(double value) {
    value = std::clamp(value, m_min, m_max);
    if (value == m_value) return;
    m_value = value;
    update();
    emit valueChanged(m_value);
}

void Slider::setSingleStep(double step) { m_step = step; }

void Slider::setBipolar(bool bipolar) {
    m_bipolar = bipolar;
    update();
}

void Slider::setTip(const QString &name, const QString &shortcut) { ui::setTip(this, name, shortcut); }

double Slider::position() const { return m_max > m_min ? (m_value - m_min) / (m_max - m_min) : 0; }

QRectF Slider::trackRect() const { return QRectF(inset, 7, qMax(0, width() - 2 * inset), 2); }

QPointF Slider::knobCentre() const {
    const QRectF t = trackRect();
    return QPointF(t.left() + position() * t.width(), 8);
}

QRectF Slider::fillRect() const {
    const QRectF t = trackRect();
    const qreal x = knobCentre().x();
    if (!m_bipolar) return QRectF(t.left(), t.top(), x - t.left(), t.height());
    const qreal mid = t.left() + t.width() / 2;
    return QRectF(qMin(mid, x), t.top(), std::abs(x - mid), t.height());
}

QSize Slider::sizeHint() const { return QSize(120 + 2 * inset, 16); }
QSize Slider::minimumSizeHint() const { return QSize(20 + 2 * inset, 16); }

void Slider::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF t = trackRect();
    p.setPen(Qt::NoPen);
    p.setBrush(color(track));
    p.drawRoundedRect(t, 1, 1);
    const QRectF fill = fillRect();
    if (fill.width() > 0) {
        p.setBrush(color(track2));
        p.drawRoundedRect(fill, 1, 1);
    }
    if (m_bipolar) p.fillRect(QRectF(std::floor(t.left() + t.width() / 2), 5, 1, 6), color(track2));
    // The knob, with its 0 1px 2px shadow (a soft disc under it).
    const QPointF c = knobCentre();
    const Shadow &ks = tokens::knob;
    const qreal r = knob / 2.0;
    const QColor shadowColor = color(ks.color);
    QColor clear = shadowColor;
    clear.setAlpha(0);
    QRadialGradient shade(c + QPointF(ks.dx, ks.dy), r + ks.blur);
    shade.setColorAt(0, shadowColor);
    shade.setColorAt((r - ks.blur / 2) / (r + ks.blur), shadowColor);
    shade.setColorAt(1, clear);
    p.setBrush(shade);
    p.drawEllipse(c + QPointF(ks.dx, ks.dy), r + ks.blur, r + ks.blur);
    p.setBrush(isEnabled() ? color(ink) : color(dis));
    p.drawEllipse(c, r, r);
}

void Slider::setFromX(qreal x) {
    const QRectF t = trackRect();
    const double k = t.width() > 0 ? std::clamp((x - t.left()) / t.width(), 0.0, 1.0) : 0;
    const double before = m_value;
    setValue(m_min + k * (m_max - m_min));
    if (m_value != before) emit sliderMoved(m_value);
}

void Slider::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return QWidget::mousePressEvent(event);
    m_dragging = true;
    emit sliderPressed();
    setFromX(event->position().x());
}

void Slider::mouseMoveEvent(QMouseEvent *event) {
    if (m_dragging) setFromX(event->position().x());
}

void Slider::mouseReleaseEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !m_dragging) return QWidget::mouseReleaseEvent(event);
    m_dragging = false;
    emit sliderReleased();
}

void Slider::keyPressEvent(QKeyEvent *event) {
    const double step = m_step * ((event->modifiers() & Qt::ShiftModifier) ? 10 : 1);
    double next = m_value;
    switch (event->key()) {
    case Qt::Key_Left:
    case Qt::Key_Down: next -= step; break;
    case Qt::Key_Right:
    case Qt::Key_Up: next += step; break;
    case Qt::Key_Home: next = m_min; break;
    case Qt::Key_End: next = m_max; break;
    default: QWidget::keyPressEvent(event); return;
    }
    const double before = m_value;
    setValue(next);
    if (m_value != before) emit sliderMoved(m_value);
}

// ================================================================ Chip

Chip::Chip(const QString &text, QWidget *parent) : KitButton(parent) {
    setCheckable(true);
    setText(text);
    setFixedSize(sizeHint());
}

void Chip::setCount(int count) {
    m_count = count;
    setFixedSize(sizeHint());
    update();
}

QSize Chip::sizeHint() const {
    int w = space::s8 + snapWidth(controlStrong, text());
    if (m_count >= 0) w += space::s4 + snapWidth(control, QString::number(m_count));
    return QSize(w + space::s8, height::h24);
}

void Chip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    p.setPen(Qt::NoPen);
    if (isChecked()) {
        p.setBrush(color(on));
        p.drawRoundedRect(r, radius::control, radius::control);
        paintInsetLine(p, r, radius::control, color(line2));
        paintTopHighlight(p, r, radius::control, color(segmentHighlight));
    } else if (isEnabled() && hoverProgress() > 0) {
        QColor fill = color(raised);
        fill.setAlphaF(float(hoverProgress()));
        p.setBrush(fill);
        p.drawRoundedRect(r, radius::control, radius::control);
    }
    const TypeStyle &labelStyle = isChecked() ? controlStrong : control;
    const int labelWidth = snapWidth(labelStyle, text());
    p.setFont(tokens::font(labelStyle));
    p.setPen(!isEnabled() ? color(dis) : isChecked() ? color(ink) : color(ink2));
    p.drawText(QRectF(space::s8, 0, labelWidth + 1, height()), Qt::AlignLeft | Qt::AlignVCenter, text());
    if (m_count >= 0) {
        p.setFont(tokens::font(control));
        p.setPen(isChecked() ? color(ink2) : color(ink3));
        p.drawText(QRectF(space::s8 + labelWidth + space::s4, 0, width(), height()), Qt::AlignLeft | Qt::AlignVCenter,
                   QString::number(m_count));
    }
    if (focusVisible()) paintFocusRing(p, r, radius::control);
}

// ================================================================ Badge

namespace {
constexpr TypeStyle kBadgeType{11, 16, 500, true}; // mono 500 11/16
} // namespace

Badge::Badge(const QString &text, QWidget *parent) : QWidget(parent), m_text(text) {
    setAttribute(Qt::WA_TransparentForMouseEvents);
    setFixedSize(sizeHint());
}

void Badge::setText(const QString &text) {
    m_text = text;
    setFixedSize(sizeHint());
    update();
}

void Badge::setCount(int count) { setText(QString::number(count)); }

void Badge::setRingColor(const QColor &color) {
    m_ring = color;
    update();
}

QSize Badge::sizeHint() const {
    const int w = qMax(16, space::s4 + snapWidth(kBadgeType, m_text) + space::s4);
    return QSize(w + 2 * ring, 16 + 2 * ring);
}

QRectF Badge::badgeRect() const { return QRectF(rect()).adjusted(ring, ring, -ring, -ring); }

void Badge::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    if (m_ring.isValid()) {
        p.setBrush(m_ring);
        p.drawRoundedRect(QRectF(rect()), 8 + ring, 8 + ring);
    }
    const QRectF b = badgeRect();
    p.setBrush(color(ink2));
    p.drawRoundedRect(b, 8, 8);
    p.setFont(tokens::font(kBadgeType));
    p.setPen(color(surround));
    p.drawText(b, Qt::AlignCenter, m_text);
}

// ================================================================ Avatar

Avatar::Avatar(Kind kind, const QString &initials, QWidget *parent)
    : QWidget(parent), m_kind(kind), m_initials(initials) {
    setFixedSize(sizeHint());
}

void Avatar::setKind(Kind kind) {
    m_kind = kind;
    update();
}

void Avatar::setInitials(const QString &initials) {
    m_initials = initials;
    update();
}

void Avatar::setSelected(bool selected) {
    m_selected = selected;
    update();
}

void Avatar::setDiameter(int diameter) {
    m_diameter = diameter;
    setFixedSize(sizeHint());
    update();
}

QSize Avatar::sizeHint() const { return QSize(m_diameter, m_diameter); }

Avatar::Kind Avatar::kindFor(const QString &authorKind) {
    if (authorKind == QLatin1String("person")) return Kind::Person;
    if (authorKind == QLatin1String("kerf")) return Kind::Kerf;
    if (authorKind == QLatin1String("qc")) return Kind::Qc;
    return Kind::Agent;
}

QString Avatar::initialsFor(const QString &name) {
    const QStringList words = name.split(QRegularExpression(QStringLiteral("\\s+")), Qt::SkipEmptyParts);
    if (words.size() >= 2) return QString(words[0].front().toUpper()) + words[1].front().toUpper();
    if (words.isEmpty()) return {};
    const QString &word = words.front();
    return word.size() >= 2 ? QString(word[0].toUpper()) + word[1] : word.toUpper();
}

void Avatar::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r(0, 0, m_diameter, m_diameter);
    QPainterPath shape;
    if (isRound()) shape.addEllipse(r);
    else shape.addRoundedRect(r, radius::control, radius::control);
    p.fillPath(shape, m_selected ? color(hover2) : color(on));
    if (m_selected) {
        if (isRound()) {
            p.setPen(QPen(color(line2), 1));
            p.setBrush(Qt::NoBrush);
            p.drawEllipse(r.adjusted(0.5, 0.5, -0.5, -0.5));
        } else {
            paintInsetLine(p, r, radius::control, color(line2));
        }
    }
    const QColor c = color(ink2);
    switch (m_kind) {
    case Kind::Kerf: icons::paint(p, QStringLiteral("g-kerf"), r, c, 16); return;
    case Kind::Qc: icons::paint(p, QStringLiteral("g-gauge"), r, c, 14); return;
    case Kind::Person:
        if (m_initials.isEmpty()) {
            icons::paint(p, QStringLiteral("g-user"), r, c, 14);
            return;
        }
        break;
    case Kind::Agent: break;
    }
    p.setFont(tokens::font(metaStrong));
    p.setPen(c);
    p.drawText(r, Qt::AlignCenter, m_initials);
}

// ================================================================ Dot

Dot::Dot(const QColor &color, QWidget *parent) : QWidget(parent), m_color(color) { setFixedSize(6, 6); }

void Dot::setColor(const QColor &color) {
    m_color = color;
    update();
}

void Dot::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    p.setPen(Qt::NoPen);
    p.setBrush(m_color);
    p.drawEllipse(QRectF(0, 0, 6, 6));
}

// ================================================================ ThinProgress

ThinProgress::ThinProgress(QWidget *parent) : QWidget(parent) {
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
    setFixedHeight(4);
}

void ThinProgress::setValue(double value) {
    value = std::clamp(value, 0.0, 1.0);
    if (value == m_value) return;
    m_value = value;
    update();
}

QRectF ThinProgress::fillRect() const { return QRectF(0, 0, width() * m_value, height()); }

void ThinProgress::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    QPainterPath shape;
    shape.addRoundedRect(QRectF(rect()), 2, 2);
    p.fillPath(shape, color(track));
    p.setClipPath(shape);
    p.fillRect(fillRect(), color(accent));
}

// ================================================================ FadeScrollArea

namespace {

class FadeOverlay final : public QWidget {
public:
    explicit FadeOverlay(FadeScrollArea *area) : QWidget(area), m_area(area) {
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setAttribute(Qt::WA_NoSystemBackground);
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        QColor solid = m_area->fadeColor();
        QColor clear = solid;
        clear.setAlpha(0);
        if (m_area->topFadeShown()) {
            QLinearGradient g(0, 0, 0, FadeScrollArea::topFade);
            g.setColorAt(0, solid);
            g.setColorAt(1, clear);
            p.fillRect(QRectF(0, 0, width(), FadeScrollArea::topFade), g);
        }
        if (m_area->bottomFadeShown()) {
            const qreal top = height() - FadeScrollArea::bottomFade;
            QLinearGradient g(0, top, 0, height());
            g.setColorAt(0, clear);
            g.setColorAt(1, solid);
            p.fillRect(QRectF(0, top, width(), FadeScrollArea::bottomFade), g);
        }
    }

private:
    FadeScrollArea *m_area;
};

} // namespace

FadeScrollArea::FadeScrollArea(QWidget *parent) : QScrollArea(parent) {
    setFrameShape(QFrame::NoFrame);
    setWidgetResizable(true);
    setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_overlay = new FadeOverlay(this);
    m_overlay->setGeometry(viewport()->geometry());
    viewport()->installEventFilter(this);
    connect(verticalScrollBar(), &QScrollBar::rangeChanged, this, &FadeScrollArea::updateFades);
    connect(verticalScrollBar(), &QScrollBar::valueChanged, this, &FadeScrollArea::updateFades);
}

void FadeScrollArea::setFadeColor(const QColor &color) {
    m_fadeColor = color;
    m_overlay->update();
}

void FadeScrollArea::updateFades() {
    const QScrollBar *bar = verticalScrollBar();
    const int top = bar->value();
    const int viewHeight = viewport()->height();
    const QWidget *content = widget();
    bool cut = false;
    if (top > 0 && content) {
        bool anyChild = false;
        for (const QObject *child : content->children()) {
            const auto *w = qobject_cast<const QWidget *>(child);
            if (!w || !w->isVisible()) continue;
            anyChild = true;
            const QRect g = w->geometry();
            if (g.top() < top - 1 && g.bottom() + 1 > top + 1) {
                cut = true;
                break;
            }
        }
        if (!anyChild) cut = true; // painted content: any scroll cuts it
    }
    const bool more = content && top + viewHeight < content->height() - 1;
    m_overlay->setGeometry(viewport()->geometry());
    m_overlay->raise();
    m_top = cut;
    m_bottom = more;
    m_overlay->update();
}

bool FadeScrollArea::eventFilter(QObject *watched, QEvent *event) {
    if (watched == viewport()) {
        if (event->type() == QEvent::ChildAdded || event->type() == QEvent::Resize) {
            if (QWidget *content = widget()) content->installEventFilter(this);
            updateFades();
        }
    } else if (watched == widget()) {
        switch (event->type()) {
        case QEvent::Resize:
        case QEvent::LayoutRequest:
        case QEvent::ChildAdded:
        case QEvent::ChildRemoved: updateFades(); break;
        default: break;
        }
    }
    return QScrollArea::eventFilter(watched, event);
}

void FadeScrollArea::resizeEvent(QResizeEvent *event) {
    QScrollArea::resizeEvent(event);
    updateFades();
}

void FadeScrollArea::scrollContentsBy(int dx, int dy) {
    QScrollArea::scrollContentsBy(dx, dy);
    updateFades();
}

// ================================================================ placement and Popover

QRect popoverPlacement(const QRectF &anchor, QSize size, const QRect &bounds, Side side) {
    const qreal W = bounds.left() + bounds.width(), H = bounds.top() + bounds.height();
    const qreal minX = bounds.left() + 8, minY = bounds.top() + 8;
    const qreal w = size.width(), h = size.height();
    const qreal right = anchor.left() + anchor.width(), bottom = anchor.top() + anchor.height();
    qreal l = side == Side::Right ? right + 8 : anchor.left();
    qreal t = side == Side::Right ? anchor.top() : bottom + 4;
    if (l + w > W - 8) l = right - w;
    if (l < minX) l = minX;
    if (t + h > H - 32) t = qMax(minY, anchor.top() - h - 4);
    if (side == Side::Right && l < right) t = bottom + 4;
    return QRect(QPoint(int(std::lround(l)), int(std::lround(t))), size);
}

namespace {

// The 160 ms open: fade in and rise 4 pt to `at` (a top-level's position).
void animateOpen(QWidget *window, QPoint at) {
    const int duration = motionDuration(motion::fade);
    if (duration <= 0) {
        window->setWindowOpacity(1);
        window->move(at);
        return;
    }
    auto *open = new QVariantAnimation(window);
    open->setDuration(duration);
    open->setEasingCurve(easing());
    open->setStartValue(0.0);
    open->setEndValue(1.0);
    QPointer<QWidget> guard(window);
    QObject::connect(open, &QVariantAnimation::valueChanged, window, [guard, at](const QVariant &v) {
        if (!guard) return;
        const qreal k = v.toReal();
        guard->setWindowOpacity(k);
        guard->move(at + QPoint(0, int(std::lround(-4 * (1 - k)))));
    });
    window->setWindowOpacity(0);
    window->move(at + QPoint(0, -4));
    open->start(QAbstractAnimation::DeleteWhenStopped);
}

} // namespace

Popover::Popover(QWidget *parent)
    : QWidget(parent, Qt::Popup | Qt::FramelessWindowHint | Qt::NoDropShadowWindowHint) {
    setAttribute(Qt::WA_TranslucentBackground);
    auto *layout = new QVBoxLayout(this);
    const QMargins m = shadowMargins();
    layout->setContentsMargins(m.left() + 4, m.top() + 4, m.right() + 4, m.bottom() + 4);
    layout->setSpacing(0);
}

void Popover::setContent(QWidget *content) {
    if (m_content) {
        layout()->removeWidget(m_content);
        m_content->deleteLater();
    }
    m_content = content;
    if (content) layout()->addWidget(content);
}

void Popover::setMinimumCardWidth(int width) {
    m_minWidth = width;
    const QMargins m = shadowMargins();
    setMinimumWidth(width + m.left() + m.right());
}

QRect Popover::cardGeometry() const { return geometry().marginsRemoved(shadowMargins()); }

void Popover::openAt(QWidget *anchor, Side side) {
    if (!anchor) return;
    QWidget *window = anchor->window();
    adjustSize();
    const QMargins m = shadowMargins();
    const QSize card = size().shrunkBy(m);
    const QRectF anchorRect(QPointF(anchor->mapTo(window, QPoint(0, 0))), QSizeF(anchor->size()));
    const QRect placed = popoverPlacement(anchorRect, card, QRect(QPoint(0, 0), window->size()), side);
    const QPoint at = window->mapToGlobal(placed.topLeft()) - QPoint(m.left(), m.top());
    move(at);
    show();
    animateOpen(this, at);
}

void Popover::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF card = QRectF(rect()).marginsRemoved(QMarginsF(shadowMargins()));
    paintShadow(p, card, radius::card);
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::card));
    p.drawRoundedRect(card, radius::card, radius::card);
    paintInsetLine(p, card, radius::card, color(line2));
}

void Popover::keyPressEvent(QKeyEvent *event) {
    if (event->key() == Qt::Key_Escape) {
        close();
        return;
    }
    QWidget::keyPressEvent(event);
}

void Popover::mousePressEvent(QMouseEvent *event) {
    const QRectF card = QRectF(rect()).marginsRemoved(QMarginsF(shadowMargins()));
    if (!card.contains(event->position())) {
        event->accept();
        close();
        return;
    }
    QWidget::mousePressEvent(event);
}

void Popover::hideEvent(QHideEvent *event) {
    QWidget::hideEvent(event);
    emit closed();
}

// ================================================================ menus

namespace {

const char kMenuRole[] = "kerfMenuRole";
const char kMenuKey[] = "kerfMenuKey";
const char kMenuKeyUi[] = "kerfMenuKeyUi";
const char kMenuKeyIcon[] = "kerfMenuKeyIcon";
const char kMenuDot[] = "kerfMenuDot";
const char kMenuAccent[] = "kerfMenuAccent";
const char kMenuStyled[] = "kerfMenuStyled";

// The card sits inside the menu window, leaving room for the shadow; QMenu's
// margins are symmetric, so each side takes the larger of the two.
int menuShadowH() { return qMax(shadowMargins().left(), shadowMargins().right()); }
int menuShadowV() { return qMax(shadowMargins().top(), shadowMargins().bottom()); }

bool menuHasCheckColumn(const QMenu *menu) {
    for (const QAction *a : menu->actions())
        if (a->isVisible() && !a->isSeparator() && (a->isCheckable() || a->property(kMenuDot).isValid())) return true;
    return false;
}

QString keyText(const QAction *a) {
    const QString custom = a->property(kMenuKey).toString();
    if (!custom.isEmpty()) return custom;
    const qsizetype tab = a->text().indexOf(QLatin1Char('\t'));
    if (tab >= 0) return a->text().mid(tab + 1);
    return a->shortcut().toString(QKeySequence::NativeText);
}

QString labelText(const QAction *a) {
    QString text = a->text();
    const qsizetype tab = text.indexOf(QLatin1Char('\t'));
    if (tab >= 0) text.truncate(tab);
    return text;
}

// The width of everything right of the label: 16 of padding, then the key,
// a key icon, the accent text or a submenu arrow.
int rightColumnWidth(const QAction *a) {
    int w = 0;
    const QString key = keyText(a);
    if (!key.isEmpty()) w += snapWidth(a->property(kMenuKeyUi).toBool() ? meta : monoSmall, key);
    if (!a->property(kMenuKeyIcon).toString().isEmpty()) w += (w ? space::s4 : 0) + 14;
    const QString accentText = a->property(kMenuAccent).toString();
    if (!accentText.isEmpty()) w += (w ? space::s8 : 0) + snapWidth(control, accentText);
    if (a->menu()) w += (w ? space::s4 : 0) + 14;
    return w ? space::s16 + w : 0;
}

// The width a kit menu needs for `a`'s row, inside the card's 4 pt padding.
int menuItemWidth(const QAction *a, bool checks) {
    if (!a->isVisible() || a->isSeparator()) return 0;
    if (menuRole(a) == MenuRole::Header) return space::s8 + snapWidth(meta, a->text()) + space::s8;
    int w = space::s8;
    if (checks) w += 14 + space::s8;
    if (!a->icon().isNull()) w += 16 + space::s8;
    w += snapWidth(control, labelText(a));
    w += rightColumnWidth(a);
    return w + space::s8;
}

// The kit menu's metrics as a style sheet of its own. A widget's own sheet
// wins over the app's, so whatever the app's QSS says about QMenu, items are
// 28 tall, separators 9, and the window keeps room for the shadow around the
// card (the paint filter below draws everything).
QString menuSheet() {
    const int h = menuShadowH() + 4, v = menuShadowV() + 4;
    return QStringLiteral(
               "QMenu { background: transparent; border: none; margin: 0px; padding: %1px %2px; }"
               "QMenu::item, QMenu::item:selected, QMenu::item:disabled, QMenu::item:checked,"
               " QMenu::item:selected:disabled { margin: 0px; border: none; padding: 0px; min-height: 28px;"
               " max-height: 28px; height: 28px; background: transparent; }"
               "QMenu::separator { margin: 0px; padding: 0px; height: 9px; background: transparent; border: none; }"
               "QMenu::indicator { width: 0px; height: 0px; }"
               "QMenu::right-arrow { width: 0px; height: 0px; }")
        .arg(v)
        .arg(h);
}

// Widens the menu to its widest row (and the design's 220 minimum card).
void fitMenuWidth(QMenu *menu) {
    const bool checks = menuHasCheckColumn(menu);
    int widest = 0;
    for (const QAction *a : menu->actions()) widest = qMax(widest, menuItemWidth(a, checks));
    const int card = qMax(menuMinimumCardWidth(menu), 4 + widest + 4);
    menu->setMinimumWidth(card + 2 * menuShadowH());
}

// The card the kit paints inside the menu window.
QRectF menuCard(const QMenu *menu) {
    return QRectF(menu->rect()).adjusted(menuShadowH(), menuShadowV(), -menuShadowH(), -menuShadowV());
}

void paintMenu(QMenu *menu) {
    QPainter p(menu);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF card = menuCard(menu);
    paintShadow(p, card, radius::card);
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::card));
    p.drawRoundedRect(card, radius::card, radius::card);
    paintInsetLine(p, card, radius::card, color(line2));

    const bool checks = menuHasCheckColumn(menu);
    for (QAction *a : menu->actions()) {
        if (!a->isVisible()) continue;
        const QRectF g = menu->actionGeometry(a);
        if (g.isEmpty()) continue;
        if (a->isSeparator()) {
            p.fillRect(QRectF(g.left(), g.top() + 4, g.width(), 1), color(line));
            continue;
        }
        if (menuRole(a) == MenuRole::Header) {
            p.setFont(tokens::font(meta));
            p.setPen(color(ink3));
            p.drawText(QRectF(g.left() + 8, g.top() + 8, g.width() - 16, 16), Qt::AlignLeft | Qt::AlignVCenter,
                       a->text());
            continue;
        }
        const bool enabled = a->isEnabled();
        if (enabled && a == menu->activeAction()) {
            p.setPen(Qt::NoPen);
            p.setBrush(color(raised));
            p.drawRoundedRect(g, radius::control, radius::control);
        }
        const QColor text = !enabled ? color(dis) : menuRole(a) == MenuRole::Warn ? color(neg) : color(ink);
        qreal x = g.left() + 8;
        if (checks) {
            const QRectF column(x, g.top() + (g.height() - 14) / 2, 14, 14);
            if (const QVariant dot = a->property(kMenuDot); dot.isValid()) {
                p.setPen(Qt::NoPen);
                p.setBrush(dot.value<QColor>());
                p.drawEllipse(QRectF(column.center() - QPointF(3, 3), QSizeF(6, 6)));
            } else if (a->isCheckable() && a->isChecked()) {
                icons::paint(p, QStringLiteral("g-check"), column, enabled ? color(ink) : color(dis), 14);
            }
            x += 14 + space::s8;
        }
        if (!a->icon().isNull()) {
            a->icon().paint(&p, QRectF(x, g.top() + 6, 16, 16).toRect(), Qt::AlignCenter,
                            enabled ? QIcon::Normal : QIcon::Disabled);
            x += 16 + space::s8;
        }
        p.setFont(tokens::font(control));
        p.setPen(text);
        p.drawText(QRectF(x, g.top(), g.right() - 8 - x, g.height()), Qt::AlignLeft | Qt::AlignVCenter | Qt::TextHideMnemonic,
                   labelText(a));
        // The right column, from the right edge in.
        qreal right = g.right() - 8;
        if (a->menu()) {
            icons::paint(p, QStringLiteral("g-chevr"), QRectF(right - 14, g.top(), 14, g.height()), color(ink3), 14);
            right -= 14 + space::s4;
        }
        const QString accentText = a->property(kMenuAccent).toString();
        if (!accentText.isEmpty()) {
            const int w = snapWidth(control, accentText);
            p.setFont(tokens::font(control));
            p.setPen(enabled ? color(accentInk) : color(dis));
            p.drawText(QRectF(right - w, g.top(), w + 1, g.height()), Qt::AlignLeft | Qt::AlignVCenter, accentText);
            right -= w + space::s8;
        }
        const QString icon = a->property(kMenuKeyIcon).toString();
        if (!icon.isEmpty()) {
            icons::paint(p, icon, QRectF(right - 14, g.top(), 14, g.height()), color(ink3), 14);
            right -= 14 + space::s4;
        }
        const QString key = keyText(a);
        if (!key.isEmpty()) {
            const TypeStyle &style = a->property(kMenuKeyUi).toBool() ? meta : monoSmall;
            const int w = snapWidth(style, key);
            p.setFont(tokens::font(style));
            p.setPen(enabled ? color(ink3) : color(dis));
            p.drawText(QRectF(right - w, g.top(), w + 1, g.height()), Qt::AlignLeft | Qt::AlignVCenter, key);
        }
    }
}

class MenuPainter final : public QObject {
public:
    using QObject::QObject;

protected:
    bool eventFilter(QObject *watched, QEvent *event) override {
        auto *menu = qobject_cast<QMenu *>(watched);
        if (!menu) return false;
        switch (event->type()) {
        case QEvent::Paint:
            paintMenu(menu);
            return true;
        case QEvent::MouseButtonPress:
        case QEvent::MouseButtonDblClick: {
            // QMenu treats the whole window as inside, so a press in the
            // shadow halo would be eaten. Hand QMenu the same press at a
            // point outside its rect: it closes the menu (and the menus
            // that opened it) as for any click outside.
            auto *mouse = dynamic_cast<QMouseEvent *>(event);
            if (!mouse) return false;
            const QPointF at = mouse->position();
            if (!menu->rect().contains(at.toPoint()) || menuCard(menu).contains(at)) return false;
            QMouseEvent outside(QEvent::MouseButtonPress, QPointF(-1, -1), mouse->scenePosition(),
                                mouse->globalPosition(), mouse->button(), mouse->buttons(), mouse->modifiers(),
                                mouse->pointingDevice());
            QCoreApplication::sendEvent(menu, &outside);
            return true;
        }
        default: return false;
        }
    }
};

} // namespace

void styleMenu(QMenu *menu) {
    if (!menu || menu->property(kMenuStyled).toBool()) return;
    menu->setProperty(kMenuStyled, true);
    menu->setWindowFlags(menu->windowFlags() | Qt::FramelessWindowHint | Qt::NoDropShadowWindowHint);
    menu->setAttribute(Qt::WA_TranslucentBackground);
    menu->setStyleSheet(menuSheet());
    menu->setFont(tokens::font(control));
    menu->installEventFilter(new MenuPainter(menu));
    fitMenuWidth(menu);
    // Rows added later widen it, and submenus are styled, when it opens.
    QObject::connect(menu, &QMenu::aboutToShow, menu, [menu] {
        fitMenuWidth(menu);
        for (QAction *a : menu->actions())
            if (QMenu *sub = a->menu()) styleMenu(sub);
    });
}

void setMenuMinimumCardWidth(QMenu *menu, int width) {
    if (!menu) return;
    menu->setProperty("kerfMenuMinCard", std::max(0, width));
    if (menu->property(kMenuStyled).toBool()) fitMenuWidth(menu);
}

int menuMinimumCardWidth(const QMenu *menu) {
    const int width = menu ? menu->property("kerfMenuMinCard").toInt() : 0;
    return width > 0 ? width : 220;
}

void popupMenu(QMenu *menu, QWidget *anchor, Side side) {
    if (!menu || !anchor) return;
    styleMenu(menu);
    fitMenuWidth(menu);
    QWidget *window = anchor->window();
    menu->ensurePolished();
    const QSize full = menu->sizeHint();
    const int h = menuShadowH(), v = menuShadowV();
    const QSize card(full.width() - 2 * h, full.height() - 2 * v);
    const QRectF anchorRect(QPointF(anchor->mapTo(window, QPoint(0, 0))), QSizeF(anchor->size()));
    const QRect placed = popoverPlacement(anchorRect, card, QRect(QPoint(0, 0), window->size()), side);
    const QPoint at = window->mapToGlobal(placed.topLeft()) - QPoint(h, v);
    menu->popup(at);
    animateOpen(menu, at);
}

QAction *addMenuHeader(QMenu *menu, const QString &text) {
    // A disabled row rather than a section: QMenu gives sections a
    // separator's height, and a disabled row is skipped by the keyboard.
    QAction *header = menu->addAction(text);
    header->setEnabled(false);
    setMenuRole(header, MenuRole::Header);
    return header;
}

void setMenuRole(QAction *action, MenuRole role) {
    if (action) action->setProperty(kMenuRole, int(role));
}

MenuRole menuRole(const QAction *action) {
    return action ? MenuRole(action->property(kMenuRole).toInt()) : MenuRole::Normal;
}

void setMenuKey(QAction *action, const QString &key, bool uiFont) {
    if (!action) return;
    action->setProperty(kMenuKey, key);
    action->setProperty(kMenuKeyUi, uiFont);
}

QString menuKey(const QAction *action) { return action ? action->property(kMenuKey).toString() : QString(); }

void setMenuKeyIcon(QAction *action, const QString &icon) {
    if (action) action->setProperty(kMenuKeyIcon, icon);
}

void setMenuDot(QAction *action, const QColor &color) {
    if (action) action->setProperty(kMenuDot, color);
}

void setMenuAccent(QAction *action, const QString &text) {
    if (action) action->setProperty(kMenuAccent, text);
}

} // namespace kerf::ui
