#pragma once

// The graph editor: a tray over the timeline (Ctrl+Alt+G, View > Graph
// Editor) with the keyframed numbers of the selected clip, each curve fitted
// to the plot's height, so that a rotation of hundreds and an opacity of one
// read side by side; the selected one has the value labels.
//
// Drag a key to move it, a whole frame at a time and between its neighbours;
// with Shift, along one axis. Drag a handle to shape the curve from a key to
// the next. Double-click to add a key on the curve, Delete to remove one, and
// right-click or Curve for how it moves to the next key. A change goes to the
// document when the drag ends, as one edit. The ruler seeks.
//
// Nothing here animates: the tray comes and goes as the others do, and what
// changes is drawn where it changed. See MOTION_UI.md.

#include "graph_model.h"

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
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

    // What a test reads and points at, in this widget's coordinates.
    QStringList channelLabels() const;
    int selectedChannel() const { return m_channel; }
    int selectedKey() const { return m_key; }
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

private:
    enum class Drag { None, Key, Handle1, Handle2, Ruler };

    void rebuild();
    void fitRanges();
    void selectKey(int channel, int key);
    void commit(int channel);
    void removeSelectedKey();
    void addKey(int channel, qint64 frame);
    void movePlayhead();
    void populateCurveMenu(QMenu *menu);
    void updateBar();
    void seekTo(double x);

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
    int m_channel = -1;
    int m_key = -1;
    Drag m_drag = Drag::None;
    QPointF m_pressed;
    // From the pointer to what it took hold of, which keeps from jumping to it.
    QPointF m_held;
    graph::Key m_grabbed;
    bool m_changed = false;
    double m_playheadX = -1.0;
};

} // namespace kerf
