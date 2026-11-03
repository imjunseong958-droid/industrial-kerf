#include "stage_rail.h"
#include "design_tokens.h"
#include "device_frame.h"
#include "editor_bus.h"
#include "icons.h"
#include "program_level.h"
#include "review_model.h"

#include <QAction>
#include <QEvent>
#include <QKeySequence>
#include <QMenu>
#include <QMetaMethod>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QSignalBlocker>

#include <algorithm>
#include <cmath>
#include <numeric>

namespace kerf {

using tokens::color;

// ================================================================ RailButton

RailButton::RailButton(Glyph glyph, QWidget *parent) : ui::KitButton(parent), m_glyph(glyph) {
    setObjectName(QStringLiteral("railButton"));
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setFixedSize(kDisc, kDisc);
}

void RailButton::setGlyph(Glyph glyph) {
    if (glyph == m_glyph) return;
    m_glyph = glyph;
    update();
    if (m_stateCallback) m_stateCallback();
}

void RailButton::setIconName(const QString &icon) {
    if (icon == m_icon) return;
    m_icon = icon;
    update();
}

QString RailButton::iconName() const {
    if (!m_icon.isEmpty()) return m_icon;
    switch (m_glyph) {
    case Glyph::Chat: return QStringLiteral("g-comments");
    case Glyph::Phone: return QStringLiteral("g-phone");
    case Glyph::Laptop: return QStringLiteral("g-laptop");
    case Glyph::SafeZones: return QStringLiteral("g-safe");
    }
    return {};
}

void RailButton::setChevron(bool chevron) {
    if (chevron == m_chevron) return;
    m_chevron = chevron;
    update();
}

void RailButton::setHeld(bool held) {
    if (held == m_held) return;
    m_held = held;
    update();
}

bool RailButton::drawsOn() const { return m_held || (isChecked() && !m_checkedByOwner); }

void RailButton::setRetired(bool retired) {
    m_retired = retired;
    if (retired) QAbstractButton::setVisible(false);
}

void RailButton::setVisible(bool visible) { QAbstractButton::setVisible(visible && !m_retired); }

void RailButton::checkStateSet() {
    QAbstractButton::checkStateSet();
    if (m_stateCallback) m_stateCallback();
}

bool RailButton::event(QEvent *event) {
    // The rail gives its buttons the design's tip (name and shortcut). An
    // old owner's setToolTip() would put a plain Qt tip over it, so the
    // design's wins.
    if (event->type() == QEvent::ToolTipChange && !ui::tipName(this).isEmpty() && !toolTip().isEmpty())
        setToolTip(QString());
    return ui::KitButton::event(event);
}

void RailButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF r = rect();
    const bool on = drawsOn();
    if (on) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(tokens::on));
        p.drawRoundedRect(r, tokens::radius::control, tokens::radius::control);
        ui::paintInsetLine(p, r, tokens::radius::control, color(tokens::line2));
    } else if (isEnabled() && hoverProgress() > 0) {
        QColor fill = color(tokens::raised);
        fill.setAlphaF(float(hoverProgress()));
        p.setPen(Qt::NoPen);
        p.setBrush(fill);
        p.drawRoundedRect(r, tokens::radius::control, tokens::radius::control);
    }
    const QColor ink = !isEnabled() ? color(tokens::dis)
                       : on         ? color(tokens::ink)
                                    : ui::mix(color(tokens::ink2), color(tokens::ink), hoverProgress());
    icons::paint(p, iconName(), r, ink, 16);
    if (m_chevron)
        icons::paint(p, QStringLiteral("rail-chevron"), QRectF(r.right() - 1 - 10, r.bottom() - 1 - 10, 10, 10),
                     color(isEnabled() ? tokens::ink3 : tokens::dis), 10);
    if (focusVisible()) ui::paintFocusRing(p, r, tokens::radius::control);
}

// ================================================================ the rail's parts

namespace {

// A label under a rail button: UI 11/16 ink2, centred in a box 60 wide. It
// is a sibling of the rail (in the rail's parent), so it may be wider than
// the rail; it takes no clicks.
class RailLabel final : public QWidget {
public:
    explicit RailLabel(const QString &name, QWidget *parent) : QWidget(parent) {
        setObjectName(name);
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setFocusPolicy(Qt::NoFocus);
    }
    void setText(const QString &text) {
        if (text == m_text) return;
        m_text = text;
        update();
    }
    QString text() const { return m_text; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(tokens::font(tokens::meta));
        p.setPen(color(tokens::ink2));
        p.drawText(rect(), Qt::AlignHCenter | Qt::AlignTop, m_text);
    }

private:
    QString m_text;
};

} // namespace

// The meter's two bars, in the meter's own rect (the rail draws its scale).
class LevelMeterWidget final : public QWidget {
public:
    explicit LevelMeterWidget(StageRail *rail) : QWidget(rail), m_rail(rail) {
        setObjectName(QStringLiteral("programLevel"));
        setAccessibleName(QStringLiteral("Program level"));
        setCursor(Qt::PointingHandCursor);
        ui::setTip(this, QStringLiteral("Program level · dBFS · click for the loudness bridge"));
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        const ProgramMeter *meter = m_rail->programMeter();
        const double h = height();
        for (int c = 0; c < ProgramMeter::kChannels; ++c) {
            const QRectF bar(6 + 8 * c, 0, 6, h);
            p.fillRect(bar, color(tokens::raised));
            p.save();
            p.setClipRect(bar);
            if (meter->hasLevel(c))
                for (const ProgramMeter::Segment &segment : ProgramMeter::segments(meter->level(c), h))
                    p.fillRect(QRectF(bar.left(), segment.top, bar.width(), segment.bottom - segment.top), color(segment.color));
            if (meter->hasHold(c))
                p.fillRect(QRectF(bar.left(), ProgramMeter::y(meter->hold(c), h), bar.width(), 2), color(tokens::ink));
            p.restore();
        }
    }
    void mousePressEvent(QMouseEvent *event) override { event->accept(); }
    void mouseReleaseEvent(QMouseEvent *event) override {
        if (event->button() == Qt::LeftButton && rect().contains(event->position().toPoint())) m_rail->requestLoudness();
    }

private:
    StageRail *m_rail;
};

// "−17.8" (mono 500 12/16 ink) over "LUFS M" (UI 11/16 ink2), centred.
class LufsReadoutWidget final : public QWidget {
public:
    explicit LufsReadoutWidget(StageRail *rail) : QWidget(rail), m_rail(rail) {
        setObjectName(QStringLiteral("railLufs"));
        setAccessibleName(QStringLiteral("Momentary loudness"));
        setCursor(Qt::PointingHandCursor);
        ui::setTip(this, QStringLiteral("Momentary loudness · loudness bridge"));
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setFont(tokens::font(tokens::timecodeStrong));
        p.setPen(color(tokens::ink));
        p.drawText(QRect(0, 0, width(), 16), Qt::AlignHCenter | Qt::AlignVCenter, m_rail->lufsText());
        p.setFont(tokens::font(tokens::meta));
        p.setPen(color(tokens::ink2));
        p.drawText(QRect(-8, 16, width() + 16, 16), Qt::AlignHCenter | Qt::AlignVCenter, QStringLiteral("LUFS M"));
    }
    void mousePressEvent(QMouseEvent *event) override { event->accept(); }
    void mouseReleaseEvent(QMouseEvent *event) override {
        if (event->button() == Qt::LeftButton && rect().contains(event->position().toPoint())) m_rail->requestLoudness();
    }

private:
    StageRail *m_rail;
};

// ================================================================ StageRail

StageRail::StageRail(QWidget *parent, EditorBus *bus) : QWidget(parent) {
    setObjectName(QStringLiteral("stageRail"));
    setAccessibleName(QStringLiteral("Stage"));
    m_meter = new ProgramMeter(this);
    m_level = new LevelMeterWidget(this);
    m_lufs = new LufsReadoutWidget(this);
    connect(m_meter, &ProgramMeter::changed, this, [this] {
        m_level->update();
        m_lufs->update();
    });

    m_device = new RailButton(RailButton::Glyph::Phone, this);
    m_device->setObjectName(QStringLiteral("railDevice"));
    m_device->setCheckable(true);
    m_device->setChecked(true);
    m_device->setCheckedByOwner(true);
    m_device->setChevron(true);
    m_device->setStateCallback([this] { refreshDevice(); });
    connect(m_device, &QAbstractButton::clicked, this, [this] {
        // The stage sets the preview; until it listens, the device button's
        // old owner opens its own menu on the click.
        if (isSignalConnected(QMetaMethod::fromSignal(&StageRail::devicePreviewRequested))) showDeviceMenu();
    });

    m_safeZones = new RailButton(RailButton::Glyph::SafeZones, this);
    m_safeZones->setObjectName(QStringLiteral("railSafeZones"));
    m_safeZones->setCheckable(true);
    m_safeZones->setRetired(true);

    m_chat = new RailButton(RailButton::Glyph::Chat, this);
    m_chat->setObjectName(QStringLiteral("railComments"));
    m_chat->setCheckable(true);
    m_chat->setAccessibleName(QStringLiteral("Comments"));

    m_badge = new ui::Badge(QString(), this);
    m_badge->setObjectName(QStringLiteral("railCommentsBadge"));
    m_badge->setRingColor(color(tokens::surround));
    m_badge->hide();

    m_vu = new VuMeter(this);

    m_deviceLabel = new RailLabel(QStringLiteral("railDeviceLabel"), this);
    m_commentsLabel = new RailLabel(QStringLiteral("railCommentsLabel"), this);
    static_cast<RailLabel *>(m_commentsLabel.data())->setText(QStringLiteral("Comments"));

    resize(kWidth, kHeight);
    relayout();
    setBus(bus);
    refreshDevice();
    refreshComments();
}

StageRail::~StageRail() {
    // The labels may live in the parent, beside the rail.
    delete m_deviceLabel.data();
    delete m_commentsLabel.data();
}

QWidget *StageRail::deviceLabelWidget() const { return m_deviceLabel; }
QWidget *StageRail::commentsLabelWidget() const { return m_commentsLabel; }

EditorBus *StageRail::bus() const { return m_bus.data(); }

void StageRail::setBus(EditorBus *bus) {
    if (bus == m_bus) return;
    for (const QMetaObject::Connection &connection : std::as_const(m_busConnections)) disconnect(connection);
    m_busConnections.clear();
    m_bus = bus;
    if (m_bus) {
        m_busConnections << connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &json) {
            if (type == QStringLiteral("meter")) m_meter->apply(json);
        });
        m_busConnections << connect(m_bus, &EditorBus::reviewChanged, this, &StageRail::watchReview);
        m_busConnections << connect(m_bus, &EditorBus::snapshotChanged, this, &StageRail::refreshDevice);
        const QJsonObject meter = m_bus->lastEvent(QStringLiteral("meter"));
        if (!meter.isEmpty()) m_meter->apply(meter);
    }
    watchReview();
    refreshDevice();
}

void StageRail::setActions(QAction *comments, QAction *device) {
    m_actionsGiven = true;
    for (QAction *old : {m_commentsAction.data(), m_deviceAction.data()})
        if (old) disconnect(old, nullptr, this, nullptr);
    m_commentsAction = comments;
    m_deviceAction = device;
    for (QAction *action : {comments, device})
        if (action) connect(action, &QAction::changed, this, &StageRail::refreshTips);
    refreshTips();
}

void StageRail::attachToWindow() {
    QWidget *top = window();
    if (!top || top == this) return;
    if (!m_bus)
        if (auto *bus = top->findChild<EditorBus *>(QStringLiteral("editorBus"))) setBus(bus);
    if (!m_actionsGiven) {
        QAction *comments = top->findChild<QAction *>(QStringLiteral("window.assistant"));
        QAction *device = top->findChild<QAction *>(QStringLiteral("view.devicePreview"));
        if (comments || device) {
            setActions(comments, device);
            // Found, not given: a later window may still give its own.
            m_actionsGiven = false;
        }
    }
}

void StageRail::watchReview() {
    for (const QMetaObject::Connection &connection : std::as_const(m_reviewConnections)) disconnect(connection);
    m_reviewConnections.clear();
    if (ReviewModel *review = m_bus ? m_bus->review() : nullptr) {
        m_reviewConnections << connect(review, &ReviewModel::countsChanged, this, &StageRail::refreshComments);
        m_reviewConnections << connect(review, &ReviewModel::changed, this, &StageRail::refreshComments);
        m_reviewConnections << connect(review, &QObject::destroyed, this, [this] {
            m_reviewConnections.clear();
            refreshComments();
        });
    }
    refreshComments();
}

// ---- geometry ---------------------------------------------------------------

int StageRail::meterHeight() const { return std::max(0, kMeterHeight + height() - kHeight); }

QRect StageRail::meterRect() const { return QRect(0, kMeterTop, kWidth, meterHeight()); }

QRect StageRail::barRect(int channel) const { return QRect(6 + 8 * channel, kMeterTop, 6, meterHeight()); }

double StageRail::meterY(double db) const { return ProgramMeter::y(db, meterHeight()); }

QRect StageRail::channelLabelRect(int channel) const {
    return QRect(6 + 8 * channel, 362 + height() - kHeight, 6, 16);
}

QRect StageRail::scaleLabelRect(int mark) const {
    return QRect(24, kMeterTop + int(meterY(-mark)) - 8, 20, 16);
}

QList<int> StageRail::shownScaleMarks() const {
    QList<int> shown;
    double last = 0.0;
    for (const int mark : ProgramMeter::scaleMarks()) {
        const double y = meterY(-mark);
        if (!shown.isEmpty() && y - last < 12.0) continue;
        shown << mark;
        last = y;
    }
    return shown;
}

QRect StageRail::lufsRect() const { return QRect(0, 390 + height() - kHeight, kWidth, 32); }
QRect StageRail::deviceRect() const { return QRect(6, 438 + height() - kHeight, 32, 32); }
QRect StageRail::deviceLabelRect() const {
    return QRect((kWidth - kLabelWidth) / 2, 472 + height() - kHeight, kLabelWidth, 16);
}
QRect StageRail::commentsRect() const { return QRect(6, 498 + height() - kHeight, 32, 32); }
QRect StageRail::commentsLabelRect() const {
    return QRect((kWidth - kLabelWidth) / 2, 532 + height() - kHeight, kLabelWidth, 16);
}

QString StageRail::lufsText() const { return m_meter->momentaryText(); }

void StageRail::relayout() {
    const bool meter = meterHeight() >= kMeterMinHeight;
    m_level->setGeometry(meterRect());
    m_level->setVisible(meter);
    m_lufs->setGeometry(lufsRect());
    m_device->setGeometry(deviceRect());
    m_chat->setGeometry(commentsRect());
    refreshComments();
    placeLabels();
    update();
}

void StageRail::placeLabels() {
    QWidget *host = parentWidget() ? parentWidget() : this;
    const QPoint offset = host == this ? QPoint() : pos();
    const QList<QPair<QWidget *, QRect>> labels{{m_deviceLabel.data(), deviceLabelRect()},
                                                {m_commentsLabel.data(), commentsLabelRect()}};
    for (const auto &[label, rect] : labels) {
        if (!label) continue;
        if (label->parentWidget() != host) label->setParent(host);
        label->setGeometry(rect.translated(offset));
        label->setVisible(host == this || !isHidden());
        if (host != this) label->raise();
    }
}

void StageRail::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    relayout();
}

void StageRail::moveEvent(QMoveEvent *event) {
    QWidget::moveEvent(event);
    placeLabels();
}

bool StageRail::event(QEvent *event) {
    const bool handled = QWidget::event(event);
    switch (event->type()) {
    case QEvent::ParentChange:
    case QEvent::Show:
        attachToWindow();
        placeLabels();
        break;
    case QEvent::Hide:
    case QEvent::ZOrderChange:
        placeLabels();
        break;
    default:
        break;
    }
    return handled;
}

void StageRail::paintEvent(QPaintEvent *) {
    if (meterHeight() < kMeterMinHeight) return;
    QPainter p(this);
    p.setFont(tokens::font(tokens::monoSmall));
    p.setPen(color(tokens::ink2));
    for (const int mark : shownScaleMarks())
        p.drawText(scaleLabelRect(mark), Qt::AlignRight | Qt::AlignVCenter, QString::number(mark));
    p.setFont(tokens::font(tokens::meta));
    p.setPen(color(tokens::ink3));
    p.drawText(channelLabelRect(0), Qt::AlignHCenter | Qt::AlignVCenter, QStringLiteral("L"));
    p.drawText(channelLabelRect(1), Qt::AlignHCenter | Qt::AlignVCenter, QStringLiteral("R"));
}

void StageRail::requestLoudness() {
    if (m_bus) {
        m_bus->run(QStringLiteral("workspace.finish"));
        emit m_bus->highlightPane(QStringLiteral("loudness"));
    }
    emit loudnessRequested();
}

// ---- the device ---------------------------------------------------------------

void StageRail::setDevicePreview(double pictureAspect, bool on) {
    m_hasDevice = true;
    m_deviceAspect = pictureAspect > 0.0 ? pictureAspect : 16.0 / 9.0;
    m_deviceOn = on;
    {
        // The button's checked state says whether the device shows, as its
        // old owner kept it.
        const QSignalBlocker blocker(m_device);
        m_device->setChecked(on);
    }
    refreshDevice();
}

double StageRail::deviceAspect() const {
    if (m_hasDevice) return m_deviceAspect;
    if (m_bus && m_bus->frameSize().width() > 0 && m_bus->frameSize().height() > 0)
        return double(m_bus->frameSize().width()) / m_bus->frameSize().height();
    return m_device->glyph() == RailButton::Glyph::Laptop ? 16.0 / 9.0 : 9.0 / 16.0;
}

bool StageRail::deviceOn() const { return m_hasDevice ? m_deviceOn : m_device->isChecked(); }

bool StageRail::deviceIsLaptop() const { return deviceAspect() > 1.0 + 1e-3; }

QString StageRail::formatName() const {
    const double aspect = deviceAspect();
    if (m_bus) {
        const QSize frame = m_bus->frameSize();
        if (frame.width() > 0 && frame.height() > 0 && std::abs(double(frame.width()) / frame.height() - aspect) < 1e-3) {
            const int g = std::gcd(frame.width(), frame.height());
            const int w = frame.width() / g, h = frame.height() / g;
            if (w <= 64 && h <= 64) return QStringLiteral("%1:%2").arg(w).arg(h);
        }
    }
    static const struct {
        int w, h;
    } known[] = {{16, 9}, {9, 16}, {1, 1}, {4, 5}, {4, 3}, {3, 4}, {21, 9}, {2, 1}};
    for (const auto &k : known)
        if (std::abs(double(k.w) / k.h - aspect) < 0.01) return QStringLiteral("%1:%2").arg(k.w).arg(k.h);
    return QStringLiteral("%1:1").arg(aspect, 0, 'f', 2);
}

QList<StageRail::DeviceChoice> StageRail::deviceChoices() const {
    const bool on = deviceOn();
    QList<DeviceChoice> out;
    if (deviceIsLaptop()) {
        out << DeviceChoice{true, laptopPreviewLabel(LaptopPreview::Laptop), QStringLiteral("g-laptop"), on};
        out << DeviceChoice{false, laptopPreviewLabel(LaptopPreview::Off), QStringLiteral("g-devoff"), !on};
        return out;
    }
    const double aspect = deviceAspect();
    for (const DevicePreview preview : devicePreviewChoices()) {
        DeviceChoice choice;
        choice.on = preview != DevicePreview::Off;
        choice.name = devicePreviewLabel(preview, aspect);
        choice.icon = !choice.on                ? QStringLiteral("g-devoff")
                      : deviceShowsFeed(aspect) ? QStringLiteral("g-feed")
                                                : QStringLiteral("g-phone");
        choice.checked = choice.on == on;
        out << choice;
    }
    return out;
}

QString StageRail::deviceLabel() const {
    if (!deviceOn()) return QStringLiteral("Off");
    if (deviceIsLaptop()) return QStringLiteral("Laptop");
    return deviceShowsFeed(deviceAspect()) ? QStringLiteral("Feed") : QStringLiteral("Phone");
}

QString StageRail::deviceMenuHeader() const { return QStringLiteral("Device preview · %1").arg(formatName()); }

QMenu *StageRail::createDeviceMenu(QWidget *parent) {
    auto *menu = new QMenu(parent);
    menu->setObjectName(QStringLiteral("deviceMenu"));
    ui::styleMenu(menu);
    menu->setMinimumWidth(240);
    ui::addMenuHeader(menu, deviceMenuHeader());
    for (const DeviceChoice &choice : deviceChoices()) {
        QAction *action = menu->addAction(choice.name);
        action->setCheckable(true);
        action->setChecked(choice.checked);
        ui::setMenuKeyIcon(action, choice.icon);
        const bool on = choice.on;
        connect(action, &QAction::triggered, this, [this, on] { emit devicePreviewRequested(on); });
    }
    return menu;
}

void StageRail::showDeviceMenu() {
    QMenu *menu = createDeviceMenu(this);
    m_device->setHeld(true);
    connect(menu, &QMenu::aboutToHide, this, [this, menu] {
        m_device->setHeld(false);
        menu->deleteLater();
    });
    ui::popupMenu(menu, m_device, ui::Side::Right);
}

void StageRail::refreshDevice() {
    const bool on = deviceOn();
    m_device->setIconName(!on                         ? QStringLiteral("g-devoff")
                          : deviceIsLaptop()          ? QStringLiteral("g-laptop")
                          : deviceShowsFeed(deviceAspect()) ? QStringLiteral("g-feed")
                                                            : QStringLiteral("g-phone"));
    if (auto *label = static_cast<RailLabel *>(m_deviceLabel.data())) label->setText(deviceLabel());
    refreshTips();
}

// ---- the comments -------------------------------------------------------------

int StageRail::openCount() const {
    const ReviewModel *review = m_bus ? m_bus->review() : nullptr;
    return review ? review->counts().open : -1;
}

QString StageRail::commentsTip() const {
    const int open = openCount();
    return open < 0 ? QStringLiteral("Comments") : QStringLiteral("Comments · %1 open").arg(open);
}

void StageRail::refreshComments() {
    const int open = openCount();
    if (open > 0) {
        m_badge->setCount(open);
        // The badge's right edge 6 past the button's, its top 6 above it;
        // the widget carries the 2 pt ring around it.
        const QRect button = commentsRect();
        const QSize size = m_badge->size();
        m_badge->move(button.right() + 1 + 6 - (size.width() - ui::Badge::ring), button.top() - 6 - ui::Badge::ring);
        m_badge->show();
        m_badge->raise();
    } else {
        m_badge->hide();
    }
    refreshTips();
}

QString StageRail::shortcutOf(const QAction *action) const {
    return action ? action->shortcut().toString(QKeySequence::NativeText) : QString();
}

void StageRail::refreshTips() {
    ui::setTip(m_chat, commentsTip(), shortcutOf(m_commentsAction));
    QString current = deviceLabel();
    for (const DeviceChoice &choice : deviceChoices())
        if (choice.checked) current = choice.name;
    ui::setTip(m_device, QStringLiteral("Device preview: %1").arg(current), shortcutOf(m_deviceAction));
    m_device->setAccessibleName(QStringLiteral("Device preview: %1").arg(current));
}

} // namespace kerf
