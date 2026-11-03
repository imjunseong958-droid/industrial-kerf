#pragma once

// The scrub strip under the picture (mockup-spec 3.4): 56 points, the comment
// lane over the bar and the transport. Shell from Q-SKELETON (W1): it shows
// the playhead and the length from the bus; Q-SCRUBRAIL builds the lane, the
// bar and the transport in W2, keeping the constructor and this API.

#include <QWidget>

class QLabel;

namespace kerf {

class EditorBus;

class ScrubStrip final : public QWidget {
    Q_OBJECT
public:
    explicit ScrubStrip(QWidget *parent, EditorBus *bus);
    // "00:00:17:08 / 00:00:30:00", from the bus.
    QString timeText() const;
    QSize sizeHint() const override;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void updateTime();
    EditorBus *m_bus = nullptr;
    QLabel *m_time = nullptr;
};

} // namespace kerf
