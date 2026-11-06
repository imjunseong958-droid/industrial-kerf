#include "graph_editor.h"

#include "components.h"
#include "design_tokens.h"
#include "editor_bus.h"
#include "tray_shell.h"

#include <QContextMenuEvent>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QLineF>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace kerf {

using graph::Channel;
using graph::Handles;
using graph::Key;

namespace {

constexpr int kBar = 28;
constexpr int kList = 176;
constexpr int kRuler = 20;
constexpr int kRow = 24;
constexpr int kInset = 12;
constexpr double kGrab = 7.0;
constexpr double kDiamond = 4.5;
// A hand that wobbles while clicking has not moved anything.
constexpr double kDragStart = 3.0;

// Tick spacing, in frames, that leaves `room` points between labels.
qint64 tickStep(double pointsPerFrame, double room) {
    qint64 step = 1;
    for (const qint64 scale : {1, 10, 100, 1000, 10000, 100000})
        for (const qint64 base : {1, 2, 5})
            if (double(base * scale) * pointsPerFrame >= room) return base * scale;
    return step * 1000000;
}

QPainterPath diamond(QPointF at, double size) {
    QPainterPath path;
    path.moveTo(at.x(), at.y() - size);
    path.lineTo(at.x() + size, at.y());
    path.lineTo(at.x(), at.y() + size);
    path.lineTo(at.x() - size, at.y());
    path.closeSubpath();
    return path;
}

} // namespace

GraphEditor::GraphEditor(QWidget *parent, EditorBus *bus) : QWidget(parent), m_bus(bus) {
    setObjectName(QStringLiteral("graphEditor"));
    setFocusPolicy(Qt::ClickFocus);
    setMouseTracking(false);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    auto *bar = new QHBoxLayout;
    bar->setContentsMargins(kInset, 0, kInset, 0);
    bar->setSpacing(8);
    bar->addWidget(shell::text(QStringLiteral("Graph"), tokens::controlStrong, tokens::ink));
    m_meta = shell::text(QString(), tokens::meta, tokens::ink3);
    m_meta->setObjectName(QStringLiteral("graphMeta"));
    bar->addWidget(m_meta);
    bar->addStretch();
    m_curve = new ui::Button(QStringLiteral("Curve"));
    m_curve->setObjectName(QStringLiteral("graphCurve"));
    m_curve->setFocusPolicy(Qt::NoFocus);
    bar->addWidget(m_curve);
    connect(m_curve, &QAbstractButton::clicked, this, [this] {
        auto *menu = new QMenu(this);
        menu->setAttribute(Qt::WA_DeleteOnClose);
        populateCurveMenu(menu);
        ui::popupMenu(menu, m_curve);
    });
    auto *barWidget = new QWidget;
    barWidget->setFixedHeight(kBar);
    barWidget->setLayout(bar);
    layout->addWidget(barWidget);
    layout->addStretch();

    if (m_bus) {
        connect(m_bus, &EditorBus::snapshotChanged, this, &GraphEditor::rebuild);
        connect(m_bus, &EditorBus::selectionChanged, this, &GraphEditor::rebuild);
        connect(m_bus, &EditorBus::playheadChanged, this, [this] { movePlayhead(); });
        // A refused edit brings no new state, so what the drag left is taken back here.
        connect(m_bus, &EditorBus::event, this, [this](const QString &type) {
            if (type == QLatin1String("error") && m_drag == Drag::None) rebuild();
        });
    }
    rebuild();
}

// ------------------------------------------------------------ the document

void GraphEditor::rebuild() {
    if (m_drag != Drag::None) return;
    const QString channelId = m_channel >= 0 && m_channel < m_channels.size() ? m_channels.at(m_channel).id() : QString();
    const qint64 keyFrame = channelId.isEmpty() || m_key < 0 || m_key >= m_channels.at(m_channel).keys.size()
                                ? -1
                                : m_channels.at(m_channel).keys.at(m_key).frame;
    const QString previous = m_clipId;
    m_clipId.clear();
    m_channels.clear();
    QString name;
    if (m_bus && !m_bus->selection().isEmpty()) {
        const QString wanted = m_bus->selection().last();
        for (const QJsonValue &track : m_bus->timeline().value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = value.toObject();
                if (clip.value(QStringLiteral("id")).toString() != wanted) continue;
                m_clipId = wanted;
                m_clipStart = clip.value(QStringLiteral("timelineStart")).toDouble();
                m_duration = std::max<qint64>(1, qint64(clip.value(QStringLiteral("durationFrames")).toDouble()));
                m_channels = graph::channelsOf(clip);
                name = clip.value(QStringLiteral("name")).toString(wanted);
            }
    }
    m_channel = -1;
    m_key = -1;
    if (m_clipId == previous)
        for (int index = 0; index < m_channels.size(); ++index)
            if (m_channels.at(index).id() == channelId) {
                m_channel = index;
                const QList<Key> &keys = m_channels.at(index).keys;
                for (int key = 0; key < keys.size(); ++key)
                    if (keys.at(key).frame == keyFrame) m_key = key;
            }
    if (m_channel < 0 && !m_channels.isEmpty()) m_channel = 0;
    fitRanges();
    shell::setTextStyle(m_meta, tokens::meta, tokens::ink3);
    m_meta->setText(m_clipId.isEmpty() ? QString() : QStringLiteral("%1 · %2 frames").arg(name).arg(m_duration));
    updateBar();
    m_playheadX = -1.0;
    update();
}

void GraphEditor::fitRanges() {
    m_ranges.clear();
    for (const Channel &channel : std::as_const(m_channels)) {
        auto [low, high] = graph::reach(channel.keys);
        if (high - low < 1e-9) {
            const double room = std::max(std::abs(low) * 0.5, 0.5);
            low -= room;
            high += room;
        }
        const double margin = (high - low) * 0.12;
        m_ranges.insert(channel.id(), {low - margin, high + margin});
    }
}

void GraphEditor::updateBar() {
    m_curve->setEnabled(m_channel >= 0 && m_key >= 0);
}

void GraphEditor::commit(int channel) {
    if (channel < 0 || channel >= m_channels.size()) return;
    emit editRequested(QJsonArray{graph::editOf(m_clipId, m_channels.at(channel), graph::keysJson(m_channels.at(channel).keys))});
}

void GraphEditor::selectKey(int channel, int key) {
    m_channel = channel;
    m_key = key;
    updateBar();
    update();
}

void GraphEditor::removeSelectedKey() {
    if (m_channel < 0 || m_key < 0) return;
    Channel &channel = m_channels[m_channel];
    const double value = channel.keys.at(m_key).value;
    channel.keys.removeAt(m_key);
    m_key = -1;
    updateBar();
    update();
    // The last key gone: the value it had stays, still.
    emit editRequested(QJsonArray{graph::editOf(m_clipId, channel, channel.keys.isEmpty() ? QJsonValue(value) : graph::keysJson(channel.keys))});
}

void GraphEditor::addKey(int index, qint64 frame) {
    if (index < 0 || index >= m_channels.size()) return;
    Channel &channel = m_channels[index];
    frame = std::clamp<qint64>(frame, 0, m_duration - 1);
    int place = 0;
    while (place < channel.keys.size() && channel.keys.at(place).frame < frame) ++place;
    if (place < channel.keys.size() && channel.keys.at(place).frame == frame) return selectKey(index, place);
    const QString interp = place > 0 ? channel.keys.at(place - 1).interp : QStringLiteral("linear");
    channel.keys.insert(place, Key{frame, graph::valueAt(channel.keys, double(frame)), interp});
    selectKey(index, place);
    commit(index);
}

void GraphEditor::setInterp(const QString &interp) {
    if (m_channel < 0 || m_key < 0) return;
    m_channels[m_channel].keys[m_key].interp = interp;
    fitRanges();
    update();
    commit(m_channel);
}

void GraphEditor::populateCurveMenu(QMenu *menu) {
    ui::styleMenu(menu);
    const QString current = m_channel >= 0 && m_key >= 0 ? m_channels.at(m_channel).keys.at(m_key).interp : QString();
    const std::pair<const char *, const char *> choices[] = {
        {"Linear", "linear"}, {"Hold", "hold"}, {"Ease", "ease"}, {"Ease In", "ease-in"}, {"Ease Out", "ease-out"},
    };
    for (const auto &[label, interp] : choices) {
        QAction *action = menu->addAction(QString::fromUtf8(label));
        action->setCheckable(true);
        action->setChecked(current == QLatin1String(interp));
        const QString name = QString::fromUtf8(interp);
        connect(action, &QAction::triggered, this, [this, name] { setInterp(name); });
    }
    menu->addSeparator();
    connect(menu->addAction(QStringLiteral("Delete Key")), &QAction::triggered, this, &GraphEditor::removeSelectedKey);
}

// ---------------------------------------------------------------- geometry

QRectF GraphEditor::listRect() const { return QRectF(0, kBar, kList, height() - kBar); }
QRectF GraphEditor::rulerRect() const { return QRectF(kList, kBar, width() - kList, kRuler); }

QRectF GraphEditor::plot() const {
    return QRectF(kList + kInset, kBar + kRuler + kInset, std::max(1, width() - kList - 2 * kInset), std::max(1, height() - kBar - kRuler - 2 * kInset));
}

double GraphEditor::xOf(double frame) const {
    const QRectF area = plot();
    return area.left() + frame * area.width() / double(std::max<qint64>(1, m_duration - 1));
}

double GraphEditor::frameOf(double x) const {
    const QRectF area = plot();
    return (x - area.left()) * double(std::max<qint64>(1, m_duration - 1)) / area.width();
}

double GraphEditor::yOf(int channel, double value) const {
    const QRectF area = plot();
    const auto [low, high] = m_ranges.value(m_channels.at(channel).id(), {0.0, 1.0});
    return area.bottom() - (value - low) / (high - low) * area.height();
}

double GraphEditor::valueOf(int channel, double y) const {
    const QRectF area = plot();
    const auto [low, high] = m_ranges.value(m_channels.at(channel).id(), {0.0, 1.0});
    return low + (area.bottom() - y) / area.height() * (high - low);
}

int GraphEditor::channelAt(QPointF point) const {
    if (!listRect().contains(point)) return -1;
    const int row = int((point.y() - kBar) / kRow);
    return row >= 0 && row < m_channels.size() ? row : -1;
}

std::pair<int, int> GraphEditor::keyAt(QPointF point) const {
    // The selected channel's keys come first, since they are on top.
    std::pair<int, int> found{-1, -1};
    double best = kGrab;
    const auto look = [&](int channel) {
        const QList<Key> &keys = m_channels.at(channel).keys;
        for (int key = 0; key < keys.size(); ++key) {
            const QPointF at(xOf(double(keys.at(key).frame)), yOf(channel, keys.at(key).value));
            const double distance = QLineF(at, point).length();
            if (distance < best) {
                best = distance;
                found = {channel, key};
            }
        }
    };
    if (m_channel >= 0) look(m_channel);
    if (found.first >= 0) return found;
    for (int channel = 0; channel < m_channels.size(); ++channel) look(channel);
    return found;
}

QStringList GraphEditor::channelLabels() const {
    QStringList labels;
    for (const Channel &channel : m_channels) labels << channel.label;
    return labels;
}

QPointF GraphEditor::keyPoint(int channel, int key) const {
    const Key &k = m_channels.at(channel).keys.at(key);
    return {xOf(double(k.frame)), yOf(channel, k.value)};
}

QPointF GraphEditor::channelPoint(int channel) const { return {kList / 2.0, kBar + (channel + 0.5) * kRow}; }
QPointF GraphEditor::rulerPoint(double frame) const { return {xOf(frame), kBar + kRuler / 2.0}; }
QPointF GraphEditor::plotPoint(int channel, double frame, double value) const { return {xOf(frame), yOf(channel, value)}; }

std::optional<std::pair<QPointF, QPointF>> GraphEditor::handlePoints() const {
    if (m_channel < 0 || m_key < 0) return std::nullopt;
    const QList<Key> &keys = m_channels.at(m_channel).keys;
    if (m_key + 1 >= keys.size()) return std::nullopt;
    const Key &left = keys.at(m_key);
    const Key &right = keys.at(m_key + 1);
    // Between keys of one value the curve is flat, whatever its handles say.
    if (std::abs(right.value - left.value) < 1e-12) return std::nullopt;
    const auto handles = graph::handlesOf(left.interp);
    if (!handles) return std::nullopt;
    const auto at = [&](double x, double y) {
        return QPointF(xOf(double(left.frame) + x * double(right.frame - left.frame)), yOf(m_channel, left.value + y * (right.value - left.value)));
    };
    return std::pair{at(handles->x1, handles->y1), at(handles->x2, handles->y2)};
}

QString GraphEditor::message() const {
    if (m_clipId.isEmpty()) return QStringLiteral("Select a clip to see how its values move.");
    if (m_channels.isEmpty()) return QStringLiteral("Nothing on this clip moves yet. Add a keyframe from the Inspector.");
    return {};
}

// ---------------------------------------------------------------- painting

void GraphEditor::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), QColor::fromRgba(tokens::chrome));
    const QRectF body(kList, kBar, width() - kList, height() - kBar);
    painter.fillRect(body, QColor::fromRgba(tokens::well));

    const QString note = message();
    if (!note.isEmpty()) {
        painter.setFont(tokens::font(tokens::control));
        painter.setPen(QColor::fromRgba(tokens::ink3));
        painter.drawText(rect().adjusted(0, kBar, 0, 0), Qt::AlignCenter, note);
        return;
    }

    // The list of what moves.
    painter.setFont(tokens::font(tokens::control));
    for (int index = 0; index < m_channels.size(); ++index) {
        const QRectF row(0, kBar + index * kRow, kList, kRow);
        if (index == m_channel) painter.fillRect(row, QColor::fromRgba(tokens::on));
        painter.setPen(QColor::fromRgba(index == m_channel ? tokens::ink : tokens::ink2));
        const QString elided = painter.fontMetrics().elidedText(m_channels.at(index).label, Qt::ElideMiddle, kList - 2 * kInset);
        painter.drawText(row.adjusted(kInset, 0, -kInset, 0), Qt::AlignVCenter | Qt::AlignLeft, elided);
    }

    // The ruler and the grid.
    const QRectF area = plot();
    const double perFrame = area.width() / double(std::max<qint64>(1, m_duration - 1));
    const qint64 step = tickStep(perFrame, 56.0);
    painter.setFont(tokens::font(tokens::monoSmall));
    for (qint64 frame = 0; frame < m_duration; frame += step) {
        const double x = xOf(double(frame));
        painter.setPen(QColor::fromRgba(tokens::line));
        painter.drawLine(QLineF(x, area.top(), x, area.bottom()));
        painter.setPen(QColor::fromRgba(tokens::ink3));
        painter.drawText(QPointF(x + 3, kBar + 14), QString::number(frame));
    }
    if (m_channel >= 0) {
        // The selected number's scale.
        for (int line = 0; line <= 4; ++line) {
            const double y = area.top() + area.height() * line / 4.0;
            painter.setPen(QColor::fromRgba(tokens::line));
            painter.drawLine(QLineF(area.left(), y, area.right(), y));
            painter.setPen(QColor::fromRgba(tokens::ink3));
            painter.drawText(QPointF(area.left() + 3, y - 3), QString::number(valueOf(m_channel, y), 'g', 4));
        }
    }

    // The curves, the selected one last.
    painter.setRenderHint(QPainter::Antialiasing, true);
    const auto drawCurve = [&](int index) {
        const QList<Key> &keys = m_channels.at(index).keys;
        const bool selected = index == m_channel;
        QPainterPath path;
        path.moveTo(area.left(), yOf(index, keys.first().value));
        for (double x = xOf(double(keys.first().frame)); x < xOf(double(keys.last().frame)); x += 2.0)
            path.lineTo(x, yOf(index, graph::valueAt(keys, frameOf(x))));
        path.lineTo(xOf(double(keys.last().frame)), yOf(index, keys.last().value));
        path.lineTo(area.right(), yOf(index, keys.last().value));
        painter.setPen(QPen(QColor::fromRgba(selected ? tokens::ink : tokens::track2), selected ? 1.5 : 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(path);
        for (int key = 0; key < keys.size(); ++key) {
            const QPointF at = keyPoint(index, key);
            const bool on = selected && key == m_key;
            painter.fillPath(diamond(at, on ? kDiamond + 1.0 : kDiamond), QColor::fromRgba(on ? tokens::accent : selected ? tokens::overlay::keyframe : tokens::track2));
        }
    };
    for (int index = 0; index < m_channels.size(); ++index)
        if (index != m_channel) drawCurve(index);
    if (m_channel >= 0) drawCurve(m_channel);

    // The selected key's handles.
    if (const auto handles = handlePoints()) {
        const QPointF from = keyPoint(m_channel, m_key);
        const QPointF to = keyPoint(m_channel, m_key + 1);
        painter.setPen(QPen(QColor::fromRgba(tokens::accentInk), 1.0));
        painter.drawLine(QLineF(from, handles->first));
        painter.drawLine(QLineF(to, handles->second));
        painter.setBrush(QColor::fromRgba(tokens::accentInk));
        painter.setPen(Qt::NoPen);
        painter.drawEllipse(handles->first, 3.5, 3.5);
        painter.drawEllipse(handles->second, 3.5, 3.5);
    }

    // The playhead.
    if (m_bus) {
        const double frame = (m_bus->playhead() - m_clipStart) * m_bus->fps();
        m_playheadX = xOf(frame);
        if (frame >= 0.0 && frame <= double(m_duration)) {
            painter.setRenderHint(QPainter::Antialiasing, false);
            painter.setPen(QColor::fromRgba(tokens::playhead));
            painter.drawLine(QLineF(m_playheadX, kBar, m_playheadX, height()));
        }
    }
}

void GraphEditor::movePlayhead() {
    if (!isVisible() || m_clipId.isEmpty() || !m_bus) return;
    // Only what the line left and where it lands.
    const double x = xOf((m_bus->playhead() - m_clipStart) * m_bus->fps());
    if (m_playheadX >= 0.0) update(QRectF(m_playheadX - 2, kBar, 4, height() - kBar).toAlignedRect());
    update(QRectF(x - 2, kBar, 4, height() - kBar).toAlignedRect());
}

void GraphEditor::resizeEvent(QResizeEvent *event) {
    QWidget::resizeEvent(event);
    update();
}

// ----------------------------------------------------------------- pointer

void GraphEditor::seekTo(double x) {
    if (!m_bus || m_bus->fps() <= 0.0) return;
    const double frame = std::clamp(std::round(frameOf(x)), 0.0, double(m_duration - 1));
    emit seekRequested(m_clipStart + frame / m_bus->fps());
}

void GraphEditor::mousePressEvent(QMouseEvent *event) {
    const QPointF point = event->position();
    m_pressed = point;
    m_changed = false;
    if (event->button() != Qt::LeftButton || !message().isEmpty()) return;
    if (const int row = channelAt(point); row >= 0) {
        selectKey(row, -1);
        return;
    }
    if (rulerRect().contains(point)) {
        m_drag = Drag::Ruler;
        seekTo(point.x());
        return;
    }
    if (const auto handles = handlePoints()) {
        if (QLineF(handles->first, point).length() < kGrab) {
            m_drag = Drag::Handle1;
            m_held = handles->first - point;
            return;
        }
        if (QLineF(handles->second, point).length() < kGrab) {
            m_drag = Drag::Handle2;
            m_held = handles->second - point;
            return;
        }
    }
    const auto [channel, key] = keyAt(point);
    if (channel >= 0) {
        selectKey(channel, key);
        m_grabbed = m_channels.at(channel).keys.at(key);
        m_drag = Drag::Key;
        m_held = keyPoint(channel, key) - point;
        return;
    }
    if (m_key >= 0) selectKey(m_channel, -1);
}

void GraphEditor::mouseMoveEvent(QMouseEvent *event) {
    const QPointF pointer = event->position();
    if (m_drag == Drag::None) return;
    if (m_drag == Drag::Ruler) {
        seekTo(pointer.x());
        return;
    }
    if (!m_changed && QLineF(pointer, m_pressed).length() < kDragStart) return;
    const QPointF point = pointer + m_held;
    switch (m_drag) {
    case Drag::None:
    case Drag::Ruler: return;
    case Drag::Key: {
        qint64 frame = std::llround(frameOf(point.x()));
        double value = valueOf(m_channel, point.y());
        if (event->modifiers() & Qt::ShiftModifier) {
            // Along whichever way it has gone further.
            const QPointF gone = pointer - m_pressed;
            if (std::abs(gone.x()) > std::abs(gone.y())) value = m_grabbed.value;
            else frame = m_grabbed.frame;
        }
        Channel &channel = m_channels[m_channel];
        channel.keys = graph::moved(channel.keys, m_key, frame, value, m_duration);
        m_changed = true;
        update();
        return;
    }
    case Drag::Handle1:
    case Drag::Handle2: {
        Channel &channel = m_channels[m_channel];
        const Key &left = channel.keys.at(m_key);
        const Key &right = channel.keys.at(m_key + 1);
        const auto handles = graph::handlesOf(left.interp);
        if (!handles) return;
        Handles shaped = *handles;
        const double x = (frameOf(point.x()) - double(left.frame)) / double(right.frame - left.frame);
        const double y = (valueOf(m_channel, point.y()) - left.value) / (right.value - left.value);
        if (m_drag == Drag::Handle1) {
            shaped.x1 = x;
            shaped.y1 = y;
        } else {
            shaped.x2 = x;
            shaped.y2 = y;
        }
        channel.keys[m_key].interp = graph::interpOf(shaped);
        m_changed = true;
        update();
        return;
    }
    }
}

void GraphEditor::mouseReleaseEvent(QMouseEvent *) {
    const bool changed = m_changed && (m_drag == Drag::Key || m_drag == Drag::Handle1 || m_drag == Drag::Handle2);
    m_drag = Drag::None;
    m_changed = false;
    if (changed) {
        // The plot keeps still during a drag; the new reach is fitted after it.
        fitRanges();
        update();
        commit(m_channel);
    }
}

void GraphEditor::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !message().isEmpty() || !plot().contains(event->position())) return;
    if (keyAt(event->position()).first >= 0) return;
    addKey(m_channel, std::llround(frameOf(event->position().x())));
}

void GraphEditor::contextMenuEvent(QContextMenuEvent *event) {
    const auto [channel, key] = keyAt(QPointF(event->pos()));
    if (channel < 0) return;
    selectKey(channel, key);
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    populateCurveMenu(menu);
    menu->popup(event->globalPos());
}

void GraphEditor::keyPressEvent(QKeyEvent *event) {
    if ((event->key() == Qt::Key_Delete || event->key() == Qt::Key_Backspace) && m_key >= 0) {
        removeSelectedKey();
        return;
    }
    QWidget::keyPressEvent(event);
}

} // namespace kerf
