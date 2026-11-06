#pragma once

// The graph editor: a tray over the timeline (Ctrl+Alt+G, View > Graph
// Editor) with the keyframed numbers of the selected clip, each curve fitted
// to the plot's height, so that a rotation of hundreds and an opacity of one
// read side by side; the selected one has the value labels.
//
// Drag a key to move it, a whole frame at a time and between its neighbours;
// with Shift, along one axis. Drag across the plot to select the keys in a box,
// or Shift-click to add one or take it away; the selected keys move together,
// keeping their spacing. Drag a handle to shape the curve from a key to the
// next. Double-click to add a key on the curve, Delete to remove what is
// selected, and right-click or Curve for how it moves to the next key. A change
// goes to the document when the drag ends, as one edit. The ruler seeks.
//
// Scrolling up and down zooms the time axis around the pointer; sideways or
// with the middle button pans it. F or Frame shows what is selected, and Shift+F
// or Fit shows the whole clip.
//
// Nothing here animates: the tray comes and goes as the others do, and what
// changes is drawn where it changed. See MOTION_UI.md.

#include "graph_model.h"

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QMap>
#include <QPoint>
#include <QPointF>
#include <QRectF>
#include <QWidget>

class QLabel;
class QMenu;

namespace kerf {

class EditorBus;

namespace ui {
class Button;
}

class GraphEditor final : public QWidget {
    Q_OBJECT

public:
    explicit GraphEditor(QWidget *parent, EditorBus *bus);

    // The effects' definitions (kerf_fx_catalog_all), for what each number allows.
    void setEffectCatalog(const QJsonArray &catalog);

    // What a test reads and points at, in this widget's coordinates.
    QStringList channelLabels() const;
    int selectedChannel() const { return m_channel; }
    int selectedKey() const { return m_key; }
    // Every selected key of a channel, in order.
    QList<int> selectedKeys(int channel) const { return m_selected.value(channel); }
    // The frames at the plot's left and right edges.
    std::pair<double, double> viewFrames() const { return {m_viewStart, m_viewStart + m_viewSpan}; }
    // Show what is selected: its keys and a little either side, one key and the
    // keys next to it, or with nothing selected the whole clip.
    void frameSelection();
    void showWholeClip();
    QPointF keyPoint(int channel, int key) const;
    QPointF channelPoint(int channel) const;
    // The selected key's handles, when it has them.
    std::optional<std::pair<QPointF, QPointF>> handlePoints() const;
    QPointF rulerPoint(double frame) const;
    QPointF plotPoint(int channel, double frame, double value) const;
    QString message() const;
    void setInterp(const QString &interp);

signals:
    void editRequested(const QJsonArray &ops);
    void seekRequested(double seconds);

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void mouseMoveEvent(QMouseEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    void mouseDoubleClickEvent(QMouseEvent *event) override;
    void contextMenuEvent(QContextMenuEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void wheelEvent(QWheelEvent *event) override;

private:
    enum class Drag { None, Keys, Handle1, Handle2, Ruler, Box, Pan };

    void rebuild();
    void fitRanges();
    void selectKey(int channel, int key);
    // The keys in the box, added to what was selected when it began.
    void selectInBox();
    void takeOutOfSelection(int channel, int key);
    bool isSelected(int channel, int key) const;
    int selectedCount() const;
    void moveSelection(QPointF point, Qt::KeyboardModifiers modifiers, QPointF pointer);
    void commit(const QList<int> &channels);
    void removeSelectedKeys();
    void addKey(int channel, qint64 frame);
    void movePlayhead();
    void populateCurveMenu(QMenu *menu);
    void updateBar();
    void seekTo(double x);
    // Show `span` frames from `start`, held to the clip.
    void setView(double start, double span);
    double wholeSpan() const { return double(std::max<qint64>(1, m_duration - 1)); }

    QRectF plot() const;
    QRectF listRect() const;
    QRectF rulerRect() const;
    double xOf(double frame) const;
    double frameOf(double x) const;
    double yOf(int channel, double value) const;
    double valueOf(int channel, double y) const;
    int channelAt(QPointF point) const;
    std::pair<int, int> keyAt(QPointF point) const;

    EditorBus *m_bus = nullptr;
    QLabel *m_meta = nullptr;
    ui::Button *m_curve = nullptr;
    QString m_clipId;
    double m_clipStart = 0.0;
    qint64 m_duration = 1;
    QList<graph::Channel> m_channels;
    QHash<QString, std::pair<double, double>> m_ranges;
    QHash<QString, QJsonArray> m_effectParams;
    // The time axis: which frame is at the plot's left, and how many it shows.
    double m_viewStart = 0.0;
    double m_viewSpan = 1.0;
    int m_channel = -1;
    int m_key = -1;
    // Selected keys by channel, each list in order. The key in m_channel and
    // m_key, the last one taken, is among them and is the one with handles.
    QMap<int, QList<int>> m_selected;
    Drag m_drag = Drag::None;
    QPointF m_pressed;
    // From the pointer to what it took hold of, which keeps from jumping to it.
    QPointF m_held;
    graph::Key m_grabbed;
    // Where the keys were when the drag began, and the view when a pan began.
    QList<graph::Channel> m_before;
    double m_panFrom = 0.0;
    // What a press on a selected key does if it turns out not to be a drag:
    // Shift takes the key away, otherwise the selection becomes just that key.
    bool m_shiftPress = false;
    bool m_pressedSelected = false;
    // What was selected when the box began, the number in hand then, and the
    // box's far corner.
    QMap<int, QList<int>> m_boxBase;
    int m_boxChannel = -1;
    QPointF m_boxTo;
    bool m_changed = false;
    double m_playheadX = -1.0;
};

} // namespace kerf
