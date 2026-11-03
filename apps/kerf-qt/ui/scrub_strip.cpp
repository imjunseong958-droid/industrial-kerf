#include "scrub_strip.h"
#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "editor_common.h"
#include "icons.h"

#include <QAction>
#include <QEvent>
#include <QFontMetricsF>
#include <QJsonObject>
#include <QKeySequence>
#include <QMetaMethod>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QSignalBlocker>
#include <QTextOption>

#include <algorithm>
#include <cmath>

namespace kerf {

using tokens::color;

namespace {

// The ranges' grey, as the mockup writes it: rgba(138,138,138,.6), and .25
// once resolved.
QColor rangeGrey(double alpha) { return QColor(138, 138, 138, tokens::alpha(alpha)); }

QFont initialsFont() {
    QFont font = tokens::font(tokens::metaStrong);
    font.setLetterSpacing(QFont::AbsoluteSpacing, tokens::markerInitialsSpacing);
    return font;
}

// The transport's play button: 28 x 28, radius 6, its icon in ink at rest
// (.ib.play), raised under the pointer.
class PlayButton final : public ui::KitButton {
public:
    explicit PlayButton(QWidget *parent) : ui::KitButton(parent) {
        setObjectName(QStringLiteral("scrubPlay"));
        setCursor(Qt::PointingHandCursor);
        setFixedSize(tokens::height::h28, tokens::height::h28);
    }
    void setPlaying(bool playing) {
        if (playing == m_playing) return;
        m_playing = playing;
        update();
    }
    bool playing() const { return m_playing; }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF r = rect();
        if (isEnabled() && hoverProgress() > 0) {
            QColor fill = color(tokens::raised);
            fill.setAlphaF(float(hoverProgress()));
            p.setPen(Qt::NoPen);
            p.setBrush(fill);
            p.drawRoundedRect(r, tokens::radius::control, tokens::radius::control);
        }
        icons::paint(p, m_playing ? QStringLiteral("g-pause") : QStringLiteral("g-play"), r,
                     color(isEnabled() ? tokens::ink : tokens::dis), 16);
        if (focusVisible()) ui::paintFocusRing(p, r, tokens::radius::control);
    }

private:
    bool m_playing = false;
};

// The marker popover (#mpop): 280 wide, card, radius 8, the pop shadow and
// an inset line2, pad 12. Each marker: a 20-point name line (name, role,
// timecode, then Open or Resolved on the right) over its text.
class MarkerPopover final : public QWidget {
public:
    explicit MarkerPopover(QWidget *parent) : QWidget(parent) {
        setObjectName(QStringLiteral("scrubMarkerPopover"));
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setAttribute(Qt::WA_NoSystemBackground);
        setFocusPolicy(Qt::NoFocus);
        hide();
    }
    void setEntries(const QList<ReviewModel::LaneMarker> &markers, double fps) {
        m_markers = markers;
        m_fps = fps;
        const QMargins m = ui::shadowMargins();
        resize(ScrubStrip::kPopoverWidth + m.left() + m.right(), cardHeight() + m.top() + m.bottom());
        update();
    }
    // The card, in this widget.
    QRect card() const {
        const QMargins m = ui::shadowMargins();
        return rect().marginsRemoved(m);
    }
    int cardHeight() const {
        int h = 12;
        for (const ReviewModel::LaneMarker &marker : m_markers) h += 20 + 2 + textHeight(marker.body) + 4;
        return h + 12;
    }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const QRectF c = card();
        ui::paintShadow(p, c, tokens::radius::card);
        p.setPen(Qt::NoPen);
        p.setBrush(color(tokens::card));
        p.drawRoundedRect(c, tokens::radius::card, tokens::radius::card);
        ui::paintInsetLine(p, c, tokens::radius::card, color(tokens::line2));
        qreal y = c.top() + 12;
        const qreal left = c.left() + 12, right = c.right() - 12;
        for (const ReviewModel::LaneMarker &marker : m_markers) {
            // The name line: 500 13/20 ink, gap 6, role 12/16 ink3, gap 6,
            // TC mono 12 accent-ink at .85, spacer, the state 11/16 ink3.
            const QString state = marker.resolved ? QStringLiteral("Resolved") : QStringLiteral("Open");
            const QFont stateFont = tokens::font(tokens::meta);
            const qreal stateWidth = ui::snapWidth(tokens::meta, state);
            p.setFont(stateFont);
            p.setPen(color(tokens::ink3));
            p.drawText(QRectF(right - stateWidth, y, stateWidth, 20), Qt::AlignRight | Qt::AlignVCenter, state);
            qreal x = left;
            const qreal limit = right - stateWidth - 6;
            const auto run = [&](const QString &text, const tokens::TypeStyle &style, const QColor &ink) {
                if (text.isEmpty() || x >= limit) return;
                const QFont font = tokens::font(style);
                const QString shown = QFontMetricsF(font).elidedText(text, Qt::ElideRight, limit - x);
                const qreal w = std::min<qreal>(ui::snapWidth(style, shown), limit - x);
                p.setFont(font);
                p.setPen(ink);
                p.drawText(QRectF(x, y, w, 20), Qt::AlignLeft | Qt::AlignVCenter, shown);
                x += w + 6;
            };
            run(marker.name, tokens::bodyName, color(tokens::ink));
            run(marker.role, tokens::control, color(tokens::ink3));
            QColor tc = color(tokens::accentInk);
            tc.setAlphaF(0.85f);
            run(timelineTimecode(marker.at, m_fps), tokens::timecode, tc);
            y += 20 + 2;
            const int h = textHeight(marker.body);
            p.setFont(tokens::font(tokens::body));
            p.setPen(color(tokens::ink));
            QTextOption wrap(Qt::AlignLeft | Qt::AlignTop);
            wrap.setWrapMode(QTextOption::WordWrap);
            p.drawText(QRectF(left, y, right - left, h), marker.body, wrap);
            y += h + 4;
        }
    }

private:
    int textHeight(const QString &text) const {
        if (text.isEmpty()) return 0;
        const QFontMetricsF metrics(tokens::font(tokens::body));
        const QRectF bounds = metrics.boundingRect(QRectF(0, 0, ScrubStrip::kPopoverWidth - 24, 1e6),
                                                   Qt::TextWordWrap | Qt::AlignLeft, text);
        // Whole 20-point lines, as the 13/20 body sets them.
        return std::max(1, int(std::ceil(bounds.height() / 20.0 - 0.05))) * tokens::body.line;
    }
    QList<ReviewModel::LaneMarker> m_markers;
    double m_fps = 24.0;
};

} // namespace

// ================================================================ ScrubStrip

ScrubStrip::ScrubStrip(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("scrubStrip"));
    setAccessibleName(QStringLiteral("Scrub"));
    setFixedHeight(tokens::layout::scrub);
    setMouseTracking(true);

    const auto button = [this](const char *name, const char *icon) {
        auto *b = new ui::IconButton(QString::fromLatin1(icon), ui::IconButton::Size::S24, this);
        b->setObjectName(QString::fromLatin1(name));
        b->setCursor(Qt::PointingHandCursor);
        return b;
    };
    m_markIn = button("scrubMarkIn", "g-markin");
    m_markOut = button("scrubMarkOut", "g-markout");
    m_start = button("scrubStart", "g-tostart");
    m_previous = button("scrubPrevious", "g-prev");
    m_play = new PlayButton(this);
    m_next = button("scrubNext", "g-next");
    m_end = button("scrubEnd", "g-toend");
    m_loopButton = button("scrubLoop", "g-loop");
    m_loopButton->setCheckable(true);

    connect(m_markIn, &QAbstractButton::clicked, this, &ScrubStrip::markIn);
    connect(m_markOut, &QAbstractButton::clicked, this, &ScrubStrip::markOut);
    connect(m_start, &QAbstractButton::clicked, this, [this] { run(QStringLiteral("playback.goToStart")); });
    connect(m_previous, &QAbstractButton::clicked, this, [this] { run(QStringLiteral("playback.previousFrame")); });
    connect(m_play, &QAbstractButton::clicked, this, [this] { run(QStringLiteral("playback.toggle")); });
    connect(m_next, &QAbstractButton::clicked, this, [this] { run(QStringLiteral("playback.nextFrame")); });
    connect(m_end, &QAbstractButton::clicked, this, [this] { run(QStringLiteral("playback.goToEnd")); });
    connect(m_loopButton, &QAbstractButton::toggled, this, &ScrubStrip::setLoop);
    // The pointer on a transport button is off the lane's markers.
    for (QWidget *child : {static_cast<QWidget *>(m_markIn), static_cast<QWidget *>(m_markOut), static_cast<QWidget *>(m_start),
                           static_cast<QWidget *>(m_previous), static_cast<QWidget *>(m_play), static_cast<QWidget *>(m_next),
                           static_cast<QWidget *>(m_end), static_cast<QWidget *>(m_loopButton)})
        child->installEventFilter(this);

    if (m_bus) {
        connect(m_bus, &EditorBus::playheadChanged, this, [this] {
            followPlayhead();
            update();
        });
        connect(m_bus, &EditorBus::playingChanged, this, [this](bool playing) {
            followPlaying(playing);
            refresh();
        });
        connect(m_bus, &EditorBus::snapshotChanged, this, &ScrubStrip::refresh);
        connect(m_bus, &EditorBus::reviewChanged, this, &ScrubStrip::watchReview);
        connect(m_bus, &EditorBus::event, this, [this](const QString &type, const QJsonObject &) {
            // The length arrives with the clock (EditorBus::setDuration is
            // quiet).
            if (type == QStringLiteral("clock") && m_bus->duration() != m_shownDuration) refresh();
        });
    }
    watchReview();
    relayout();
    refresh();
}

ScrubStrip::~ScrubStrip() { delete m_popover.data(); }

void ScrubStrip::watchReview() {
    for (const QMetaObject::Connection &connection : std::as_const(m_reviewConnections)) disconnect(connection);
    m_reviewConnections.clear();
    if (ReviewModel *review = m_bus ? m_bus->review() : nullptr) {
        m_reviewConnections << connect(review, &ReviewModel::changed, this, &ScrubStrip::refresh);
        m_reviewConnections << connect(review, &ReviewModel::currentChanged, this, &ScrubStrip::refresh);
        m_reviewConnections << connect(review, &QObject::destroyed, this, [this] {
            m_reviewConnections.clear();
            m_hover = -1;
            hidePopover();
            update();
        });
    }
    m_hover = -1;
    hidePopover();
    refresh();
}

void ScrubStrip::setActions(const QHash<QString, QAction *> &actions) {
    m_actionsGiven = true;
    for (const QPointer<QAction> &old : std::as_const(m_actions))
        if (old) disconnect(old, nullptr, this, nullptr);
    m_actions.clear();
    for (auto it = actions.cbegin(); it != actions.cend(); ++it) {
        if (!it.value()) continue;
        m_actions.insert(it.key(), it.value());
        connect(it.value(), &QAction::changed, this, &ScrubStrip::refreshTransport);
    }
    refreshTransport();
}

void ScrubStrip::attachToWindow() {
    if (m_actionsGiven) return;
    QWidget *top = window();
    if (!top || top == this) return;
    QHash<QString, QAction *> found;
    for (const char *id : {"playback.toggle", "playback.goToStart", "playback.previousFrame", "playback.nextFrame",
                           "playback.goToEnd", "playback.loop", "mark.in", "mark.out"})
        if (QAction *a = top->findChild<QAction *>(QString::fromLatin1(id))) found.insert(QString::fromLatin1(id), a);
    if (found.isEmpty()) return;
    setActions(found);
    // Found, not given: a later window may still give its own.
    m_actionsGiven = false;
}

QAction *ScrubStrip::action(const QString &command) const { return m_actions.value(command).data(); }

void ScrubStrip::run(const QString &command) {
    if (m_bus) m_bus->run(command);
}

// ---- text ---------------------------------------------------------------------

QString ScrubStrip::currentText() const {
    return m_bus ? timelineTimecode(m_bus->playhead(), m_bus->fps()) : timelineTimecode(0.0, 24.0);
}

QString ScrubStrip::durationText() const {
    return m_bus ? timelineTimecode(m_bus->duration(), m_bus->fps()) : timelineTimecode(0.0, 24.0);
}

QString ScrubStrip::timeText() const { return QStringLiteral("%1 / %2").arg(currentText(), durationText()); }

QSize ScrubStrip::sizeHint() const { return {360, tokens::layout::scrub}; }

void ScrubStrip::setNarrow(bool narrow) {
    if (narrow == m_narrow) return;
    m_narrow = narrow;
    relayout();
    update();
}

bool ScrubStrip::isNarrow() const { return m_narrow || width() < kFullWidth; }

// ---- geometry -------------------------------------------------------------------

QRect ScrubStrip::currentRect() const {
    const int w = isNarrow() ? ui::snapWidth(tokens::timecodeStrong, currentText()) : kTimecodeWidth;
    return QRect(0, kTransportTop + 6, w, 16);
}

QRect ScrubStrip::durationRect() const {
    if (isNarrow()) return {};
    return QRect(width() - kTimecodeWidth, kTransportTop + 6, kTimecodeWidth, 16);
}

void ScrubStrip::relayout() {
    // The centre group: [in, out] gap [start, previous, play, next, end] gap
    // [loop], 4 apart inside each group; centred between the timecodes.
    const bool narrow = isNarrow();
    const int gap = narrow ? 8 : 12;
    const int groupWidth = 24 + 4 + 24 + gap + (24 + 4 + 24 + 4 + 28 + 4 + 24 + 4 + 24) + gap + 24;
    const int left = narrow ? currentRect().width() + 8 : kTimecodeWidth;
    const int right = narrow ? width() : width() - kTimecodeWidth;
    int x = left + std::max(0, (right - left - groupWidth) / 2);
    const int y24 = kTransportTop + 2, y28 = kTransportTop;
    const auto place = [&](QWidget *w, int size, int after) {
        w->setGeometry(x, size == 28 ? y28 : y24, size, size);
        x += size + after;
    };
    place(m_markIn, 24, 4);
    place(m_markOut, 24, gap);
    place(m_start, 24, 4);
    place(m_previous, 24, 4);
    place(m_play, 28, 4);
    place(m_next, 24, 4);
    place(m_end, 24, gap);
    place(m_loopButton, 24, 0);
}

void ScrubStrip::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    relayout();
}

QRectF ScrubStrip::barRect() const { return QRectF(0, kBarTop, width(), kBarHeight); }

double ScrubStrip::xOf(double seconds) const {
    const double duration = m_bus ? m_bus->duration() : 0.0;
    if (duration <= 0.0) return 0.0;
    return std::clamp(seconds / duration, 0.0, 1.0) * width();
}

double ScrubStrip::timeAt(double x) const {
    const double duration = m_bus ? m_bus->duration() : 0.0;
    if (duration <= 0.0 || width() <= 0) return 0.0;
    return std::clamp(x / width(), 0.0, 1.0) * duration;
}

double ScrubStrip::playheadX() const { return m_bus ? xOf(m_bus->playhead()) : 0.0; }

QList<ReviewModel::MarkerStack> ScrubStrip::stacks() const {
    const ReviewModel *review = m_bus ? m_bus->review() : nullptr;
    const double duration = m_bus ? m_bus->duration() : 0.0;
    if (!review || duration <= 0.0 || width() <= 0) return {};
    return review->markerStacks(width(), duration);
}

QRectF ScrubStrip::markerRect(int index) const {
    const QList<ReviewModel::MarkerStack> all = stacks();
    if (index < 0 || index >= all.size()) return {};
    return QRectF(all.at(index).x - 10, 0, 20, 20);
}

int ScrubStrip::stackAt(const QPointF &pos) const {
    if (pos.y() < 0 || pos.y() >= kLaneHeight) return -1;
    const QList<ReviewModel::MarkerStack> all = stacks();
    // The later marker is drawn on top, so it takes the pointer first.
    for (qsizetype i = all.size() - 1; i >= 0; --i)
        if (std::abs(pos.x() - all.at(i).x) <= 10.0) return int(i);
    return -1;
}

QList<ScrubStrip::Range> ScrubStrip::ranges() const {
    const ReviewModel *review = m_bus ? m_bus->review() : nullptr;
    if (!review || !m_bus || m_bus->duration() <= 0.0) return {};
    QList<Range> out;
    for (const ReviewModel::LaneMarker &marker : review->laneMarkers()) {
        if (!marker.ranged) continue;
        Range range;
        range.id = marker.id;
        const double x0 = xOf(marker.at), x1 = xOf(marker.end);
        range.rect = QRectF(x0, kBarTop, std::max(0.0, x1 - x0), kBarHeight);
        range.current = marker.current;
        range.resolved = marker.resolved;
        // Resolved wins over current, as the mockup's .rng.res comes last.
        range.color = marker.resolved ? rangeGrey(0.25) : marker.current ? color(tokens::accent) : rangeGrey(0.6);
        out << range;
    }
    return out;
}

// ---- refresh ------------------------------------------------------------------------

void ScrubStrip::refresh() {
    if (m_hover >= 0) {
        // The hovered marker's popover follows the review as it changes.
        if (m_hover < stacks().size()) {
            showPopover(m_hover);
        } else {
            m_hover = -1;
            hidePopover();
        }
    }
    if (m_bus) m_shownDuration = m_bus->duration();
    refreshTransport();
    update();
}

void ScrubStrip::refreshTransport() {
    const bool playing = m_bus && m_bus->playing();
    static_cast<PlayButton *>(m_play)->setPlaying(playing);
    // A registry command's button is as its action is: enabled with it,
    // its shortcut in the tip, the reason it is unavailable after its name.
    const auto follow = [this](QAbstractButton *button, const QString &command, const QString &name, bool fallback) {
        const QAction *a = action(command);
        const bool enabled = a ? a->isEnabled() : fallback;
        button->setEnabled(enabled);
        const QString reason = a && !enabled ? a->statusTip() : QString();
        auto *kit = static_cast<ui::KitButton *>(button);
        kit->setTip(reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason),
                    a && enabled ? a->shortcut().toString(QKeySequence::NativeText) : QString());
        button->setAccessibleName(name);
    };
    const bool can = m_bus != nullptr;
    follow(m_start, QStringLiteral("playback.goToStart"), QStringLiteral("Go to start"), can);
    follow(m_previous, QStringLiteral("playback.previousFrame"), QStringLiteral("Previous frame"), can);
    follow(m_play, QStringLiteral("playback.toggle"), playing ? QStringLiteral("Pause") : QStringLiteral("Play"), can);
    follow(m_next, QStringLiteral("playback.nextFrame"), QStringLiteral("Next frame"), can);
    follow(m_end, QStringLiteral("playback.goToEnd"), QStringLiteral("Go to end"), can);
    // The marks and the loop are the strip's own, so they work while the
    // registry's commands for them still wait; their keys show once those
    // commands run.
    const bool hasProgram = m_bus && m_bus->duration() > 0.0;
    const auto own = [this, hasProgram](ui::IconButton *button, const QString &command, const QString &name) {
        button->setEnabled(hasProgram);
        const QAction *a = action(command);
        button->setTip(name, a && a->isEnabled() ? a->shortcut().toString(QKeySequence::NativeText) : QString());
        button->setAccessibleName(name);
    };
    own(m_markIn, QStringLiteral("mark.in"), QStringLiteral("Mark in"));
    own(m_markOut, QStringLiteral("mark.out"), QStringLiteral("Mark out"));
    own(m_loopButton, QStringLiteral("playback.loop"), QStringLiteral("Loop playback"));
}

bool ScrubStrip::event(QEvent *event) {
    const bool handled = QWidget::event(event);
    if (event->type() == QEvent::ParentChange || event->type() == QEvent::Show) attachToWindow();
    if (event->type() == QEvent::Hide) hidePopover();
    return handled;
}

// ---- marks and loop -----------------------------------------------------------------

void ScrubStrip::markIn() {
    if (!m_bus || m_bus->duration() <= 0.0) return;
    m_in = std::clamp(m_bus->playhead(), 0.0, m_bus->duration());
    if (hasOut() && m_out <= m_in) m_out = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    update();
    emit marksChanged();
}

void ScrubStrip::markOut() {
    if (!m_bus || m_bus->duration() <= 0.0) return;
    m_out = std::clamp(m_bus->playhead(), 0.0, m_bus->duration());
    if (hasIn() && m_in >= m_out) m_in = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    update();
    emit marksChanged();
}

void ScrubStrip::clearMarks() {
    if (!hasIn() && !hasOut()) return;
    m_in = m_out = std::numeric_limits<double>::quiet_NaN();
    m_wrapped = false;
    update();
    emit marksChanged();
}

void ScrubStrip::setLoop(bool loop) {
    if (loop == m_loop) return;
    m_loop = loop;
    m_wrapped = false;
    {
        const QSignalBlocker blocker(m_loopButton);
        m_loopButton->setOn(loop);
    }
    emit loopChanged(loop);
}

double ScrubStrip::loopStart() const { return hasIn() ? m_in : 0.0; }

double ScrubStrip::loopEnd() const {
    const double duration = m_bus ? m_bus->duration() : 0.0;
    return hasOut() ? std::min(m_out, duration) : duration;
}

void ScrubStrip::followPlayhead() {
    if (!m_loop || !m_bus || !m_bus->playing()) return;
    const double frame = 1.0 / std::max(1.0, m_bus->fps());
    const double end = loopEnd(), start = loopStart();
    const double t = m_bus->playhead();
    if (t < end - frame / 2) {
        m_wrapped = false;
        return;
    }
    // Playing on past the out mark: back to the in mark, still playing. At
    // the program's own end playback stops, and followPlaying starts it
    // again.
    const bool atProgramEnd = end >= m_bus->duration() - frame;
    if (m_wrapped || atProgramEnd || end - start < frame) return;
    m_wrapped = true;
    seek(start);
}

void ScrubStrip::followPlaying(bool playing) {
    if (playing || !m_loop || !m_bus) return;
    const double frame = 1.0 / std::max(1.0, m_bus->fps());
    const double end = loopEnd(), start = loopStart();
    // Stopped at the loop's end (the program's end, where playback ends by
    // itself, or an out mark the playhead had reached): start again.
    const double slack = std::max(2.0 * frame, 0.1);
    if (end - start < frame || m_bus->playhead() < end - slack) return;
    m_wrapped = true;
    seek(start);
    run(QStringLiteral("playback.toggle"));
}

void ScrubStrip::seek(double seconds) {
    if (!m_bus) return;
    const double t = std::clamp(seconds, 0.0, std::max(0.0, m_bus->duration()));
    if (isSignalConnected(QMetaMethod::fromSignal(&ScrubStrip::seekRequested))) {
        emit seekRequested(t);
        return;
    }
    m_bus->send(QJsonObject{{QStringLiteral("type"), QStringLiteral("seek")},
                            {QStringLiteral("position"), t},
                            {QStringLiteral("scrub"), false}});
}

// ---- the lane's markers -------------------------------------------------------------

void ScrubStrip::openStack(int index) {
    const QList<ReviewModel::MarkerStack> all = stacks();
    if (index < 0 || index >= all.size() || all.at(index).markers.isEmpty()) return;
    const ReviewModel::LaneMarker marker = all.at(index).markers.first();
    hidePopover();
    if (m_bus) {
        if (ReviewModel *review = m_bus->review()) review->setCurrentThread(marker.id);
        emit m_bus->openThread(marker.id);
    }
    seek(marker.at);
}

void ScrubStrip::setHover(int index) {
    if (index == m_hover) return;
    m_hover = index;
    if (index >= 0) showPopover(index);
    else hidePopover();
    update();
}

QWidget *ScrubStrip::popover() const { return m_popover; }

bool ScrubStrip::popoverShown() const { return m_popover && m_popover->isVisible(); }

void ScrubStrip::showPopover(int index) {
    const QList<ReviewModel::MarkerStack> all = stacks();
    if (index < 0 || index >= all.size()) return;
    QWidget *host = window();
    if (!m_popover) m_popover = new MarkerPopover(host);
    else if (m_popover->parentWidget() != host) m_popover->setParent(host);
    auto *pop = static_cast<MarkerPopover *>(m_popover.data());
    pop->setEntries(all.at(index).markers, m_bus ? m_bus->fps() : 24.0);
    // 8 above the marker; x from the marker's left + 8 - 140, kept on the
    // strip (the mockup's showPop), in the window's coordinates.
    const QRectF marker = markerRect(index);
    const QPoint origin = host == this ? QPoint() : mapTo(host, QPoint(0, 0));
    const double left = std::clamp(origin.x() + marker.left() + 8 - 140, double(origin.x()),
                                   double(origin.x() + width() - kPopoverWidth));
    const double top = origin.y() + marker.top() - 8 - pop->cardHeight();
    const QMargins m = ui::shadowMargins();
    pop->move(int(std::lround(left)) - m.left(), int(std::lround(top)) - m.top());
    pop->show();
    pop->raise();
}

void ScrubStrip::hidePopover() {
    if (m_popover) m_popover->hide();
}

// ---- the pointer ----------------------------------------------------------------------

void ScrubStrip::mousePressEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton) return QWidget::mousePressEvent(event);
    const QPointF pos = event->position();
    if (const int index = stackAt(pos); index >= 0) {
        openStack(index);
        event->accept();
        return;
    }
    // The bar with its 4-point slop.
    if (pos.y() >= kBarTop - kBarSlop && pos.y() <= kBarTop + kBarHeight + kBarSlop && m_bus && m_bus->duration() > 0.0) {
        m_dragging = true;
        seek(timeAt(pos.x()));
        event->accept();
        return;
    }
    QWidget::mousePressEvent(event);
}

void ScrubStrip::mouseMoveEvent(QMouseEvent *event) {
    const QPointF pos = event->position();
    if (m_dragging) {
        seek(timeAt(pos.x()));
        return;
    }
    setHover(stackAt(pos));
    setCursor(m_hover >= 0 || (pos.y() >= kBarTop - kBarSlop && pos.y() <= kBarTop + kBarHeight + kBarSlop)
                  ? Qt::PointingHandCursor
                  : Qt::ArrowCursor);
}

void ScrubStrip::mouseReleaseEvent(QMouseEvent *event) {
    if (m_dragging && event->button() == Qt::LeftButton) {
        m_dragging = false;
        event->accept();
        return;
    }
    QWidget::mouseReleaseEvent(event);
}

bool ScrubStrip::eventFilter(QObject *watched, QEvent *event) {
    if (event->type() == QEvent::Enter) setHover(-1);
    return QWidget::eventFilter(watched, event);
}

void ScrubStrip::leaveEvent(QEvent *event) {
    setHover(-1);
    QWidget::leaveEvent(event);
}

// ---- paint ----------------------------------------------------------------------------

void ScrubStrip::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const QRectF bar = barRect();
    // The bar, the played part, the ranges.
    p.setPen(Qt::NoPen);
    p.setBrush(color(tokens::track));
    p.drawRoundedRect(bar, 2, 2);
    const double head = playheadX();
    if (head > 0) {
        p.setBrush(color(tokens::track2));
        p.drawRoundedRect(QRectF(bar.left(), bar.top(), head, bar.height()), 2, 2);
    }
    for (const Range &range : ranges()) {
        p.setBrush(range.color);
        if (range.rect.width() > 0) p.drawRoundedRect(range.rect, 2, 2);
        // The 2 x 8 end tick.
        p.drawRoundedRect(QRectF(range.rect.right() - 2, kBarTop - 2, 2, 8), 1, 1);
    }
    // The marks, as brackets on the bar.
    p.setBrush(color(tokens::ink2));
    if (hasIn()) {
        const double x = xOf(m_in);
        p.drawRect(QRectF(x, kBarTop - 3, 1.5, 10));
        p.drawRect(QRectF(x, kBarTop - 3, 4, 1.5));
    }
    if (hasOut()) {
        const double x = xOf(m_out);
        p.drawRect(QRectF(x - 1.5, kBarTop - 3, 1.5, 10));
        p.drawRect(QRectF(x - 4, kBarTop - 3, 4, 1.5));
    }
    // The playhead: a 2 x 28 line from the top, its 10-point knob on the bar
    // with a 1 pt #0e0e0e ring.
    if (m_bus && m_bus->duration() > 0.0) {
        p.setBrush(color(tokens::playhead));
        p.drawRect(QRectF(head - 1, 0, 2, 28));
        p.setBrush(color(tokens::surround));
        p.drawEllipse(QPointF(head, 26), 6, 6);
        p.setBrush(color(tokens::playhead));
        p.drawEllipse(QPointF(head, 26), 5, 5);
    }

    // The lane's markers, over the playhead.
    const QList<ReviewModel::MarkerStack> all = stacks();
    for (qsizetype i = 0; i < all.size(); ++i) {
        const ReviewModel::MarkerStack &stack = all.at(i);
        const QRectF r(stack.x - 10, 0, 20, 20);
        const bool hovered = int(i) == m_hover;
        if (stack.isStack()) {
            p.setPen(Qt::NoPen);
            p.setBrush(color(tokens::raised));
            p.drawEllipse(r);
            if (hovered) {
                p.setPen(QPen(color(tokens::lineh), 1));
                p.setBrush(Qt::NoBrush);
                p.drawEllipse(r.adjusted(-0.5, -0.5, 0.5, 0.5));
            }
            p.setFont(initialsFont());
            p.setPen(color(tokens::ink));
            p.drawText(r, Qt::AlignCenter, QString::number(stack.markers.size()));
            continue;
        }
        const ReviewModel::LaneMarker &m = stack.markers.first();
        QPainterPath shape;
        if (m.circle) shape.addEllipse(r);
        else shape.addRoundedRect(r, tokens::radius::marker, tokens::radius::marker);
        QColor ink = color(tokens::ink2);
        if (m.resolved) {
            // Transparent, an inset line2, ink3 (it wins over current and
            // hover, as .mk.res does in the mockup).
            ink = color(tokens::ink3);
            QPainterPath inner;
            const QRectF in = r.adjusted(0.5, 0.5, -0.5, -0.5);
            if (m.circle) inner.addEllipse(in);
            else inner.addRoundedRect(in, tokens::radius::marker - 0.5, tokens::radius::marker - 0.5);
            p.setPen(QPen(color(tokens::line2), 1));
            p.setBrush(Qt::NoBrush);
            p.drawPath(inner);
        } else {
            p.setPen(Qt::NoPen);
            p.fillPath(shape, color(tokens::on));
            const auto ring = [&](qreal width, const QColor &c) {
                QPainterPath outer;
                const QRectF out = r.adjusted(-width / 2, -width / 2, width / 2, width / 2);
                if (m.circle) outer.addEllipse(out);
                else outer.addRoundedRect(out, tokens::radius::marker + width / 2, tokens::radius::marker + width / 2);
                p.setPen(QPen(c, width));
                p.setBrush(Qt::NoBrush);
                p.drawPath(outer);
            };
            if (m.current) {
                ring(1.5, color(tokens::ink2));
                ink = color(tokens::ink);
            } else if (hovered) {
                ring(1.0, color(tokens::lineh));
            }
        }
        if (m.glyph) {
            icons::paint(p, m.kind == QStringLiteral("qc") ? QStringLiteral("g-gauge") : QStringLiteral("g-kerf"), r, ink, 14);
        } else {
            p.setFont(initialsFont());
            p.setPen(ink);
            p.drawText(r, Qt::AlignCenter, m.initials);
        }
    }

    // The transport's timecodes.
    p.setFont(tokens::font(tokens::timecodeStrong));
    p.setPen(color(tokens::ink));
    p.drawText(currentRect(), Qt::AlignLeft | Qt::AlignVCenter, currentText());
    if (!isNarrow()) {
        p.setFont(tokens::font(tokens::timecode));
        p.setPen(color(tokens::ink2));
        p.drawText(durationRect(), Qt::AlignRight | Qt::AlignVCenter, durationText());
    }
}

QAbstractButton *ScrubStrip::playButton() const { return m_play; }
QAbstractButton *ScrubStrip::loopButton() const { return m_loopButton; }
QAbstractButton *ScrubStrip::markInButton() const { return m_markIn; }
QAbstractButton *ScrubStrip::markOutButton() const { return m_markOut; }
QAbstractButton *ScrubStrip::startButton() const { return m_start; }
QAbstractButton *ScrubStrip::previousButton() const { return m_previous; }
QAbstractButton *ScrubStrip::nextButton() const { return m_next; }
QAbstractButton *ScrubStrip::endButton() const { return m_end; }

} // namespace kerf
