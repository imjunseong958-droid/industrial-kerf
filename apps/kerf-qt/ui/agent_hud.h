#pragma once

// The agent HUD over the picture (mockup-spec 3.3): Kerf's preview render at
// work, from the bridge's `preview {status, progress, message, proposalId}`
// events. It shows only while a preview renders (status running) and hides
// on any other status, or at once after Stop.
//
//   16:9, a card (288 x 72 at most, inside title safe; see cardRect):
//     row 1  the Kerf tile and "Rendering preview · v2" (the version of the
//            proposal being previewed, from the snapshot's proposals)
//     row 2  the proposal's first change and its count: "Logo reveal →
//            00:00:26:12 · 2 changes" (the review model's proposal diff),
//            else its summary, else the event's message
//     row 3  the progress bar, "62%" and Stop (preview_stop; Esc)

#include <QJsonObject>
#include <QPointer>
#include <QRectF>
#include <QSizeF>
#include <QWidget>

class QAbstractButton;
class QLabel;

namespace kerf {

class EditorBus;

namespace ui {
class Button;
class ThinProgress;
}

class AgentHud final : public QWidget {
    Q_OBJECT
public:
    static constexpr int kCardWidth = 288;
    static constexpr int kCardHeight = 72;

    explicit AgentHud(QWidget *parent, EditorBus *bus);
    ~AgentHud() override;

    // A preview renders and was not stopped here: the HUD shows.
    bool active() const { return m_running && !m_stopped; }
    // Row 1 ("Rendering preview · v2"), or empty while hidden.
    QString text() const;
    QString title() const;
    // Row 2 as plain text.
    QString detail() const;
    double progress() const { return m_progress; }
    // "62%".
    QString percentText() const;
    QString proposalId() const { return m_proposal; }

    QSize sizeHint() const override;
    // Where the card sits on a picture of `picture` size, in the picture's
    // coordinates: w = min(288, w - 2tx - 16), 72 tall, at
    // (w - tx - 8 - hw, ty + 8), with tx and ty the 5% title-safe insets.
    static QRectF cardRect(const QSizeF &picture);

    QAbstractButton *stopButton() const;
    // Stops the render (preview_stop) and hides the HUD until the next one.
    void stop();

signals:
    void activeChanged(bool active);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    struct Run {
        QString text;
        bool mono = false;
    };
    void handle(const QString &type, const QJsonObject &event);
    void refresh();
    void relayout();
    void updateEscape();
    int versionOf(const QString &proposalId) const;
    QList<Run> detailRuns() const;

    QPointer<EditorBus> m_bus;
    ui::Button *m_stop = nullptr;
    ui::ThinProgress *m_bar = nullptr;
    bool m_running = false;
    bool m_stopped = false;
    bool m_wasActive = false;
    bool m_filtering = false;
    double m_progress = 0.0;
    QString m_proposal;
    QString m_message;
};

} // namespace kerf
