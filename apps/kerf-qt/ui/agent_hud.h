#pragma once

// The agent HUD over the picture (mockup-spec 3.3): who is at work and their
// latest step, while an agent works. Shell from Q-SKELETON (W1): it follows
// the bridge's `agent_busy`, `agent_state` and `agent` (tool and status)
// events; Q-SCRUBRAIL gives it its look and place in W2, keeping the
// constructor and this API.

#include <QJsonObject>
#include <QWidget>

class QLabel;

namespace kerf {

class EditorBus;

class AgentHud final : public QWidget {
    Q_OBJECT
public:
    explicit AgentHud(QWidget *parent, EditorBus *bus);
    // An agent is at work: the HUD has something to say.
    bool active() const { return m_busy; }
    // "Kerf · Trimming B003_C011", or empty while idle.
    QString text() const;

signals:
    void activeChanged(bool active);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void handle(const QString &type, const QJsonObject &event);
    EditorBus *m_bus = nullptr;
    QLabel *m_text = nullptr;
    QString m_step;
    bool m_busy = false;
};

} // namespace kerf
