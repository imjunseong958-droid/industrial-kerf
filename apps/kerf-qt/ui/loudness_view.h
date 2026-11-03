#pragma once

// The Loudness pane of the Finish and Deliver trays (mockup-spec 12): the
// momentary and short-term loudness while the program plays (the bridge's
// `meter`), and the programme's integrated loudness, range and true peak
// (its `loudness` event, plan section 3). Nothing shows a number the engine
// has not measured: an unmeasured value is a dash.
//
// Shell from Q-SKELETON (W1); Q-FINISH draws the history and the true-peak
// bars in W2, keeping the constructor and this API.

#include <QHash>
#include <QJsonObject>
#include <QWidget>

class QLabel;

namespace kerf {

class EditorBus;

class LoudnessView final : public QWidget {
    Q_OBJECT
public:
    explicit LoudnessView(QWidget *parent, EditorBus *bus);
    // A `loudness` event: {status, integrated, lra, truePeak:[l,r], …}.
    void setLoudness(const QJsonObject &loudness);
    QJsonObject loudness() const { return m_loudness; }
    // The programme has been measured (status ready).
    bool measured() const;
    // The text of a statistic: "M", "S", "I", "LRA", "TP L", "TP R".
    QString statText(const QString &key) const;
    QString statusText() const;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void setMeter(const QJsonObject &meter);
    EditorBus *m_bus = nullptr;
    QJsonObject m_loudness;
    QHash<QString, QLabel *> m_values;
    QLabel *m_status = nullptr;
};

} // namespace kerf
