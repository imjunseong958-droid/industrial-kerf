#pragma once

// The Inspector's Deliver pane (mockup-spec 5.2), shown in the Deliver
// workspace while Video is chosen: the master the Deliver tray renders, as
// groups of key-value rows.
//
// setOutput takes {title, file, groups:[{title, rows:[[label, value]]}]},
// built by Q-DELIVER from the render options and `render_queue` in W2. With
// none set it says so; it never shows a made-up master.
//
// Shell from Q-SKELETON (W1), keeping the constructor and this API.

#include <QJsonObject>
#include <QWidget>

class QLabel;
class QVBoxLayout;

namespace kerf {

class EditorBus;

class DeliverPane final : public QWidget {
    Q_OBJECT
public:
    explicit DeliverPane(QWidget *parent, EditorBus *bus);
    void setOutput(const QJsonObject &output);
    QJsonObject output() const { return m_output; }
    // The header's two lines: "Master", "atlas_30s_v7_master.mov".
    QString titleText() const;
    QString fileText() const;
    // "Codec" → "ProRes 422 HQ", from the rows shown.
    QString value(const QString &label) const;

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    EditorBus *m_bus = nullptr;
    QJsonObject m_output;
    QLabel *m_title = nullptr;
    QLabel *m_file = nullptr;
    QWidget *m_body = nullptr;
    QVBoxLayout *m_bodyLayout = nullptr;
};

} // namespace kerf
