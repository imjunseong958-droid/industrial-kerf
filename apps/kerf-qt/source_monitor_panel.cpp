#include "source_monitor_panel.h"

#include "design_tokens.h"
#include "editor_common.h"
#include "media_artwork.h"
#include "ui/components.h"
#include "ui/source_state.h"

#include <QAction>
#include <QEvent>
#include <QGraphicsOpacityEffect>
#include <QFontMetricsF>
#include <QKeyEvent>
#include <QLinearGradient>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QTimer>
#include <QVariantAnimation>
#include <QVBoxLayout>
#include <QWheelEvent>

#include <algorithm>
#include <cmath>

namespace kerf {

namespace {

using tokens::color;

constexpr int kHeader = 40;     // header height
constexpr int kInset = 16;      // the card's side padding
constexpr int kPictureFoot = 280; // what the column keeps under the picture
constexpr int kBar = 4;
constexpr int kTransport = 28;
constexpr int kFilmstrip = 40;
constexpr int kFrameWidth = 71; // filmstrip frames, with a 1 pt gap
constexpr int kRow = 28;
constexpr int kFooter = 48;
constexpr int kLengthWidth = 88; // the transport's marked length
constexpr int kRowFade = 16;
// Compact.
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
    {"source.reveal", "Open in timeline", "⇧F"},
    {"source.markIn", "Mark in", "I"},
    {"source.markOut", "Mark out", "O"},
    {"source.previousFrame", "Previous frame", "←"},
    {"source.play", "Play source", "Space"},
    {"source.nextFrame", "Next frame", "→"},
    {"source.insert", "Insert", ","},
    {"source.overwrite", "Overwrite", "."},
    {"source.clearMarks", "Clear in and out", "⇧⌘X"},
};
const QString kClearCompact = QStringLiteral("source.clearMarks.compact");
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
// In the stage form it fills the viewer; in the compact form it scrolls.
class SourceMonitorPanel::Body final : public QWidget {
public:
    explicit Body(SourceMonitorPanel *panel) : QWidget(panel), m_panel(panel) {
        setObjectName(QStringLiteral("sourceBody"));
        setFocusPolicy(Qt::StrongFocus);
        setMouseTracking(true);
        for (const ButtonSpec &spec : kButtons) {
            const QString id = QString::fromLatin1(spec.id);
            QAbstractButton *button = nullptr;
            if (id == QStringLiteral("source.insert") || id == QStringLiteral("source.overwrite"))
                button = new ui::Button(QString::fromUtf8(spec.name), ui::Button::Kind::Secondary, this);
            else if (id == QStringLiteral("source.clearMarks"))
                button = new ui::GhostButton(QString::fromUtf8(spec.name), this);
            else
                button = new ui::IconButton(iconFor(id), ui::IconButton::Size::S24, this);
            add(id, button);
        }
        add(kClearCompact, new ui::Button(QStringLiteral("Clear in and out"), ui::Button::Kind::Secondary, this));
        m_refresh.setSingleShot(true);
        m_refresh.setInterval(0);
        connect(&m_refresh, &QTimer::timeout, this, &Body::refreshButtons);
    }

    // ---- what it shows -------------------------------------------------------
    SourceState *state() const { return m_panel->m_state; }
    Mode mode() const { return m_panel->m_mode; }
    bool stage() const { return mode() == Mode::Stage; }

    struct Parts {
        QRect header, box, picture, bar, transport, tc, length, filmstrip, ranges, footer;
        QRect tcs, section;
        QVector<QRect> keyValues;
        int height = 0;
    };
    Parts parts;

    QAbstractButton *button(const QString &id) const {
        if (id == QStringLiteral("source.clearMarks") && !stage()) return m_buttons.value(kClearCompact);
        return m_buttons.value(id);
    }

    QRect rowRect(int index) const {
        return QRect(0, parts.ranges.top() + index * kRow - m_scroll, width(), kRow);
    }
    int rowCount() const { return state() ? int(state()->ranges().size()) : 0; }

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
        if (stage()) {
            const StagePicture picture = stagePicture(size());
            p.header = QRect(0, 0, w, kHeader);
            p.box = picture.box;
            p.picture = picture.picture;
            p.bar = QRect(kInset, p.box.bottom() + 1 + 12, std::max(0, w - 2 * kInset), kBar);
            p.transport = QRect(0, p.bar.bottom() + 1 + 4, w, kTransport);
            p.filmstrip = QRect(kInset, p.transport.bottom() + 1 + 12, std::max(0, w - 2 * kInset), kFilmstrip);
            p.footer = QRect(0, height() - kFooter, w, kFooter);
            const int rowsTop = p.filmstrip.bottom() + 1 + 8;
            p.ranges = QRect(0, rowsTop, w, std::max(0, p.footer.top() - rowsTop));
            p.height = height();
            // The header: Open in timeline at its right (pad 0 12 0 16).
            place(QStringLiteral("source.reveal"), QPoint(w - 12 - 24, (kHeader - 24) / 2));
            // The transport: the position, then the five buttons centred in
            // what is left before the marked length.
            const QString tcText = timecodeText();
            const int tcWidth = ui::snapWidth(tokens::timecode, tcText.isEmpty() ? QStringLiteral("00:00:00:00") : tcText);
            p.tc = QRect(kInset, p.transport.top(), tcWidth, kTransport);
            p.length = QRect(w - kInset - kLengthWidth, p.transport.top(), kLengthWidth, kTransport);
            const int group = 5 * 24 + 4 * 4;
            const int from = p.tc.right() + 1 + 4;
            const int to = p.length.left() - 4;
            int x = from + std::max(0, (to - from - group) / 2);
            for (const QString &id : kTransportIds) {
                place(id, QPoint(x, p.transport.top() + (kTransport - 24) / 2));
                x += 24 + 4;
            }
            // The footer: Insert and Overwrite, then Clear in and out at the right.
            const int y = p.footer.top() + (kFooter - 24) / 2;
            QAbstractButton *insert = m_buttons.value(QStringLiteral("source.insert"));
            QAbstractButton *overwrite = m_buttons.value(QStringLiteral("source.overwrite"));
            QAbstractButton *clear = m_buttons.value(QStringLiteral("source.clearMarks"));
            insert->setGeometry(QRect(QPoint(kInset, y), insert->sizeHint()));
            overwrite->setGeometry(QRect(QPoint(insert->geometry().right() + 1 + 8, y), overwrite->sizeHint()));
            clear->setGeometry(QRect(QPoint(w - kInset - clear->sizeHint().width(), y), clear->sizeHint()));
            m_buttons.value(kClearCompact)->hide();
            clear->show();
        } else {
            p.box = QRect(QPoint(kPad, kPad), canvasFor(w));
            p.picture = p.box;
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
            for (const QString &id : {QStringLiteral("source.insert"), QStringLiteral("source.overwrite"), kClearCompact}) {
                QAbstractButton *b = m_buttons.value(id);
                const QSize hint = b->sizeHint();
                if (bx > kPad && bx + hint.width() > kPad + inner) {
                    bx = kPad;
                    by += 24 + 8;
                }
                b->setGeometry(QRect(QPoint(bx, by), hint));
                bx += hint.width() + 8;
            }
            m_buttons.value(QStringLiteral("source.clearMarks"))->hide();
            m_buttons.value(kClearCompact)->show();
            const int srcpBottom = by + 24 + kPad;
            p.section = QRect(0, srcpBottom + 4, w, kSection);
            for (int i = 0; i < 4; ++i) p.keyValues.append(QRect(0, p.section.bottom() + 1 + i * kKeyValue, w, kKeyValue));
            p.height = p.keyValues.last().bottom() + 1;
        }
        m_buttons.value(QStringLiteral("source.reveal"))->setVisible(stage());
        parts = p;
        clampScroll();
        if (!stage() && minimumHeight() != p.height) setMinimumHeight(p.height);
        if (stage() && minimumHeight() != 0) setMinimumHeight(0);
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
            const bool own = id == QStringLiteral("source.play") || id == QStringLiteral("source.reveal");
            // Play and Open in timeline keep the viewer's own Space and ⇧F;
            // a key of the registry's (one's own keymap) shows instead.
            if (own && action && !action->shortcut().isEmpty()) key = action->shortcut().toString(QKeySequence::NativeText);
            if (id == QStringLiteral("source.reveal")) {
                enabled = open && !s->revealClip().isEmpty();
                if (open && !enabled) reason = QStringLiteral("This media is not in the timeline");
            } else if (id == QStringLiteral("source.play")) {
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
            for (QAbstractButton *b : buttonsFor(id)) {
                b->setEnabled(enabled);
                static_cast<ui::KitButton *>(b)->setTip(tip, enabled ? key : QString());
                b->setAccessibleName(name);
            }
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
        const QString command = id == kClearCompact ? QStringLiteral("source.clearMarks") : id;
        if (QAction *action = s->action(command)) {
            action->trigger();
            return;
        }
        // A viewer with no registry acts on its state itself.
        if (command == QStringLiteral("source.markIn")) s->markIn();
        else if (command == QStringLiteral("source.markOut")) s->markOut();
        else if (command == QStringLiteral("source.previousFrame")) s->step(-1);
        else if (command == QStringLiteral("source.nextFrame")) s->step(1);
        else if (command == QStringLiteral("source.clearMarks")) s->clearMarks();
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
            m_drag = Drag::Bar;
            seekAt(parts.bar, event->position().x());
        } else if (stage() && parts.filmstrip.contains(at)) {
            m_drag = Drag::Filmstrip;
            seekAt(parts.filmstrip, event->position().x());
        } else if (stage() && parts.ranges.contains(at)) {
            const int row = rowAt(at);
            if (row >= 0) activateRow(row);
        }
    }

    void mouseMoveEvent(QMouseEvent *event) override {
        const QPoint at = event->position().toPoint();
        if (m_drag == Drag::Bar) seekAt(parts.bar, event->position().x());
        else if (m_drag == Drag::Filmstrip) seekAt(parts.filmstrip, event->position().x());
        setHover(stage() && parts.ranges.contains(at) ? rowAt(at) : -1);
        const bool pointing = seekArea(parts.bar).contains(at) || (stage() && parts.filmstrip.contains(at)) || m_hover >= 0;
        setCursor(pointing ? Qt::PointingHandCursor : Qt::ArrowCursor);
    }

    void mouseReleaseEvent(QMouseEvent *) override { m_drag = Drag::None; }
    void leaveEvent(QEvent *) override { setHover(-1); }

    void wheelEvent(QWheelEvent *event) override {
        if (!stage() || !parts.ranges.contains(event->position().toPoint())) {
            event->ignore();
            return;
        }
        const int delta = !event->pixelDelta().isNull() ? event->pixelDelta().y() : event->angleDelta().y() * kRow / 120;
        m_scroll -= delta;
        clampScroll();
        setHover(rowAt(event->position().toPoint()));
        update();
        event->accept();
    }

    void resizeEvent(QResizeEvent *) override { relayout(); }

    void activateRow(int row) {
        SourceState *s = state();
        if (!s || row < 0 || row >= rowCount()) return;
        const SourceState::Range range = s->ranges().at(row);
        s->setRange(range.start, range.end);
        s->seek(range.start);
    }

    QString timecodeText() const {
        const SourceState *s = state();
        return timelineTimecode(s ? std::max(0.0, s->position()) : 0.0, s ? s->fps() : 24.0);
    }

    // ---- painting ------------------------------------------------------------
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        if (stage()) paintStage(painter);
        else paintCompact(painter);
    }

private:
    enum class Drag { None, Bar, Filmstrip };

    static QString iconFor(const QString &id) {
        if (id == QStringLiteral("source.reveal")) return QStringLiteral("g-chevr");
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

    QList<QAbstractButton *> buttonsFor(const QString &id) const {
        if (id == QStringLiteral("source.clearMarks")) return {m_buttons.value(id), m_buttons.value(kClearCompact)};
        return {m_buttons.value(id)};
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

    int rowAt(QPoint at) const {
        if (!parts.ranges.contains(at)) return -1;
        const int row = (at.y() - parts.ranges.top() + m_scroll) / kRow;
        return row >= 0 && row < rowCount() ? row : -1;
    }

    void setHover(int row) {
        if (row == m_hover) return;
        m_hover = row;
        update(parts.ranges);
    }

    void clampScroll() {
        const int content = rowCount() * kRow;
        m_scroll = std::clamp(m_scroll, 0, std::max(0, content - parts.ranges.height()));
    }

    // The frame fitted into `area` on black, else the waveform of sound,
    // else what the state says.
    void paintPicture(QPainter &painter, const QRect &box, const QRect &area, qreal radius) {
        const SourceState *s = state();
        painter.save();
        QPainterPath clip;
        clip.addRoundedRect(QRectF(box), radius, radius);
        painter.setClipPath(clip);
        painter.fillRect(box, color(tokens::overlay::deviceScreen));
        const QImage frame = s ? s->frame() : QImage();
        const MediaArtwork *artwork = s ? s->artwork() : nullptr;
        if (!frame.isNull()) {
            const QSize fitted = frame.size().scaled(area.size(), Qt::KeepAspectRatio);
            const QRect target(QPoint(area.left() + (area.width() - fitted.width()) / 2, area.top() + (area.height() - fitted.height()) / 2),
                               fitted);
            painter.setRenderHint(QPainter::SmoothPixmapTransform);
            painter.drawImage(target, frame);
        } else if (s && s->isOpen() && !s->hasVideo() && artwork && !artwork->peaks.value(s->mediaId()).isEmpty()) {
            const AudioPeaks &peaks = artwork->peaks[s->mediaId()];
            drawWaveform(painter, QRectF(area).adjusted(8, area.height() * 0.25, -8, -area.height() * 0.25), peaks, 0.0,
                         s->duration(), color(tokens::overlay::waveformPeak), color(tokens::overlay::waveformRms),
                         peaks.displayGain());
        } else {
            const QString message = s && s->isOpen() && !s->hasVideo() ? s->name() : s ? s->message() : QString();
            painter.setFont(tokens::font(tokens::control));
            painter.setPen(color(tokens::ink3));
            painter.drawText(QRectF(area).adjusted(16, 0, -16, 0), Qt::AlignCenter | Qt::TextWordWrap, message);
        }
        painter.restore();
    }

    void paintBar(QPainter &painter, const QRect &bar, bool tick) {
        const SourceState *s = state();
        painter.setPen(Qt::NoPen);
        painter.setBrush(color(tokens::track));
        painter.drawRoundedRect(QRectF(bar), 2, 2);
        if (!s || !s->isOpen() || s->duration() <= 0.0) return;
        const auto x = [&](double seconds) { return bar.left() + seconds / s->duration() * bar.width(); };
        painter.setBrush(color(tokens::track2));
        painter.drawRoundedRect(QRectF(QPointF(x(s->inPoint()), bar.top()), QPointF(x(s->outPoint()), bar.bottom() + 1)), 2, 2);
        if (!tick) return;
        painter.setBrush(color(tokens::playhead));
        painter.drawRoundedRect(QRectF(x(std::max(0.0, s->position())) - 1, bar.top() - 4, 2, 12), 1, 1);
    }

    void paintFilmstrip(QPainter &painter, const QRect &strip) {
        const SourceState *s = state();
        if (strip.width() <= 0) return;
        painter.save();
        QPainterPath clip;
        clip.addRoundedRect(QRectF(strip), tokens::radius::small, tokens::radius::small);
        painter.setClipPath(clip);
        const MediaArtwork *artwork = s ? s->artwork() : nullptr;
        const QString media = s ? s->mediaId() : QString();
        const Filmstrip film = artwork && s->isOpen() ? artwork->strips.value(media) : Filmstrip{};
        const double duration = s ? s->duration() : 0.0;
        if (s && s->isOpen() && !s->hasVideo()) {
            painter.fillRect(strip, color(tokens::raised));
            if (artwork && !artwork->peaks.value(media).isEmpty()) {
                const AudioPeaks &peaks = artwork->peaks[media];
                drawWaveform(painter, QRectF(strip).adjusted(0, 4, 0, -4), peaks, 0.0, duration,
                             color(tokens::overlay::waveformPeak), color(tokens::overlay::waveformRms), peaks.displayGain());
            }
        } else {
            painter.setRenderHint(QPainter::SmoothPixmapTransform);
            const int count = (strip.width() + kFrameWidth) / (kFrameWidth + 1);
            for (int i = 0; i < count; ++i) {
                const QRect tile(strip.left() + i * (kFrameWidth + 1), strip.top(), kFrameWidth, kFilmstrip);
                const int index = duration > 0.0 ? film.frameAt((i * (kFrameWidth + 1) + kFrameWidth / 2.0) / strip.width() * duration) : -1;
                if (index < 0 || index >= film.frames.size()) {
                    painter.fillRect(tile, color(tokens::raised));
                    continue;
                }
                const QImage &frame = film.frames.at(index);
                const QSize cover = frame.size().scaled(tile.size(), Qt::KeepAspectRatioByExpanding);
                const QRect target(QPoint(tile.center().x() + 1 - cover.width() / 2, tile.center().y() + 1 - cover.height() / 2), cover);
                painter.save();
                painter.setClipRect(tile, Qt::IntersectClip);
                painter.drawImage(target, frame);
                painter.restore();
            }
        }
        painter.restore();
        ui::paintInsetLine(painter, QRectF(strip), tokens::radius::small, color(tokens::line));
        if (s && s->isOpen() && duration > 0.0) {
            const qreal left = strip.left() + s->inPoint() / duration * strip.width();
            const qreal right = strip.left() + s->outPoint() / duration * strip.width();
            ui::paintInsetLine(painter, QRectF(left, strip.top(), std::max<qreal>(3, right - left), strip.height()),
                               tokens::radius::small, color(tokens::ink), 1.5);
        }
    }

    // A shot's span names its last frame, so neighbouring shots do not read
    // as overlapping (the range itself stays end-exclusive).
    static QString spanText(double start, double end, double fps) {
        const double last = std::max(start, end - 1.0 / fps);
        return QStringLiteral("%1–%2").arg(timelineTimecode(start, fps), timelineTimecode(last, fps));
    }

public:
    QString rowSpan(int index) const {
        const SourceState *s = state();
        if (!s || !s->isOpen() || index < 0 || index >= s->ranges().size()) return {};
        const SourceState::Range range = s->ranges().at(index);
        return spanText(range.start, range.end, s->fps());
    }

    QPair<QString, QString> compactTimecodes() const {
        const SourceState *s = state();
        if (!s || !s->isOpen()) return {timecodeText(), QString()};
        return {timelineTimecode(s->inPoint(), s->fps()), timelineTimecode(s->outPoint(), s->fps())};
    }

private:
    void paintRows(QPainter &painter) {
        const SourceState *s = state();
        const QRect area = parts.ranges;
        if (!s || !s->isOpen() || area.height() <= 0) return;
        painter.save();
        painter.setClipRect(area);
        const QVector<SourceState::Range> ranges = s->ranges();
        if (ranges.isEmpty()) {
            if (!s->indexMessage().isEmpty())
                drawText(painter, QRectF(kInset, area.top(), width() - 2 * kInset, kRow), tokens::control, tokens::ink3, s->indexMessage());
            painter.restore();
            return;
        }
        const double fps = s->fps();
        painter.setFont(tokens::font(tokens::monoSmall));
        for (int i = 0; i < ranges.size(); ++i) {
            const QRect row = rowRect(i);
            if (row.bottom() < area.top() || row.top() > area.bottom()) continue;
            if (i == m_hover) painter.fillRect(row, color(tokens::raised));
            const SourceState::Range &range = ranges.at(i);
            const QString span = spanText(range.start, range.end, fps);
            const int spanWidth = ui::snapWidth(tokens::monoSmall, span);
            drawText(painter, QRectF(width() - kInset - spanWidth, row.top(), spanWidth, kRow), tokens::monoSmall, tokens::ink3, span,
                     Qt::AlignRight | Qt::AlignVCenter);
            drawText(painter, QRectF(kInset, row.top(), std::max(0, width() - 2 * kInset - spanWidth - 8), kRow), tokens::control,
                     tokens::ink, range.name);
        }
        // The rows fade out into the card at the bottom.
        QLinearGradient fade(0, area.bottom() + 1 - kRowFade, 0, area.bottom() + 1);
        QColor clear = color(tokens::card);
        clear.setAlpha(0);
        fade.setColorAt(0, clear);
        fade.setColorAt(1, color(tokens::card));
        painter.fillRect(QRect(area.left(), area.bottom() + 1 - kRowFade, area.width(), kRowFade), fade);
        painter.restore();
    }

    void paintStage(QPainter &painter) {
        const SourceState *s = state();
        const bool open = s && s->isOpen();
        QPainterPath card;
        card.addRoundedRect(QRectF(rect()), tokens::radius::panel, tokens::radius::panel);
        painter.fillPath(card, color(tokens::card));
        painter.save();
        painter.setClipPath(card);
        // Header: Source, then the media's name and length.
        const int titleWidth = ui::snapWidth(tokens::title, QStringLiteral("Source"));
        drawText(painter, QRectF(kInset, 0, titleWidth, kHeader), tokens::title, tokens::ink, QStringLiteral("Source"));
        if (open) {
            const QString meta = QStringLiteral("%1 · %2").arg(s->name(), timelineTimecode(s->duration(), s->fps()));
            const qreal left = kInset + titleWidth + 8;
            drawText(painter, QRectF(left, 0, std::max<qreal>(0, width() - 12 - 24 - 8 - left), kHeader), tokens::meta, tokens::ink3, meta);
        }
        paintPicture(painter, parts.box, parts.picture, 0);
        paintBar(painter, parts.bar, true);
        drawText(painter, parts.tc, tokens::timecode, tokens::ink, timecodeText());
        if (open)
            drawText(painter, parts.length, tokens::timecode, tokens::ink2, timelineTimecode(s->outPoint() - s->inPoint(), s->fps()),
                     Qt::AlignRight | Qt::AlignVCenter);
        paintFilmstrip(painter, parts.filmstrip);
        paintRows(painter);
        // Footer: a top hairline.
        painter.fillRect(QRect(0, parts.footer.top(), width(), 1), color(tokens::line));
        painter.restore();
        ui::paintInsetLine(painter, QRectF(rect()), tokens::radius::panel, color(tokens::line));
    }

    void paintCompact(QPainter &painter) {
        const SourceState *s = state();
        const bool open = s && s->isOpen();
        painter.fillRect(rect(), color(tokens::chrome));
        paintPicture(painter, parts.box, parts.picture, tokens::radius::small);
        // The compact bar shows the marked range only; its row names the in
        // and out marks.
        paintBar(painter, parts.bar, false);
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

    SourceMonitorPanel *m_panel;
    QHash<QString, QAbstractButton *> m_buttons;
    QTimer m_refresh;
    Drag m_drag = Drag::None;
    int m_hover = -1;
    int m_scroll = 0;
};

// ---------------------------------------------------------------- SourceMonitorPanel

SourceMonitorPanel::StagePicture SourceMonitorPanel::stagePicture(const QSize &size) {
    const int w = std::max(0, size.width() - 2 * kInset);
    const int most = size.height() - kPictureFoot;
    const int fit = int(std::lround(w * 9.0 / 16.0));
    StagePicture out;
    if (fit <= most) {
        out.box = QRect(kInset, kHeader, w, fit);
        out.picture = out.box;
    } else {
        const int k = std::max(0, most) / 9;
        const int pictureWidth = std::min(w, 16 * k);
        out.box = QRect(kInset, kHeader, w, 9 * k);
        out.picture = QRect(kInset + (w - pictureWidth) / 2, kHeader, pictureWidth, 9 * k);
    }
    return out;
}

SourceMonitorPanel::SourceMonitorPanel(QWidget *parent) : SourceMonitorPanel(nullptr, Mode::Stage, parent) {}

SourceMonitorPanel::SourceMonitorPanel(SourceState *state, Mode mode, QWidget *parent) : QWidget(parent), m_mode(mode) {
    setObjectName(QStringLiteral("sourceMonitor"));
    setMinimumSize(0, 0);
    // The body fills the viewer (stage), or scrolls in the pane (compact).
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
    m_scroll->hide();
    layout->addWidget(m_body);
    layout->addWidget(m_scroll);
    setFocusProxy(m_body);
    setFocusPolicy(Qt::StrongFocus);
    if (!state) {
        m_ownState = new SourceState(this);
        state = m_ownState;
    }
    m_state = state;
    attach();
    m_mode = Mode::Stage;
    setMode(mode);
}

SourceMonitorPanel::~SourceMonitorPanel() = default;

void SourceMonitorPanel::showEvent(QShowEvent *event) {
    QWidget::showEvent(event);
    // The stage column fades in (mockup #srcv, the 160 ms fade) as it comes
    // into a window already on screen; with reduced motion it is there at
    // once, and a viewer that is a window of its own just shows. The
    // effect lives only while it fades.
    if (m_mode != Mode::Stage || isWindow() || !window()->isVisible() || event->spontaneous()) return;
    const int duration = ui::motionDuration(tokens::motion::fade);
    if (duration <= 0) return;
    if (!m_fade) {
        m_fade = new QVariantAnimation(this);
        m_fade->setStartValue(0.0);
        m_fade->setEndValue(1.0);
        m_fade->setEasingCurve(tokens::easing());
        connect(m_fade, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
            if (auto *effect = qobject_cast<QGraphicsOpacityEffect *>(graphicsEffect())) effect->setOpacity(value.toDouble());
        });
        connect(m_fade, &QVariantAnimation::finished, this, [this] { setGraphicsEffect(nullptr); });
    }
    m_fade->stop();
    m_fade->setDuration(duration);
    auto *effect = new QGraphicsOpacityEffect(this);
    effect->setOpacity(0.0);
    setGraphicsEffect(effect);
    m_fade->start();
}

bool SourceMonitorPanel::isFading() const {
    return m_fade && m_fade->state() == QAbstractAnimation::Running;
}

qreal SourceMonitorPanel::opacity() const {
    const auto *effect = qobject_cast<const QGraphicsOpacityEffect *>(graphicsEffect());
    return effect ? effect->opacity() : 1.0;
}

SourceState *SourceMonitorPanel::state() const {
    return m_state;
}

void SourceMonitorPanel::attach() {
    SourceState *state = m_state;
    if (!state) return;
    const auto refresh = [this] {
        m_body->relayout();
        m_body->scheduleRefresh();
    };
    connect(state, &SourceState::changed, this, [this] {
        // The position's timecode can change the transport's widths.
        if (m_mode == Mode::Stage) m_body->relayout();
        else m_body->update();
        m_body->scheduleRefresh();
        emit changed();
    });
    connect(state, &SourceState::frameChanged, m_body, qOverload<>(&QWidget::update));
    connect(state, &SourceState::artworkChanged, m_body, qOverload<>(&QWidget::update));
    connect(state, &SourceState::indexChanged, this, refresh);
    connect(state, &SourceState::opened, this, refresh);
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

void SourceMonitorPanel::setMode(Mode mode) {
    if (mode == m_mode) return;
    m_mode = mode;
    auto *box = static_cast<QVBoxLayout *>(layout());
    if (mode == Mode::Compact) {
        box->removeWidget(m_body);
        m_scroll->setWidget(m_body);
        m_body->show();
        m_scroll->show();
    } else {
        if (m_scroll->widget() == m_body) m_scroll->takeWidget();
        m_body->setParent(this);
        box->insertWidget(0, m_body);
        m_body->show();
        m_scroll->hide();
    }
    m_body->setMinimumHeight(0);
    m_body->relayout();
    m_body->scheduleRefresh();
    updateGeometry();
}

QSize SourceMonitorPanel::sizeHint() const {
    if (m_mode == Mode::Compact) return {tokens::layout::inspector, m_body->parts.height > 0 ? m_body->parts.height : 418};
    return {400, 548};
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
QRect SourceMonitorPanel::pictureArea() const {
    return QRect(m_body->mapTo(this, m_body->parts.picture.topLeft()), m_body->parts.picture.size());
}
QRect SourceMonitorPanel::headerRect() const { return m_body->parts.header; }
QRect SourceMonitorPanel::barRect() const { return QRect(m_body->mapTo(this, m_body->parts.bar.topLeft()), m_body->parts.bar.size()); }
QRect SourceMonitorPanel::transportRect() const {
    return QRect(m_body->mapTo(this, m_body->parts.transport.topLeft()), m_body->parts.transport.size());
}
QRect SourceMonitorPanel::filmstripRect() const { return m_body->parts.filmstrip; }
QRect SourceMonitorPanel::rangesRect() const { return m_body->parts.ranges; }
QRect SourceMonitorPanel::rangeRowRect(int index) const { return m_body->rowRect(index); }
int SourceMonitorPanel::rangeCount() const { return m_body->rowCount(); }
QString SourceMonitorPanel::rangeSpan(int index) const { return m_body->rowSpan(index); }
QPair<QString, QString> SourceMonitorPanel::compactTimecodes() const { return m_body->compactTimecodes(); }
QRect SourceMonitorPanel::footerRect() const { return m_body->parts.footer; }
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
