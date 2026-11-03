#pragma once

#include "test_friends.h"

#include <QImage>
#include <QJsonObject>
#include <QProcess>
#include <QWidget>

class QAction;
class QLabel;

namespace kerf {

// The source monitor: one piece of media from the bin, scrubbed frame by
// frame, with in and out marks for insert and overwrite edits. Frames decode
// in the background; the latest request wins.
class SourceMonitorPanel final : public QWidget {
    Q_OBJECT
    KERF_TEST_FRIENDS
public:
    explicit SourceMonitorPanel(QWidget *parent = nullptr);
    ~SourceMonitorPanel() override;

    // `source` is a snapshot media entry; `path` is resolved against the project.
    void open(const QJsonObject &source, const QString &path);
    void close();
    bool isOpen() const { return !m_mediaId.isEmpty(); }
    QString mediaId() const { return m_mediaId; }
    QJsonObject source() const { return m_source; }
    double fps() const { return m_fps; }
    double position() const { return m_position; }
    double inPoint() const { return m_in; }
    double outPoint() const { return m_out; }
    // The decoded picture on screen; null until the first frame arrives.
    QImage frame() const;

    void seek(double seconds);
    void step(int frames);
    void markIn();
    void markOut();
    void clearMarks();
    // Buttons under the picture, run as commands.
    void setActions(const QList<QAction *> &actions);

signals:
    void changed();

private:
    class FrameView;
    class ScrubBar;
    void decode(double seconds);
    void decoded(int exitCode, QProcess::ExitStatus status);
    void updateLabels();

    QJsonObject m_source;
    QString m_mediaId;
    QString m_path;
    double m_duration = 0.0;
    double m_fps = 24.0;
    double m_position = 0.0;
    double m_in = 0.0;
    double m_out = 0.0;
    bool m_hasVideo = false;
    QProcess m_decoder;
    bool m_hasPending = false;
    double m_pending = 0.0;
    FrameView *m_view = nullptr;
    ScrubBar *m_scrub = nullptr;
    QLabel *m_time = nullptr;
    QLabel *m_range = nullptr;
    QWidget *m_buttons = nullptr;
};

} // namespace kerf
