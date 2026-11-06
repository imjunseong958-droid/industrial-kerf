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
#include <QWheelEvent>

#include <algorithm>
#include <cmath>
#include <limits>

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
// The fewest frames the plot shows across, however far it zooms.
constexpr double kFewestFrames = 8.0;
constexpr double kZoomPerNotch = 1.25;
constexpr double kPanPerNotch = 48.0;
// A framed key stands this share of what is framed in from the edge.
constexpr double kFrameMargin = 0.1;
// The value axis's zoom: no closer than a thousandth across, the document holding
// four places, and no further out than this many times what the number fits.
constexpr double kFewestValues = 0.001;
constexpr double kWidestValues = 100.0;

// Tick spacing, in frames, that leaves `room` points between labels.
qint64 tickStep(double pointsPerFrame, double room) {
    qint64 step = 1;
    for (const qint64 scale : {1, 10, 100, 1000, 10000, 100000})
        for (const qint64 base : {1, 2, 5})
            if (double(base * scale) * pointsPerFrame >= room) return base * scale;
    return step * 1000000;
}

// The F key, whichever input source is on. With Hangul or Cyrillic it arrives as
// a letter of that alphabet, and only the key's own code says which key it is.
bool isFKey(const QKeyEvent *event) {
    if (event->key() == Qt::Key_F) return true;
#ifdef Q_OS_MACOS
    constexpr quint32 kVK_ANSI_F = 0x03;
    return event->key() > 0x7f && event->key() < Qt::Key_Escape && event->nativeVirtualKey() == kVK_ANSI_F;
#else
    return false;
#endif
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
    auto *frame = new ui::Button(QStringLiteral("Frame"));
    frame->setObjectName(QStringLiteral("graphFrame"));
    frame->setToolTip(QStringLiteral("Show the selected keys (F)"));
    frame->setFocusPolicy(Qt::NoFocus);
    bar->addWidget(frame);
    connect(frame, &QAbstractButton::clicked, this, &GraphEditor::frameSelection);
    auto *fit = new ui::Button(QStringLiteral("Fit"));
    fit->setObjectName(QStringLiteral("graphFit"));
    fit->setToolTip(QStringLiteral("Show the whole clip and fit the values to the plot (Shift+F)"));
    fit->setFocusPolicy(Qt::NoFocus);
    bar->addWidget(fit);
    connect(fit, &QAbstractButton::clicked, this, &GraphEditor::showWholeClip);
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

void GraphEditor::setEffectCatalog(const QJsonArray &catalog) {
    m_effectParams.clear();
    for (const QJsonValue &value : catalog) {
        const QJsonObject def = value.toObject();
        m_effectParams.insert(def.value(QStringLiteral("id")).toString(), def.value(QStringLiteral("params")).toArray());
    }
    rebuild();
}

void GraphEditor::rebuild() {
    if (m_drag != Drag::None) return;
    const QString channelId = m_channel >= 0 && m_channel < m_channels.size() ? m_channels.at(m_channel).id() : QString();
    const qint64 keyFrame = channelId.isEmpty() || m_key < 0 || m_key >= m_channels.at(m_channel).keys.size()
                                ? -1
                                : m_channels.at(m_channel).keys.at(m_key).frame;
    // What is selected is a number and a frame, which outlast the lists they are in.
    QHash<QString, QList<qint64>> selectedFrames;
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it)
        for (const int key : it.value())
            if (it.key() < m_channels.size() && key < m_channels.at(it.key()).keys.size())
                selectedFrames[m_channels.at(it.key()).id()].append(m_channels.at(it.key()).keys.at(key).frame);
    const QString previous = m_clipId;
    const bool showedWhole = m_viewSpan >= wholeSpan();
    m_clipId.clear();
    m_channels.clear();
    QString name;
    if (m_bus && !m_bus->selection().isEmpty()) {
        const QString wanted = m_bus->selection().last();
        const QJsonObject project = m_bus->timeline();
        for (const QJsonValue &track : project.value(QStringLiteral("tracks")).toArray())
            for (const QJsonValue &value : track.toObject().value(QStringLiteral("clips")).toArray()) {
                const QJsonObject clip = value.toObject();
                if (clip.value(QStringLiteral("id")).toString() != wanted) continue;
                m_clipId = wanted;
                m_clipStart = clip.value(QStringLiteral("timelineStart")).toDouble();
                m_duration = std::max<qint64>(1, qint64(clip.value(QStringLiteral("durationFrames")).toDouble()));
                m_channels = graph::channelsOf(clip);
                name = clip.value(QStringLiteral("name")).toString(wanted);
                // What the document would refuse, a drag does not ask for.
                const QString type = track.toObject().value(QStringLiteral("type")).toString();
                const QJsonArray effects = clip.value(QStringLiteral("effects")).toArray();
                for (Channel &channel : m_channels) {
                    switch (channel.kind) {
                    case Channel::Kind::Prop:
                        graph::holdToCatalog(channel, project.value(QStringLiteral("properties")).toObject().value(type).toArray());
                        graph::holdToCatalog(channel, project.value(QStringLiteral("generators")).toObject().value(clip.value(QStringLiteral("fx")).toString()).toArray());
                        break;
                    case Channel::Kind::Effect:
                        graph::holdToCatalog(channel, m_effectParams.value(effects.at(channel.index).toObject().value(QStringLiteral("fx")).toString()));
                        break;
                    case Channel::Kind::Mask:
                        graph::holdToCatalog(channel, project.value(QStringLiteral("maskProperties")).toArray());
                        break;
                    }
                }
            }
    }
    m_channel = -1;
    m_key = -1;
    m_selected.clear();
    if (m_clipId == previous)
        for (int index = 0; index < m_channels.size(); ++index) {
            const QString id = m_channels.at(index).id();
            const QList<Key> &keys = m_channels.at(index).keys;
            const QList<qint64> frames = selectedFrames.value(id);
            for (int key = 0; key < keys.size(); ++key)
                if (frames.contains(keys.at(key).frame)) m_selected[index].append(key);
            if (id != channelId) continue;
            m_channel = index;
            for (int key = 0; key < keys.size(); ++key)
                if (keys.at(key).frame == keyFrame && isSelected(index, key)) m_key = key;
        }
    if (m_channel < 0 && !m_channels.isEmpty()) m_channel = 0;
    // A view of the values lasts as long as the number does.
    if (m_clipId != previous) {
        m_valueViews.clear();
    } else {
        QHash<QString, std::pair<double, double>> kept;
        for (const Channel &channel : std::as_const(m_channels))
            if (const auto view = m_valueViews.constFind(channel.id()); view != m_valueViews.cend()) kept.insert(view.key(), view.value());
        m_valueViews = kept;
    }
    fitRanges();
    if (m_clipId != previous || showedWhole) setView(0.0, wholeSpan());
    else setView(m_viewStart, m_viewSpan);
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

void GraphEditor::commit(const QList<int> &channels) {
    QJsonArray ops;
    for (const int channel : channels)
        if (channel >= 0 && channel < m_channels.size())
            ops.append(graph::editOf(m_clipId, m_channels.at(channel), graph::keysJson(m_channels.at(channel).keys)));
    if (!ops.isEmpty()) emit editRequested(ops);
}

void GraphEditor::selectKey(int channel, int key) {
    m_channel = channel;
    m_key = key;
    m_selected.clear();
    if (key >= 0) m_selected[channel] = {key};
    updateBar();
    update();
}

bool GraphEditor::isSelected(int channel, int key) const {
    const auto found = m_selected.constFind(channel);
    return found != m_selected.cend() && found.value().contains(key);
}

int GraphEditor::selectedCount() const {
    int count = 0;
    for (const QList<int> &keys : m_selected) count += int(keys.size());
    return count;
}

void GraphEditor::takeOutOfSelection(int channel, int key) {
    const auto found = m_selected.find(channel);
    if (found == m_selected.end()) return;
    found.value().removeAll(key);
    if (found.value().isEmpty()) m_selected.erase(found);
    // Another selected key stands in as the one in hand.
    if (m_selected.contains(channel)) {
        m_key = m_selected.value(channel).last();
    } else if (!m_selected.isEmpty()) {
        m_channel = m_selected.firstKey();
        m_key = m_selected.first().last();
    } else {
        m_key = -1;
    }
    updateBar();
    update();
}

void GraphEditor::selectInBox() {
    const QRectF box = QRectF(m_pressed, m_boxTo).normalized();
    const QRectF area = plot();
    m_selected = m_boxBase;
    for (int channel = 0; channel < m_channels.size(); ++channel) {
        const QList<Key> &keys = m_channels.at(channel).keys;
        for (int key = 0; key < keys.size(); ++key) {
            const QPointF at = keyPoint(channel, key);
            if (!box.contains(at) || !area.contains(at) || isSelected(channel, key)) continue;
            m_selected[channel].append(key);
            std::sort(m_selected[channel].begin(), m_selected[channel].end());
        }
    }
    // The number in hand when it began stays so while the box catches any of its keys.
    if (m_selected.contains(m_boxChannel)) {
        m_channel = m_boxChannel;
        m_key = m_selected.value(m_channel).last();
    } else if (!m_selected.isEmpty()) {
        m_channel = m_selected.firstKey();
        m_key = m_selected.first().last();
    } else {
        m_channel = m_boxChannel;
        m_key = -1;
    }
    updateBar();
}

void GraphEditor::removeSelectedKeys() {
    if (m_selected.isEmpty()) return;
    QJsonArray ops;
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it) {
        Channel &channel = m_channels[it.key()];
        // All its keys gone: the value of the first of them stays, still.
        const double value = channel.keys.at(it.value().first()).value;
        for (auto key = it.value().crbegin(); key != it.value().crend(); ++key) channel.keys.removeAt(*key);
        ops.append(graph::editOf(m_clipId, channel, channel.keys.isEmpty() ? QJsonValue(value) : graph::keysJson(channel.keys)));
    }
    m_selected.clear();
    m_key = -1;
    updateBar();
    update();
    emit editRequested(ops);
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
    commit({index});
}

void GraphEditor::setInterp(const QString &interp) {
    if (m_selected.isEmpty()) return;
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it)
        for (const int key : it.value()) m_channels[it.key()].keys[key].interp = interp;
    fitRanges();
    update();
    commit(m_selected.keys());
}

void GraphEditor::setView(double start, double span) {
    const double whole = wholeSpan();
    m_viewSpan = std::clamp(span, std::min(kFewestFrames, whole), whole);
    m_viewStart = std::clamp(start, 0.0, whole - m_viewSpan);
    update();
}

void GraphEditor::showWholeClip() {
    m_valueViews.clear();
    setView(0.0, wholeSpan());
}

bool GraphEditor::valuesZoomed(int channel) const { return m_valueViews.contains(m_channels.at(channel).id()); }

void GraphEditor::setValueView(double low, double high) {
    if (m_channel < 0) return;
    const QString id = m_channels.at(m_channel).id();
    const auto [fitLow, fitHigh] = m_ranges.value(id, {0.0, 1.0});
    const double widest = std::max((fitHigh - fitLow) * kWidestValues, kFewestValues);
    const double span = std::clamp(high - low, std::min(kFewestValues, fitHigh - fitLow), widest);
    const double middle = (low + high) / 2.0;
    m_valueViews.insert(id, {middle - span / 2.0, middle + span / 2.0});
    update();
}

std::pair<double, double> GraphEditor::rangeOf(int channel) const {
    const QString id = m_channels.at(channel).id();
    const auto view = m_valueViews.constFind(id);
    return view != m_valueViews.cend() ? view.value() : m_ranges.value(id, {0.0, 1.0});
}

void GraphEditor::frameSelection() {
    if (m_selected.isEmpty()) return showWholeClip();
    // What is framed is on the plot: a number zoomed away from a selected key goes back to fitting.
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it) {
        const auto [low, high] = rangeOf(it.key());
        for (const int key : it.value()) {
            const double value = m_channels.at(it.key()).keys.at(key).value;
            if (value < low || value > high) m_valueViews.remove(m_channels.at(it.key()).id());
        }
    }
    qint64 first = std::numeric_limits<qint64>::max();
    qint64 last = std::numeric_limits<qint64>::min();
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it) {
        const QList<Key> &keys = m_channels.at(it.key()).keys;
        first = std::min(first, keys.at(it.value().first()).frame);
        last = std::max(last, keys.at(it.value().last()).frame);
    }
    double reach = double(last - first);
    if (selectedCount() == 1) {
        // One key in the middle, with the key nearest it for company so its curve can be read,
        // and never so far out that framing it does not bring it closer.
        const QList<Key> &keys = m_channels.at(m_channel).keys;
        double nearest = wholeSpan();
        if (m_key > 0) nearest = std::min(nearest, double(keys.at(m_key).frame - keys.at(m_key - 1).frame));
        if (m_key + 1 < keys.size()) nearest = std::min(nearest, double(keys.at(m_key + 1).frame - keys.at(m_key).frame));
        reach = std::min(2.0 * nearest, wholeSpan() / 2.0);
    }
    const double span = std::max(reach, kFewestFrames) * (1.0 + 2.0 * kFrameMargin);
    setView((double(first) + double(last) - span) / 2.0, span);
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
    connect(menu->addAction(selectedCount() > 1 ? QStringLiteral("Delete Keys") : QStringLiteral("Delete Key")), &QAction::triggered, this,
            &GraphEditor::removeSelectedKeys);
}

// ---------------------------------------------------------------- geometry

QRectF GraphEditor::listRect() const { return QRectF(0, kBar, kList, height() - kBar); }
QRectF GraphEditor::rulerRect() const { return QRectF(kList, kBar, width() - kList, kRuler); }

QRectF GraphEditor::plot() const {
    return QRectF(kList + kInset, kBar + kRuler + kInset, std::max(1, width() - kList - 2 * kInset), std::max(1, height() - kBar - kRuler - 2 * kInset));
}

double GraphEditor::xOf(double frame) const {
    const QRectF area = plot();
    return area.left() + (frame - m_viewStart) * area.width() / m_viewSpan;
}

double GraphEditor::frameOf(double x) const {
    const QRectF area = plot();
    return m_viewStart + (x - area.left()) * m_viewSpan / area.width();
}

double GraphEditor::yOf(int channel, double value) const {
    const QRectF area = plot();
    const auto [low, high] = rangeOf(channel);
    return area.bottom() - (value - low) / (high - low) * area.height();
}

double GraphEditor::valueOf(int channel, double y) const {
    const QRectF area = plot();
    const auto [low, high] = rangeOf(channel);
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
    const QRectF area = plot();
    const auto look = [&](int channel) {
        const QList<Key> &keys = m_channels.at(channel).keys;
        for (int key = 0; key < keys.size(); ++key) {
            const QPointF at(xOf(double(keys.at(key).frame)), yOf(channel, keys.at(key).value));
            // A key zoomed out of the plot is not there to take.
            if (!area.contains(at)) continue;
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
    if (m_channel < 0 || m_key < 0 || selectedCount() > 1) return std::nullopt;
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
    const double perFrame = area.width() / m_viewSpan;
    const qint64 step = tickStep(perFrame, 56.0);
    painter.setFont(tokens::font(tokens::monoSmall));
    const qint64 lastFrame = std::min(m_duration - 1, qint64(std::floor(m_viewStart + m_viewSpan)));
    for (qint64 frame = qint64(std::ceil(m_viewStart / double(step))) * step; frame <= lastFrame; frame += step) {
        const double x = xOf(double(frame));
        painter.setPen(QColor::fromRgba(tokens::line));
        painter.drawLine(QLineF(x, area.top(), x, area.bottom()));
        painter.setPen(QColor::fromRgba(tokens::ink3));
        painter.drawText(QPointF(x + 3, kBar + 14), QString::number(frame));
    }
    if (m_channel >= 0) {
        // The selected number's scale, in as many places as tell its lines apart.
        const auto [low, high] = rangeOf(m_channel);
        const double largest = std::max({std::abs(low), std::abs(high), 1e-9});
        const int places = std::max(4, int(std::ceil(std::log10(largest * 4.0 / (high - low)))) + 1);
        for (int line = 0; line <= 4; ++line) {
            const double y = area.top() + area.height() * line / 4.0;
            painter.setPen(QColor::fromRgba(tokens::line));
            painter.drawLine(QLineF(area.left(), y, area.right(), y));
            painter.setPen(QColor::fromRgba(tokens::ink3));
            painter.drawText(QPointF(area.left() + 3, y - 3), QString::number(valueOf(m_channel, y), 'g', places));
        }
    }

    // The curves, the selected one last; what is zoomed out of the plot stays out of it.
    painter.setRenderHint(QPainter::Antialiasing, true);
    painter.save();
    painter.setClipRect(area.adjusted(-kDiamond - 1.0, -kInset, kDiamond + 1.0, kInset));
    const auto drawCurve = [&](int index) {
        const QList<Key> &keys = m_channels.at(index).keys;
        const bool selected = index == m_channel;
        QPainterPath path;
        path.moveTo(area.left(), yOf(index, graph::valueAt(keys, frameOf(area.left()))));
        // Along the plot, with a corner at each key it passes.
        qsizetype next = 0;
        for (double x = area.left() + 2.0; x < area.right(); x += 2.0) {
            for (; next < keys.size() && xOf(double(keys.at(next).frame)) <= x; ++next)
                if (xOf(double(keys.at(next).frame)) >= area.left()) path.lineTo(keyPoint(index, int(next)));
            path.lineTo(x, yOf(index, graph::valueAt(keys, frameOf(x))));
        }
        path.lineTo(area.right(), yOf(index, graph::valueAt(keys, frameOf(area.right()))));
        painter.setPen(QPen(QColor::fromRgba(selected ? tokens::ink : tokens::track2), selected ? 1.5 : 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(path);
        for (int key = 0; key < keys.size(); ++key) {
            const QPointF at = keyPoint(index, key);
            if (at.x() < area.left() || at.x() > area.right()) continue;
            const bool on = isSelected(index, key);
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
    painter.restore();

    // The box being dragged out.
    if (m_drag == Drag::Box && m_changed) {
        QColor fill = QColor::fromRgba(tokens::accent);
        fill.setAlphaF(0.15);
        painter.setPen(QPen(QColor::fromRgba(tokens::accent), 1.0));
        painter.setBrush(fill);
        painter.drawRect(QRectF(m_pressed, m_boxTo).normalized());
    }

    // The playhead.
    if (m_bus) {
        const double frame = (m_bus->playhead() - m_clipStart) * m_bus->fps();
        m_playheadX = xOf(frame);
        if (frame >= 0.0 && frame <= double(m_duration) && m_playheadX >= area.left() && m_playheadX <= area.right()) {
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
    if (!message().isEmpty()) return;
    if (event->button() == Qt::MiddleButton) {
        m_drag = Drag::Pan;
        m_panFrom = m_viewStart;
        if (m_channel >= 0) m_panValuesFrom = rangeOf(m_channel);
        return;
    }
    if (event->button() != Qt::LeftButton) return;
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
    const bool shift = event->modifiers() & Qt::ShiftModifier;
    const auto [channel, key] = keyAt(point);
    if (channel >= 0) {
        m_shiftPress = shift;
        m_pressedSelected = isSelected(channel, key);
        if (!m_pressedSelected) {
            if (shift) {
                m_selected[channel].append(key);
                std::sort(m_selected[channel].begin(), m_selected[channel].end());
            } else {
                selectKey(channel, key);
            }
        }
        m_channel = channel;
        m_key = key;
        updateBar();
        update();
        m_grabbed = m_channels.at(channel).keys.at(key);
        m_before = m_channels;
        m_drag = Drag::Keys;
        m_held = keyPoint(channel, key) - point;
        return;
    }
    // A box can start in the margin, since a key can sit on the plot's edge.
    if (!plot().adjusted(-kInset, -kInset, kInset, kInset).contains(point)) return;
    m_drag = Drag::Box;
    m_boxTo = point;
    m_boxChannel = m_channel;
    m_boxBase = shift ? m_selected : QMap<int, QList<int>>{};
    if (!shift && !m_selected.isEmpty()) selectKey(m_channel, -1);
}

void GraphEditor::moveSelection(QPointF point, Qt::KeyboardModifiers modifiers, QPointF pointer) {
    // The key in hand leads, and the rest go as far as it does.
    qint64 frames = std::llround(frameOf(point.x())) - m_grabbed.frame;
    double down = point.y() - yOf(m_channel, m_grabbed.value);
    if (modifiers & Qt::ShiftModifier) {
        // Along whichever way it has gone further.
        const QPointF gone = pointer - m_pressed;
        if (std::abs(gone.x()) > std::abs(gone.y())) down = 0.0;
        else frames = 0;
    }
    // The tightest neighbour stops them all, so that they keep their spacing.
    qint64 back = std::numeric_limits<qint64>::min();
    qint64 forward = std::numeric_limits<qint64>::max();
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it) {
        const auto [low, high] = graph::roomToMove(m_before.at(it.key()).keys, it.value(), m_duration);
        back = std::max(back, low);
        forward = std::min(forward, high);
    }
    frames = std::clamp(frames, back, forward);
    for (auto it = m_selected.cbegin(); it != m_selected.cend(); ++it) {
        // Each number on its own scale: the same distance up the plot for all.
        const double rest = yOf(it.key(), 0.0);
        const double value = down == 0.0 ? 0.0 : valueOf(it.key(), rest + down);
        const Channel &before = m_before.at(it.key());
        m_channels[it.key()].keys = graph::shifted(before.keys, it.value(), frames, value, before.low, before.high);
    }
}

void GraphEditor::mouseMoveEvent(QMouseEvent *event) {
    const QPointF pointer = event->position();
    if (m_drag == Drag::None) return;
    if (m_drag == Drag::Ruler) {
        seekTo(pointer.x());
        return;
    }
    if (m_drag == Drag::Pan) {
        setView(m_panFrom - (pointer.x() - m_pressed.x()) * m_viewSpan / plot().width(), m_viewSpan);
        // Up and down only once the number has been zoomed: a hand that wobbles
        // going along should not take it off fitting the plot.
        if (m_channel >= 0 && valuesZoomed(m_channel)) {
            const auto [low, high] = m_panValuesFrom;
            const double by = (pointer.y() - m_pressed.y()) * (high - low) / plot().height();
            setValueView(low + by, high + by);
        }
        return;
    }
    if (!m_changed && QLineF(pointer, m_pressed).length() < kDragStart) return;
    const QPointF point = pointer + m_held;
    switch (m_drag) {
    case Drag::None:
    case Drag::Ruler:
    case Drag::Pan: return;
    case Drag::Box:
        m_boxTo = pointer;
        m_changed = true;
        selectInBox();
        update();
        return;
    case Drag::Keys:
        moveSelection(point, event->modifiers(), pointer);
        m_changed = true;
        update();
        return;
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
    const Drag drag = m_drag;
    const bool changed = m_changed;
    m_drag = Drag::None;
    m_changed = false;
    m_before.clear();
    if (drag == Drag::Box) update();
    if (drag == Drag::Keys && !changed && m_pressedSelected) {
        // It was a click on what was already selected.
        if (m_shiftPress) takeOutOfSelection(m_channel, m_key);
        else if (selectedCount() > 1) selectKey(m_channel, m_key);
    }
    if (changed && (drag == Drag::Keys || drag == Drag::Handle1 || drag == Drag::Handle2)) {
        // The plot keeps still during a drag; the new reach is fitted after it.
        fitRanges();
        update();
        commit(drag == Drag::Keys ? m_selected.keys() : QList<int>{m_channel});
    }
}

void GraphEditor::wheelEvent(QWheelEvent *event) {
    if (!message().isEmpty()) {
        event->ignore();
        return;
    }
    event->accept();
    if ((event->modifiers() & Qt::AltModifier) && m_channel >= 0) return wheelValues(event);
    const QPoint angle = event->angleDelta();
    const QPoint pixels = event->pixelDelta();
    const bool shift = event->modifiers() & Qt::ShiftModifier;
    const QRectF area = plot();
    // Sideways pans, and so does up and down with Shift where the platform has not turned it.
    if (shift || std::abs(angle.x()) > std::abs(angle.y())) {
        const int notches = angle.x() != 0 ? angle.x() : angle.y();
        const int moved = pixels.x() != 0 ? pixels.x() : pixels.y();
        const double points = moved != 0 ? double(moved) : double(notches) * kPanPerNotch / 120.0;
        setView(m_viewStart - points * m_viewSpan / area.width(), m_viewSpan);
        return;
    }
    // The fingers lifted: zoom stops where they left it.
    if (event->phase() == Qt::ScrollMomentum || angle.y() == 0) return;
    const double x = std::clamp(event->position().x(), area.left(), area.right());
    const double frame = frameOf(x);
    const double span = std::clamp(m_viewSpan * std::pow(kZoomPerNotch, -double(angle.y()) / 120.0), std::min(kFewestFrames, wholeSpan()), wholeSpan());
    setView(frame - (x - area.left()) * span / area.width(), span);
}

void GraphEditor::wheelValues(QWheelEvent *event) {
    // Some platforms hand a wheel turned with Alt over as going sideways, and with
    // Shift as well on this one, so either way is the one way here.
    const QPoint angle = event->angleDelta();
    const QPoint pixels = event->pixelDelta();
    const int notches = angle.y() != 0 ? angle.y() : angle.x();
    const QRectF area = plot();
    const auto [low, high] = rangeOf(m_channel);
    if (event->modifiers() & Qt::ShiftModifier) {
        const int moved = pixels.y() != 0 ? pixels.y() : pixels.x();
        const double points = moved != 0 ? double(moved) : double(notches) * kPanPerNotch / 120.0;
        const double by = points * (high - low) / area.height();
        setValueView(low + by, high + by);
        return;
    }
    // The fingers lifted: zoom stops where they left it.
    if (event->phase() == Qt::ScrollMomentum || notches == 0) return;
    const double y = std::clamp(event->position().y(), area.top(), area.bottom());
    const double value = valueOf(m_channel, y);
    const auto [fitLow, fitHigh] = m_ranges.value(m_channels.at(m_channel).id(), {0.0, 1.0});
    const double span = std::clamp((high - low) * std::pow(kZoomPerNotch, -double(notches) / 120.0), std::min(kFewestValues, fitHigh - fitLow),
                                   std::max((fitHigh - fitLow) * kWidestValues, kFewestValues));
    const double below = (area.bottom() - y) / area.height() * span;
    setValueView(value - below, value - below + span);
}

void GraphEditor::mouseDoubleClickEvent(QMouseEvent *event) {
    if (event->button() != Qt::LeftButton || !message().isEmpty() || !plot().contains(event->position())) return;
    if (keyAt(event->position()).first >= 0) return;
    addKey(m_channel, std::llround(frameOf(event->position().x())));
}

void GraphEditor::contextMenuEvent(QContextMenuEvent *event) {
    const auto [channel, key] = keyAt(QPointF(event->pos()));
    if (channel < 0) return;
    if (isSelected(channel, key)) {
        m_channel = channel;
        m_key = key;
    } else {
        selectKey(channel, key);
    }
    auto *menu = new QMenu(this);
    menu->setAttribute(Qt::WA_DeleteOnClose);
    populateCurveMenu(menu);
    menu->popup(event->globalPos());
}

void GraphEditor::keyPressEvent(QKeyEvent *event) {
    if ((event->key() == Qt::Key_Delete || event->key() == Qt::Key_Backspace) && !m_selected.isEmpty()) {
        removeSelectedKeys();
        return;
    }
    if (isFKey(event) && message().isEmpty() && (event->modifiers() & ~Qt::ShiftModifier) == Qt::NoModifier) {
        if (event->modifiers() & Qt::ShiftModifier) showWholeClip();
        else frameSelection();
        return;
    }
    QWidget::keyPressEvent(event);
}

} // namespace kerf
