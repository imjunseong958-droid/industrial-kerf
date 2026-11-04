#include "source_monitor_panel.h"

#include "design_tokens.h"
#include "editor_common.h"
#include "media_artwork.h"
#include "ui/components.h"
#include "ui/source_state.h"

#include <QAction>
#include <QEvent>
#include <QFontMetricsF>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

using tokens::color;

constexpr int kBar = 4;
constexpr int kPad = 12;
constexpr QSize kCanvas{280, 158};
// The narrowest pane keeps the canvas's shape: 240 wide gives 216 x 122.
QSize canvasFor(int width) {
    const int w = std::clamp(width - 2 * kPad, 0, kCanvas.width());
    return QSize(w, int(std::lround(w * double(kCanvas.height()) / kCanvas.width())));
}
constexpr int kKeyValue = 24;
constexpr int kSection = 28;

// The buttons, by command id: their name and the key the design shows when
// no registry action says otherwise.
struct ButtonSpec {
    const char *id;
    const char *name;
    const char *key;
};
constexpr ButtonSpec kButtons[] = {
    {"source.markIn", "Mark in", "I"},
    {"source.markOut", "Mark out", "O"},
    {"source.previousFrame", "Previous frame", "←"},
    {"source.play", "Play source", "Space"},
    {"source.nextFrame", "Next frame", "→"},
    {"source.insert", "Insert", ","},
    {"source.overwrite", "Overwrite", "."},
    {"source.clearMarks", "Clear in and out", ""},
};
const QStringList kTransportIds{QStringLiteral("source.markIn"), QStringLiteral("source.markOut"),
                                QStringLiteral("source.previousFrame"), QStringLiteral("source.play"),
                                QStringLiteral("source.nextFrame")};

void drawText(QPainter &painter, const QRectF &rect, const tokens::TypeStyle &style, QRgb ink, const QString &text,
              Qt::Alignment align = Qt::AlignLeft | Qt::AlignVCenter) {
    painter.setFont(tokens::font(style));
    painter.setPen(color(ink));
    const QString shown = QFontMetricsF(painter.font()).elidedText(text, Qt::ElideRight, rect.width());
    painter.drawText(rect, int(align) | Qt::TextSingleLine, shown);
}

} // namespace

// ---------------------------------------------------------------- Body

// Everything the viewer shows, painted, with the kit's buttons placed on it.
// It scrolls in the pane.
class SourceMonitorPanel::Body final : public QWidget {
public:
    explicit Body(SourceMonitorPanel *panel) : QWidget(panel), m_panel(panel) {
        setObjectName(QStringLiteral("sourceBody"));
        setFocusPolicy(Qt::StrongFocus);
        setMouseTracking(true);
        for (const ButtonSpec &spec : kButtons) {
            const QString id = QString::fromLatin1(spec.id);
            QAbstractButton *button = nullptr;
            if (id == QStringLiteral("source.insert") || id == QStringLiteral("source.overwrite") || id == QStringLiteral("source.clearMarks"))
                button = new ui::Button(QString::fromUtf8(spec.name), ui::Button::Kind::Secondary, this);
            else
                button = new ui::IconButton(iconFor(id), ui::IconButton::Size::S24, this);
            add(id, button);
        }
        m_refresh.setSingleShot(true);
        m_refresh.setInterval(0);
        connect(&m_refresh, &QTimer::timeout, this, &Body::refreshButtons);
    }

    // ---- what it shows -------------------------------------------------------
    SourceState *state() const { return m_panel->m_state; }

    struct Parts {
        QRect box, bar, transport, tcs, section;
        QVector<QRect> keyValues;
        int height = 0;
    };
    Parts parts;

    QAbstractButton *button(const QString &id) const { return m_buttons.value(id); }

    QStringList keyValues() const {
        const SourceState *s = state();
        if (!s || !s->isOpen()) return {QString(), QString(), QString(), QString()};
        const double fps = s->fps();
        return {s->name(), timelineTimecode(s->inPoint(), fps), timelineTimecode(s->outPoint(), fps),
                timelineTimecode(s->outPoint() - s->inPoint(), fps)};
    }

    // ---- layout --------------------------------------------------------------
    void relayout() {
        const int w = width();
        Parts p;
        p.box = QRect(QPoint(kPad, kPad), canvasFor(w));
        const int inner = std::max(0, w - 2 * kPad);
        p.bar = QRect(kPad, p.box.bottom() + 1 + 12, inner, kBar);
        p.tcs = QRect(kPad, p.bar.bottom() + 1 + 8, inner, 16);
        const int transportTop = p.tcs.bottom() + 1 + 8;
        p.transport = QRect(kPad, transportTop, inner, 24);
        const int group = 5 * 24 + 4 * 4;
        int x = kPad + std::max(0, (inner - group) / 2);
        for (const QString &id : kTransportIds) {
            place(id, QPoint(x, transportTop));
            x += 24 + 4;
        }
        // Insert, Overwrite and Clear in and out, wrapping (gap 8).
        int bx = kPad;
        int by = transportTop + 24 + 12;
        for (const QString &id : {QStringLiteral("source.insert"), QStringLiteral("source.overwrite"), QStringLiteral("source.clearMarks")}) {
            QAbstractButton *b = m_buttons.value(id);
            const QSize hint = b->sizeHint();
            if (bx > kPad && bx + hint.width() > kPad + inner) {
                bx = kPad;
                by += 24 + 8;
            }
            b->setGeometry(QRect(QPoint(bx, by), hint));
            bx += hint.width() + 8;
        }
        const int srcpBottom = by + 24 + kPad;
        p.section = QRect(0, srcpBottom + 4, w, kSection);
        for (int i = 0; i < 4; ++i) p.keyValues.append(QRect(0, p.section.bottom() + 1 + i * kKeyValue, w, kKeyValue));
        p.height = p.keyValues.last().bottom() + 1;
        parts = p;
        if (minimumHeight() != p.height) setMinimumHeight(p.height);
        update();
    }

    // ---- the buttons -------------------------------------------------------
    void scheduleRefresh() {
        if (!m_refresh.isActive()) m_refresh.start();
    }

    void refreshButtons() {
        SourceState *s = state();
        const bool open = s && s->isOpen();
        for (const ButtonSpec &spec : kButtons) {
            const QString id = QString::fromLatin1(spec.id);
            QString name = QString::fromUtf8(spec.name);
            QString key = QString::fromUtf8(spec.key);
            QAction *action = s ? s->action(id) : nullptr;
            bool enabled = open;
            QString reason;
            if (id == QStringLiteral("source.play")) {
                // Play keeps the viewer's own Space; a key of the registry's
                // (one's own keymap) shows instead.
                if (action && !action->shortcut().isEmpty()) key = action->shortcut().toString(QKeySequence::NativeText);
                const bool playing = s && s->isPlaying();
                name = playing ? QStringLiteral("Pause source") : QStringLiteral("Play source");
                static_cast<ui::IconButton *>(m_buttons.value(id))->setIconName(playing ? QStringLiteral("g-pause")
                                                                                        : QStringLiteral("g-play"));
                enabled = s && s->canPlay();
                if (open && !enabled) reason = QStringLiteral("This media has no picture");
            } else if (action) {
                enabled = action->isEnabled();
                reason = enabled ? QString() : action->statusTip();
                key = action->shortcut().toString(QKeySequence::NativeText);
            } else if (id == QStringLiteral("source.insert") || id == QStringLiteral("source.overwrite")) {
                // An edit needs the registry: it knows the timeline and its targets.
                enabled = false;
            }
            if (!open && reason.isEmpty()) reason = QStringLiteral("Open media in the source monitor");
            const QString tip = enabled || reason.isEmpty() ? name : QStringLiteral("%1 · %2").arg(name, reason);
            QAbstractButton *b = m_buttons.value(id);
            b->setEnabled(enabled);
            static_cast<ui::KitButton *>(b)->setTip(tip, enabled ? key : QString());
            b->setAccessibleName(name);
        }
    }

    void bindActions() {
        if (SourceState *s = state())
            for (QAction *action : s->actions()) connect(action, &QAction::changed, this, &Body::scheduleRefresh, Qt::UniqueConnection);
        refreshButtons();
    }

    // ---- input -------------------------------------------------------------
    void run(const QString &id) {
        SourceState *s = state();
        if (!s) return;
        // Play and Open in timeline run the registry's commands when it has
        // them (source.play, source.reveal), so they run as the menu and
        // one's own keys do; a viewer without them acts itself.
        if (id == QStringLiteral("source.play")) {
            if (QAction *action = s->action(id); action && action->isEnabled()) action->trigger();
            else s->togglePlay();
            return;
        }
        if (id == QStringLiteral("source.reveal")) {
            if (s->revealClip().isEmpty()) return;
            if (QAction *action = s->action(id); action && action->isEnabled()) action->trigger();
            else emit m_panel->revealRequested();
            return;
        }
        if (QAction *action = s->action(id)) {
            action->trigger();
            return;
        }
        // A viewer with no registry acts on its state itself.
        if (id == QStringLiteral("source.markIn")) s->markIn();
        else if (id == QStringLiteral("source.markOut")) s->markOut();
        else if (id == QStringLiteral("source.previousFrame")) s->step(-1);
        else if (id == QStringLiteral("source.nextFrame")) s->step(1);
        else if (id == QStringLiteral("source.clearMarks")) s->clearMarks();
    }

    bool event(QEvent *event) override {
        // Space plays the source and ⇧F opens it in the timeline while the
        // viewer has the focus, ahead of the window's own Space.
        if (event->type() == QEvent::ShortcutOverride) {
            auto *key = static_cast<QKeyEvent *>(event);
            if (ownKey(key)) {
                event->accept();
                return true;
            }
        }
        return QWidget::event(event);
    }

    void keyPressEvent(QKeyEvent *event) override {
        if (ownKey(event)) {
            run(event->key() == Qt::Key_Space ? QStringLiteral("source.play") : QStringLiteral("source.reveal"));
            event->accept();
            return;
        }
        QWidget::keyPressEvent(event);
    }

    void mousePressEvent(QMouseEvent *event) override {
        setFocus(Qt::MouseFocusReason);
        if (event->button() != Qt::LeftButton) return;
        const QPoint at = event->position().toPoint();
        if (seekArea(parts.bar).contains(at)) {
            m_seeking = true;
            seekAt(parts.bar, event->position().x());
        }
    }

    void mouseMoveEvent(QMouseEvent *event) override {
        if (m_seeking) seekAt(parts.bar, event->position().x());
        setCursor(seekArea(parts.bar).contains(event->position().toPoint()) ? Qt::PointingHandCursor : Qt::ArrowCursor);
    }

    void mouseReleaseEvent(QMouseEvent *) override { m_seeking = false; }

    void resizeEvent(QResizeEvent *) override { relayout(); }

    QString timecodeText() const {
        const SourceState *s = state();
        return timelineTimecode(s ? std::max(0.0, s->position()) : 0.0, s ? s->fps() : 24.0);
    }

    // ---- painting ------------------------------------------------------------
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        const SourceState *s = state();
        const bool open = s && s->isOpen();
        painter.fillRect(rect(), color(tokens::chrome));
        paintPicture(painter, parts.box);
        // The bar shows the marked range only; its row names the in and out marks.
        paintBar(painter, parts.bar);
        const auto [inText, outText] = compactTimecodes();
        drawText(painter, parts.tcs, tokens::timecode, tokens::ink, inText);
        if (open) drawText(painter, parts.tcs, tokens::monoSmall, tokens::ink3, outText, Qt::AlignRight | Qt::AlignVCenter);
        drawText(painter, parts.section.adjusted(12, 0, -12, 0), tokens::controlStrong, tokens::ink, QStringLiteral("Source clip"));
        static const QStringList labels{QStringLiteral("Name"), QStringLiteral("In"), QStringLiteral("Out"), QStringLiteral("Duration")};
        const QStringList values = keyValues();
        for (int i = 0; i < parts.keyValues.size(); ++i) {
            const QRect row = parts.keyValues.at(i).adjusted(12, 0, -12, 0);
            const int labelWidth = ui::snapWidth(tokens::control, labels.at(i));
            drawText(painter, row, tokens::control, tokens::ink2, labels.at(i));
            drawText(painter, row.adjusted(labelWidth + 8, 0, 0, 0), tokens::timecode, tokens::ink, values.at(i),
                     Qt::AlignRight | Qt::AlignVCenter);
        }
    }

    QPair<QString, QString> compactTimecodes() const {
        const SourceState *s = state();
        if (!s || !s->isOpen()) return {timecodeText(), QString()};
        return {timelineTimecode(s->inPoint(), s->fps()), timelineTimecode(s->outPoint(), s->fps())};
    }

private:
    static QString iconFor(const QString &id) {
        if (id == QStringLiteral("source.markIn")) return QStringLiteral("g-markin");
        if (id == QStringLiteral("source.markOut")) return QStringLiteral("g-markout");
        if (id == QStringLiteral("source.previousFrame")) return QStringLiteral("g-prev");
        if (id == QStringLiteral("source.play")) return QStringLiteral("g-play");
        return QStringLiteral("g-next");
    }

    void add(const QString &id, QAbstractButton *button) {
        button->setObjectName(id);
        button->setFocusPolicy(Qt::NoFocus);
        connect(button, &QAbstractButton::clicked, this, [this, id] { run(id); });
        m_buttons.insert(id, button);
    }

    void place(const QString &id, QPoint at) {
        QAbstractButton *b = m_buttons.value(id);
        b->setGeometry(QRect(at, QSize(24, 24)));
        b->show();
    }

    // Space and ⇧F are the viewer's only while they would do something;
    // otherwise they reach the window's shortcuts.
    bool ownKey(const QKeyEvent *key) const {
        const SourceState *s = state();
        if (!s) return false;
        const Qt::KeyboardModifiers mods = key->modifiers() & ~Qt::KeypadModifier;
        if (key->key() == Qt::Key_Space && mods == Qt::NoModifier) return s->canPlay();
        if (key->key() == Qt::Key_F && mods == Qt::ShiftModifier) return !s->revealClip().isEmpty();
        return false;
    }

    static QRect seekArea(const QRect &bar) { return bar.adjusted(0, -6, 0, 6); }

    void seekAt(const QRect &track, qreal x) {
        SourceState *s = state();
        if (!s || !s->isOpen() || track.width() <= 0) return;
        const double fraction = std::clamp((x - track.left()) / track.width(), 0.0, 1.0);
        s->seek(fraction * s->duration());
    }

    // The frame fitted into the canvas on black, else the waveform of sound,
    // else what the state says.
    void paintPicture(QPainter &painter, const QRect &box) {
        const SourceState *s = state();
        painter.save();
        QPainterPath clip;
        clip.addRoundedRect(QRectF(box), tokens::radius::small, tokens::radius::small);
        painter.setClipPath(clip);
        painter.fillRect(box, color(tokens::overlay::deviceScreen));
        const QImage frame = s ? s->frame() : QImage();
        const MediaArtwork *artwork = s ? s->artwork() : nullptr;
        if (!frame.isNull()) {
            const QSize fitted = frame.size().scaled(box.size(), Qt::KeepAspectRatio);
            const QRect target(QPoint(box.left() + (box.width() - fitted.width()) / 2, box.top() + (box.height() - fitted.height()) / 2),
                               fitted);
            painter.setRenderHint(QPainter::SmoothPixmapTransform);
            painter.drawImage(target, frame);
        } else if (s && s->isOpen() && !s->hasVideo() && artwork && !artwork->peaks.value(s->mediaId()).isEmpty()) {
            const AudioPeaks &peaks = artwork->peaks[s->mediaId()];
            drawWaveform(painter, QRectF(box).adjusted(8, box.height() * 0.25, -8, -box.height() * 0.25), peaks, 0.0,
                         s->duration(), color(tokens::overlay::waveformPeak), color(tokens::overlay::waveformRms),
                         peaks.displayGain());
        } else {
            const QString message = s && s->isOpen() && !s->hasVideo() ? s->name() : s ? s->message() : QString();
            painter.setFont(tokens::font(tokens::control));
            painter.setPen(color(tokens::ink3));
            painter.drawText(QRectF(box).adjusted(16, 0, -16, 0), Qt::AlignCenter | Qt::TextWordWrap, message);
        }
        painter.restore();
    }

    void paintBar(QPainter &painter, const QRect &bar) {
        const SourceState *s = state();
        painter.setPen(Qt::NoPen);
        painter.setBrush(color(tokens::track));
        painter.drawRoundedRect(QRectF(bar), 2, 2);
        if (!s || !s->isOpen() || s->duration() <= 0.0) return;
        const auto x = [&](double seconds) { return bar.left() + seconds / s->duration() * bar.width(); };
        painter.setBrush(color(tokens::track2));
        painter.drawRoundedRect(QRectF(QPointF(x(s->inPoint()), bar.top()), QPointF(x(s->outPoint()), bar.bottom() + 1)), 2, 2);
    }

    SourceMonitorPanel *m_panel;
    QHash<QString, QAbstractButton *> m_buttons;
    QTimer m_refresh;
    bool m_seeking = false;
};

// ---------------------------------------------------------------- SourceMonitorPanel

SourceMonitorPanel::SourceMonitorPanel(SourceState *state, QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("sourceMonitor"));
    setMinimumSize(0, 0);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->setSizeConstraint(QLayout::SetNoConstraint);
    m_body = new Body(this);
    m_scroll = new ui::FadeScrollArea(this);
    m_scroll->setObjectName(QStringLiteral("sourcePane"));
    m_scroll->setFocusPolicy(Qt::NoFocus);
    m_scroll->setFadeColor(tokens::color(tokens::chrome));
    m_scroll->viewport()->setAutoFillBackground(false);
    m_scroll->setWidget(m_body);
    layout->addWidget(m_scroll);
    setFocusProxy(m_body);
    setFocusPolicy(Qt::StrongFocus);
    if (!state) {
        m_ownState = new SourceState(this);
        state = m_ownState;
    }
    m_state = state;
    attach();
}

SourceMonitorPanel::~SourceMonitorPanel() = default;

SourceState *SourceMonitorPanel::state() const {
    return m_state;
}

void SourceMonitorPanel::attach() {
    SourceState *state = m_state;
    if (!state) return;
    connect(state, &SourceState::changed, this, [this] {
        m_body->update();
        m_body->scheduleRefresh();
        emit changed();
    });
    connect(state, &SourceState::frameChanged, m_body, qOverload<>(&QWidget::update));
    connect(state, &SourceState::artworkChanged, m_body, qOverload<>(&QWidget::update));
    connect(state, &SourceState::opened, this, [this] {
        m_body->relayout();
        m_body->scheduleRefresh();
    });
    connect(state, &SourceState::playingChanged, m_body, &Body::scheduleRefresh);
    connect(state, &SourceState::revealChanged, m_body, &Body::scheduleRefresh);
    connect(state, &SourceState::actionsChanged, this, [this] {
        addSourceActions();
        m_body->bindActions();
    });
    addSourceActions();
    m_body->bindActions();
    m_body->relayout();
}

void SourceMonitorPanel::addSourceActions() {
    // The registry's source-scoped keys work while this viewer has the focus
    // too (the same actions, one more widget each).
    if (!m_state) return;
    for (QAction *action : m_state->actions())
        if (action->shortcutContext() == Qt::WidgetWithChildrenShortcut && !actions().contains(action)) addAction(action);
}

void SourceMonitorPanel::setState(SourceState *state) {
    if (!state || state == m_state) return;
    if (m_state) {
        m_state->disconnect(this);
        m_state->disconnect(m_body);
        // The previous state's keys no longer act on this viewer.
        for (QAction *action : m_state->actions()) removeAction(action);
    }
    m_state = state;
    if (m_ownState && m_ownState != state) {
        delete m_ownState;
        m_ownState = nullptr;
    }
    attach();
    emit changed();
}

QSize SourceMonitorPanel::sizeHint() const {
    return {tokens::layout::inspector, m_body->parts.height > 0 ? m_body->parts.height : 418};
}

// ---- the state, as before

void SourceMonitorPanel::open(const QJsonObject &source, const QString &path) {
    if (m_state) m_state->open(source, path);
    setFocus();
}

void SourceMonitorPanel::close() {
    if (m_state) m_state->close();
}

bool SourceMonitorPanel::isOpen() const { return m_state && m_state->isOpen(); }
QString SourceMonitorPanel::mediaId() const { return m_state ? m_state->mediaId() : QString(); }
QJsonObject SourceMonitorPanel::source() const { return m_state ? m_state->source() : QJsonObject(); }
double SourceMonitorPanel::fps() const { return m_state ? m_state->fps() : 24.0; }
double SourceMonitorPanel::position() const { return m_state ? m_state->position() : 0.0; }
double SourceMonitorPanel::inPoint() const { return m_state ? m_state->inPoint() : 0.0; }
double SourceMonitorPanel::outPoint() const { return m_state ? m_state->outPoint() : 0.0; }
QImage SourceMonitorPanel::frame() const { return m_state ? m_state->frame() : QImage(); }

void SourceMonitorPanel::seek(double seconds) {
    if (m_state) m_state->seek(seconds);
}
void SourceMonitorPanel::step(int frames) {
    if (m_state) m_state->step(frames);
}
void SourceMonitorPanel::markIn() {
    if (m_state) m_state->markIn();
}
void SourceMonitorPanel::markOut() {
    if (m_state) m_state->markOut();
}
void SourceMonitorPanel::clearMarks() {
    if (m_state) m_state->clearMarks();
}

void SourceMonitorPanel::setActions(const QList<QAction *> &list) {
    if (!m_state) return;
    QHash<QString, QAction *> byId;
    // The registry's source-scoped commands it gave this viewer, then those passed.
    for (QAction *action : actions())
        if (action && action->objectName().startsWith(QLatin1String("source."))) byId.insert(action->objectName(), action);
    for (QAction *action : list)
        if (action && !action->objectName().isEmpty()) byId.insert(action->objectName(), action);
    m_state->addActions(byId);
}

// ---- geometry

QRect SourceMonitorPanel::pictureRect() const { return QRect(m_body->mapTo(this, m_body->parts.box.topLeft()), m_body->parts.box.size()); }
QRect SourceMonitorPanel::barRect() const { return QRect(m_body->mapTo(this, m_body->parts.bar.topLeft()), m_body->parts.bar.size()); }
QRect SourceMonitorPanel::transportRect() const {
    return QRect(m_body->mapTo(this, m_body->parts.transport.topLeft()), m_body->parts.transport.size());
}
QPair<QString, QString> SourceMonitorPanel::compactTimecodes() const { return m_body->compactTimecodes(); }
QRect SourceMonitorPanel::sectionRect() const {
    return QRect(m_body->mapTo(this, m_body->parts.section.topLeft()), m_body->parts.section.size());
}
QRect SourceMonitorPanel::keyValueRect(int index) const {
    if (index < 0 || index >= m_body->parts.keyValues.size()) return {};
    const QRect row = m_body->parts.keyValues.at(index);
    return QRect(m_body->mapTo(this, row.topLeft()), row.size());
}
QAbstractButton *SourceMonitorPanel::button(const QString &id) const { return m_body->button(id); }
QStringList SourceMonitorPanel::keyValues() const { return m_body->keyValues(); }
QWidget *SourceMonitorPanel::body() const { return m_body; }

} // namespace kerf
